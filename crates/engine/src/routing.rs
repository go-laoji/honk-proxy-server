//! Immutable first-match policy. Production and dry-run share this evaluator.
use crate::ProxyError;
use hps_config::{ProxyConfig, RuleConfig};
use hps_domain::{FlowOrigin, RouteAction, RouteDecision, RoutingContext, RuleEvaluation};
use ipnet::IpNet;
use std::{
    net::IpAddr,
    time::{SystemTime, UNIX_EPOCH},
};

#[derive(Clone, Copy, Default)]
pub struct RoutingCapabilities {
    pub local_process: bool,
    pub udp: bool,
}
struct CompiledRule {
    rule: RuleConfig,
    domains: Vec<String>,
    suffixes: Vec<String>,
    destinations: Vec<IpNet>,
    sources: Vec<IpNet>,
}
pub struct Router {
    rules: Vec<CompiledRule>,
    default: RouteAction,
    observe: bool,
}
impl Router {
    pub fn potential_actions(
        &self,
        inbound: &str,
        protocol: &str,
        transport: hps_domain::SelectionTransport,
    ) -> Vec<&RouteAction> {
        let mut actions = vec![];
        for rule in &self.rules {
            let c = &rule.rule.conditions;
            if !rule.rule.enabled
                || (!c.inbounds.is_empty() && !c.inbounds.iter().any(|id| id == inbound))
                || (!c.inbound_protocols.is_empty()
                    && !c.inbound_protocols.iter().any(|p| p == protocol))
                || (!c.transports.is_empty() && !c.transports.contains(&transport))
            {
                continue;
            }
            actions.push(&rule.rule.action);
            if c.domains.is_empty()
                && c.domain_suffixes.is_empty()
                && c.destination_cidrs.is_empty()
                && c.destination_ports.is_empty()
                && c.source_cidrs.is_empty()
                && c.process.is_none()
            {
                return actions;
            }
        }
        actions.push(&self.default);
        actions
    }
    pub fn compile(
        config: &ProxyConfig,
        capabilities: RoutingCapabilities,
    ) -> Result<Self, ProxyError> {
        config
            .routing
            .validate(config)
            .map_err(|_| ProxyError::RuleConfiguration)?;
        let mut rules = Vec::with_capacity(config.routing.rules.len());
        for rule in &config.routing.rules {
            if rule.enabled
                && ((rule.conditions.process.is_some() && !capabilities.local_process)
                    || (!capabilities.udp
                        && rule.conditions.transports == [hps_domain::SelectionTransport::Udp]
                        && rule.action != RouteAction::Reject))
            {
                return Err(ProxyError::UnsupportedRule);
            }
            let c = &rule.conditions;
            rules.push(CompiledRule {
                rule: rule.clone(),
                domains: c.domains.iter().map(|d| canonical(d)).collect(),
                suffixes: c.domain_suffixes.iter().map(|d| canonical(d)).collect(),
                destinations: c
                    .destination_cidrs
                    .iter()
                    .map(|s| s.parse().map_err(|_| ProxyError::RuleConfiguration))
                    .collect::<Result<_, _>>()?,
                sources: c
                    .source_cidrs
                    .iter()
                    .map(|s| s.parse().map_err(|_| ProxyError::RuleConfiguration))
                    .collect::<Result<_, _>>()?,
            });
        }
        Ok(Self {
            rules,
            default: config.routing.default_action.clone(),
            observe: config.routing.use_dns_observations,
        })
    }
    pub fn evaluate(
        &self,
        context: &RoutingContext,
        revision: i64,
        explain: bool,
    ) -> RouteDecision {
        let ip = context.destination_host.parse::<IpAddr>().ok();
        let (domain, source) = if ip.is_none() {
            (Some(canonical(&context.destination_host)), "explicit")
        } else if self.observe && context.observed_domains.len() == 1 {
            (
                Some(canonical(&context.observed_domains[0])),
                "dns_observation",
            )
        } else if self.observe && context.observed_domains.len() > 1 {
            (None, "ambiguous_dns_observations")
        } else {
            (None, "unknown")
        };
        let mut trace = Vec::new();
        let mut action = self.default.clone();
        let mut matched = None;
        for compiled in &self.rules {
            let c = &compiled.rule.conditions;
            let mut reasons = Vec::<String>::new();
            if !compiled.rule.enabled {
                reasons.push("disabled".into());
            } else {
                if (!compiled.domains.is_empty() || !compiled.suffixes.is_empty())
                    && domain.as_ref().is_none_or(|d| {
                        !compiled.domains.contains(d)
                            && !compiled.suffixes.iter().any(|suffix| {
                                d == suffix
                                    || d.strip_suffix(suffix)
                                        .is_some_and(|prefix| prefix.ends_with('.'))
                            })
                    })
                {
                    reasons.push(
                        if domain.is_none() {
                            "domain_unknown"
                        } else {
                            "domain_mismatch"
                        }
                        .into(),
                    );
                }
                if !compiled.destinations.is_empty()
                    && ip.is_none_or(|ip| !contains(&compiled.destinations, ip))
                {
                    reasons.push(
                        if ip.is_none() {
                            "destination_ip_unknown"
                        } else {
                            "destination_cidr_mismatch"
                        }
                        .into(),
                    );
                }
                if !c.destination_ports.is_empty()
                    && !c
                        .destination_ports
                        .iter()
                        .any(|p| (p.start..=p.end).contains(&context.destination_port))
                {
                    reasons.push("destination_port_mismatch".into());
                }
                if !c.transports.is_empty() && !c.transports.contains(&context.transport) {
                    reasons.push("transport_mismatch".into());
                }
                if !c.inbound_protocols.is_empty()
                    && context
                        .inbound_protocol
                        .as_ref()
                        .is_none_or(|p| !c.inbound_protocols.contains(p))
                {
                    reasons.push("inbound_protocol_unknown_or_mismatch".into());
                }
                if !c.inbounds.is_empty()
                    && context
                        .inbound
                        .as_ref()
                        .is_none_or(|i| !c.inbounds.contains(i))
                {
                    reasons.push("inbound_unknown_or_mismatch".into());
                }
                if !compiled.sources.is_empty()
                    && context
                        .source_ip
                        .is_none_or(|ip| !contains(&compiled.sources, ip))
                {
                    reasons.push(
                        if context.source_ip.is_none() {
                            "source_ip_unknown"
                        } else {
                            "source_cidr_mismatch"
                        }
                        .into(),
                    );
                }
                if let Some(condition) = &c.process {
                    if context.origin != FlowOrigin::LinuxLocal || context.process.is_none() {
                        reasons.push("verified_local_process_unavailable".into());
                    } else if let Some(process) = &context.process
                        && (condition.pid.is_some_and(|pid| pid != process.pid)
                            || condition
                                .command
                                .as_ref()
                                .is_some_and(|comm| comm != &process.command)
                            || condition
                                .cgroup_id
                                .is_some_and(|id| id != process.cgroup_id))
                    {
                        reasons.push("process_mismatch".into());
                    }
                }
            }
            let hit = reasons.is_empty();
            if explain {
                trace.push(RuleEvaluation {
                    id: compiled.rule.id.clone(),
                    matched: hit,
                    reasons,
                });
            }
            if hit {
                matched = Some(compiled.rule.id.clone());
                action = compiled.rule.action.clone();
                break;
            }
        }
        RouteDecision {
            configuration_revision: revision,
            evaluated_at_unix: unix_now(),
            rule_id: matched,
            action,
            domain,
            domain_source: source.into(),
            trace,
        }
    }
}
fn canonical(name: &str) -> String {
    name.trim_end_matches('.').to_ascii_lowercase()
}
fn contains(networks: &[IpNet], ip: IpAddr) -> bool {
    networks.iter().any(|network| {
        network.contains(&ip)
            || match ip {
                IpAddr::V6(ip) => ip
                    .to_ipv4_mapped()
                    .is_some_and(|v4| network.contains(&IpAddr::V4(v4))),
                _ => false,
            }
    })
}
pub(crate) fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;
    use hps_config::{RoutingConfig, RuleConditions};
    use hps_domain::{ProcessIdentity, SelectionTransport};
    fn context(host: &str) -> RoutingContext {
        RoutingContext {
            destination_host: host.into(),
            destination_port: 443,
            transport: SelectionTransport::Tcp,
            source_ip: Some("::ffff:192.0.2.8".parse().unwrap()),
            source_port: None,
            inbound: Some("browser".into()),
            inbound_protocol: Some("http".into()),
            observed_domains: vec![],
            origin: FlowOrigin::Explicit,
            process: None,
        }
    }
    fn config(rules: Vec<RuleConfig>) -> ProxyConfig {
        ProxyConfig {
            routing: RoutingConfig {
                rules,
                default_action: RouteAction::Direct,
                use_dns_observations: false,
            },
            ..Default::default()
        }
    }
    fn rule(id: &str, conditions: RuleConditions, action: RouteAction) -> RuleConfig {
        RuleConfig {
            id: id.into(),
            enabled: true,
            conditions,
            action,
        }
    }
    #[test]
    fn first_match_domain_boundary_combinations_unknowns_and_order_are_shared() {
        let c = config(vec![
            rule(
                "domain",
                RuleConditions {
                    domain_suffixes: vec!["Example.COM.".into()],
                    source_cidrs: vec!["192.0.2.0/24".into()],
                    destination_ports: vec![hps_config::PortRange {
                        start: 443,
                        end: 443,
                    }],
                    inbound_protocols: vec!["http".into()],
                    ..Default::default()
                },
                RouteAction::Reject,
            ),
            rule("last", RuleConditions::default(), RouteAction::Default),
        ]);
        let router = Router::compile(&c, RoutingCapabilities::default()).unwrap();
        let decision = router.evaluate(&context("sub.example.com"), 42, true);
        assert_eq!(decision.rule_id.as_deref(), Some("domain"));
        assert_eq!(decision.configuration_revision, 42);
        assert_eq!(decision.action, RouteAction::Reject);
        assert_eq!(decision.trace.len(), 1);
        let d = router.evaluate(&context("notexample.com"), 42, true);
        assert_eq!(d.rule_id.as_deref(), Some("last"));
        assert!(d.trace[0].reasons.contains(&"domain_mismatch".into()));
        let mut unknown = context("203.0.113.9");
        unknown.source_ip = None;
        let d = router.evaluate(&unknown, 42, true);
        assert!(d.trace[0].reasons.contains(&"domain_unknown".into()));
        assert!(d.trace[0].reasons.contains(&"source_ip_unknown".into()));
        let mut c = c;
        c.routing.rules.reverse();
        let router = Router::compile(&c, RoutingCapabilities::default()).unwrap();
        assert_eq!(
            router
                .evaluate(&context("sub.example.com"), 43, true)
                .rule_id
                .as_deref(),
            Some("last")
        );
    }
    #[test]
    fn dns_inference_is_opt_in_unambiguous_and_never_resolves_for_cidr_matching() {
        let mut c = config(vec![
            rule(
                "domain",
                RuleConditions {
                    domains: vec!["site.example".into()],
                    ..Default::default()
                },
                RouteAction::Reject,
            ),
            rule(
                "ipv6",
                RuleConditions {
                    destination_cidrs: vec!["2001:db8::/32".into()],
                    ..Default::default()
                },
                RouteAction::Reject,
            ),
        ]);
        let mut ctx = context("203.0.113.9");
        ctx.observed_domains = vec!["site.example".into()];
        assert_eq!(
            Router::compile(&c, RoutingCapabilities::default())
                .unwrap()
                .evaluate(&ctx, 1, true)
                .action,
            RouteAction::Direct
        );
        c.routing.use_dns_observations = true;
        let r = Router::compile(&c, RoutingCapabilities::default()).unwrap();
        assert_eq!(r.evaluate(&ctx, 1, true).rule_id.as_deref(), Some("domain"));
        ctx.observed_domains.push("other.example".into());
        assert_eq!(
            r.evaluate(&ctx, 1, true).domain_source,
            "ambiguous_dns_observations"
        );
        assert_eq!(r.evaluate(&ctx, 1, true).action, RouteAction::Direct);
        assert_eq!(
            r.evaluate(&context("2001:db8::1"), 1, true)
                .rule_id
                .as_deref(),
            Some("ipv6")
        );
        assert!(
            r.evaluate(&context("unknown.example"), 1, true).trace[1]
                .reasons
                .contains(&"destination_ip_unknown".into())
        );
    }
    #[test]
    fn unsupported_process_or_udp_cannot_publish_and_remote_process_is_never_invented() {
        let mut c = config(vec![rule(
            "process",
            RuleConditions {
                process: Some(hps_config::ProcessCondition {
                    command: Some("browser".into()),
                    ..Default::default()
                }),
                ..Default::default()
            },
            RouteAction::Reject,
        )]);
        assert!(matches!(
            Router::compile(&c, RoutingCapabilities::default()),
            Err(ProxyError::UnsupportedRule)
        ));
        let router = Router::compile(
            &c,
            RoutingCapabilities {
                local_process: true,
                udp: false,
            },
        )
        .unwrap();
        let mut ctx = context("site.example");
        ctx.process = Some(ProcessIdentity {
            pid: 123,
            command: "browser".into(),
            cgroup_id: 1,
        });
        assert_eq!(router.evaluate(&ctx, 1, true).action, RouteAction::Direct);
        ctx.origin = FlowOrigin::Gateway;
        assert_eq!(router.evaluate(&ctx, 1, true).action, RouteAction::Direct);
        ctx.origin = FlowOrigin::LinuxLocal;
        assert_eq!(router.evaluate(&ctx, 1, true).action, RouteAction::Reject);
        c.routing.rules[0].enabled = false;
        assert!(Router::compile(&c, RoutingCapabilities::default()).is_ok());
        c.routing.rules = vec![rule(
            "udp",
            RuleConditions {
                transports: vec![SelectionTransport::Udp],
                ..Default::default()
            },
            RouteAction::Direct,
        )];
        assert!(matches!(
            Router::compile(&c, RoutingCapabilities::default()),
            Err(ProxyError::UnsupportedRule)
        ));
    }
}
