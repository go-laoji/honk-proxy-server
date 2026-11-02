//! One decision path for diagnostics and production; only production advances state.
use crate::dial::Upstream;
use hps_config::{GroupConfig, GroupStrategy, SelectionConfig, SessionKey};
use hps_domain::{GroupCandidate, SelectionDecision, SelectionQuery, SelectionTransport};
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

#[derive(Clone)]
pub(crate) struct Choice {
    pub upstream: Arc<Upstream>,
    pub reasons: Vec<String>,
    pub remaining: Option<Duration>,
    pub minimum_lifetime: u64,
    pub sampled: Instant,
    pub latency: Option<u64>,
}
struct Binding {
    node: String,
    identity: String,
    until: Instant,
}
#[derive(Default)]
struct State {
    cursors: HashMap<String, u64>,
    sessions: HashMap<String, Binding>,
    last: HashMap<String, String>,
    last_prune: Option<Instant>,
    retired: bool,
}
struct CompiledGroup {
    index: usize,
    members: HashSet<String>,
    providers: HashSet<String>,
    fallbacks: HashSet<String>,
}
pub(crate) struct Selector {
    compiled: HashMap<String, CompiledGroup>,
    node_tag_sets: HashMap<String, HashSet<String>>,
    provider_tag_sets: HashMap<String, HashSet<String>>,
    config: SelectionConfig,
    state: Mutex<State>,
}
impl Selector {
    pub fn allows_refill(
        &self,
        query: &SelectionQuery,
        provider: &str,
        protocol: Option<hps_config::ProxyProtocol>,
    ) -> bool {
        if query.node.is_some() || self.state.lock().unwrap().retired {
            return false;
        }
        if query.transport == SelectionTransport::Udp
            && protocol != Some(hps_config::ProxyProtocol::Socks5)
        {
            return false;
        }
        let group = query
            .group
            .as_ref()
            .or_else(|| {
                query
                    .inbound
                    .as_ref()
                    .and_then(|i| self.config.inbound_groups.get(i))
            })
            .or(self.config.default_group.as_ref());
        let Some(id) = group else {
            return true;
        };
        let Some(group) = self.group(id) else {
            return false;
        };
        // A nested path may request only sources it explicitly contains. Never refill a pinned scope.
        self.group_allows_refill(group, provider, protocol, 0)
    }
    fn group_allows_refill(
        &self,
        group: &GroupConfig,
        provider: &str,
        protocol: Option<hps_config::ProxyProtocol>,
        depth: usize,
    ) -> bool {
        depth < 8
            && group.pinned_node.is_none()
            && (group.include_all
                || self.compiled[&group.id].providers.contains(provider)
                || group
                    .groups
                    .iter()
                    .filter_map(|id| self.group(id))
                    .any(|g| self.group_allows_refill(g, provider, protocol, depth + 1)))
            && (group.protocols.is_empty()
                || protocol.is_some_and(|p| group.protocols.contains(&p)))
            && group.tags.iter().all(|tag| {
                self.config
                    .provider_tags
                    .get(provider)
                    .is_some_and(|tags| tags.contains(tag))
            })
    }

    pub fn new(config: SelectionConfig) -> Self {
        let compiled = config
            .groups
            .iter()
            .enumerate()
            .map(|(index, g)| {
                (
                    g.id.clone(),
                    CompiledGroup {
                        index,
                        members: g.members.iter().cloned().collect(),
                        providers: g.providers.iter().cloned().collect(),
                        fallbacks: g.fallback_nodes.iter().cloned().collect(),
                    },
                )
            })
            .collect();
        let node_tag_sets = config
            .node_tags
            .iter()
            .map(|(id, tags)| (id.clone(), tags.iter().cloned().collect()))
            .collect();
        let provider_tag_sets = config
            .provider_tags
            .iter()
            .map(|(id, tags)| (id.clone(), tags.iter().cloned().collect()))
            .collect();
        Self {
            config,
            compiled,
            node_tag_sets,
            provider_tag_sets,
            state: Mutex::default(),
        }
    }
    fn group(&self, id: &str) -> Option<&GroupConfig> {
        self.compiled
            .get(id)
            .and_then(|g| self.config.groups.get(g.index))
    }
    fn expand_group(
        &self,
        id: &str,
        choices: &[Choice],
        query: &SelectionQuery,
        cache: &mut HashMap<String, HashSet<String>>,
        depth: usize,
    ) {
        if cache.contains_key(id) || depth >= 8 {
            return;
        }
        let Some(group) = self.group(id) else {
            return;
        };
        for child in &group.groups {
            self.expand_group(child, choices, query, cache, depth + 1);
        }
        let compiled = &self.compiled[id];
        let mut nodes = HashSet::new();
        for choice in choices {
            let up = &choice.upstream;
            if !choice.reasons.is_empty()
                || (query.transport == SelectionTransport::Udp
                    && up.config.protocol != hps_config::ProxyProtocol::Socks5)
                || (group.strategy == GroupStrategy::Latency && choice.latency.is_none())
            {
                continue;
            }
            let provider = up.source.as_ref().map(|(id, _)| id);
            let member = group.include_all
                || compiled.members.contains(&up.config.id)
                || provider.is_some_and(|id| compiled.providers.contains(id))
                || group.groups.iter().any(|id| {
                    cache
                        .get(id)
                        .is_some_and(|nodes| nodes.contains(&up.config.id))
                });
            let tags = self.node_tag_sets.get(&up.config.id);
            let source_tags = provider.and_then(|id| self.provider_tag_sets.get(id));
            if member
                && (group.protocols.is_empty() || group.protocols.contains(&up.config.protocol))
                && group.tags.iter().all(|tag| {
                    tags.is_some_and(|t| t.contains(tag))
                        || source_tags.is_some_and(|t| t.contains(tag))
                })
            {
                nodes.insert(up.config.id.clone());
            }
        }
        if let Some(pin) = &group.pinned_node {
            if nodes.contains(pin) {
                nodes.retain(|id| id == pin);
            } else {
                nodes.retain(|id| compiled.fallbacks.contains(id));
            }
        }
        cache.insert(id.to_owned(), nodes);
    }
    pub fn last(&self) -> HashMap<String, String> {
        self.state.lock().unwrap().last.clone()
    }
    pub fn retire(&self) {
        let mut state = self.state.lock().unwrap();
        state.retired = true;
        state.sessions.clear();
    }
    pub fn decide(
        &self,
        choices: Vec<Choice>,
        query: &SelectionQuery,
        commit: bool,
    ) -> (SelectionDecision, Option<Arc<Upstream>>) {
        let id = query
            .node
            .is_none()
            .then(|| {
                query
                    .group
                    .as_ref()
                    .or_else(|| {
                        query
                            .inbound
                            .as_ref()
                            .and_then(|i| self.config.inbound_groups.get(i))
                    })
                    .or(self.config.default_group.as_ref())
            })
            .flatten();
        let group = id.and_then(|id| self.group(id));
        let unknown = id.is_some() && group.is_none();
        let strategy = if query.node.is_some() {
            GroupStrategy::Fixed
        } else {
            group.map_or(GroupStrategy::RoundRobin, |g| g.strategy)
        };
        let members = group
            .map(|g| g.members.iter().map(String::as_str).collect::<HashSet<_>>())
            .unwrap_or_default();
        let fallbacks = group
            .map(|g| {
                g.fallback_nodes
                    .iter()
                    .map(String::as_str)
                    .collect::<HashSet<_>>()
            })
            .unwrap_or_default();
        let mut nested = HashMap::new();
        if let Some(group) = group {
            for child in &group.groups {
                self.expand_group(child, &choices, query, &mut nested, 0);
            }
        }
        let mut rows = Vec::with_capacity(choices.len());
        for choice in &choices {
            let up = &choice.upstream;
            let provider = up.source.as_ref().map(|(id, _)| id);
            let mut tags = self
                .config
                .node_tags
                .get(&up.config.id)
                .cloned()
                .unwrap_or_default();
            if let Some(tags_from_source) =
                provider.and_then(|id| self.config.provider_tags.get(id))
            {
                tags.extend(tags_from_source.iter().cloned());
            }
            tags.sort();
            tags.dedup();
            let mut reasons = choice.reasons.clone();
            if query.node.as_ref().is_some_and(|id| id != &up.config.id) {
                reasons.push("different_explicit_node".into());
            }
            if unknown {
                reasons.push("unknown_group".into());
            }
            if let Some(group) = group {
                let member = group.include_all
                    || members.contains(up.config.id.as_str())
                    || provider.is_some_and(|id| self.compiled[&group.id].providers.contains(id))
                    || group.groups.iter().any(|id| {
                        nested
                            .get(id)
                            .is_some_and(|nodes| nodes.contains(&up.config.id))
                    });
                if !member {
                    reasons.push("not_member".into());
                }
                if !group.tags.iter().all(|tag| tags.contains(tag)) {
                    reasons.push("tag_filter".into());
                }
                if !group.protocols.is_empty() && !group.protocols.contains(&up.config.protocol) {
                    reasons.push("protocol_filter".into());
                }
                if group.pinned_node.as_ref().is_some_and(|pin| {
                    pin != &up.config.id && !fallbacks.contains(up.config.id.as_str())
                }) {
                    reasons.push("outside_pin_fallback".into());
                }
                if strategy == GroupStrategy::Latency && choice.latency.is_none() {
                    reasons.push("no_fresh_latency".into());
                }
            }
            if query.transport == SelectionTransport::Udp
                && up.config.protocol != hps_config::ProxyProtocol::Socks5
            {
                reasons.push("tcp_only".into());
            }
            rows.push(GroupCandidate {
                node_id: up.config.id.clone(),
                provider_id: provider.cloned(),
                protocol: format!("{:?}", up.config.protocol).to_lowercase(),
                tags,
                eligible: reasons.is_empty(),
                reasons,
                remaining_secs: choice.remaining.map(|r| r.as_secs()),
                latency_ms: choice.latency,
                member_groups: group
                    .map(|g| {
                        g.groups
                            .iter()
                            .filter(|id| {
                                nested
                                    .get(*id)
                                    .is_some_and(|nodes| nodes.contains(&up.config.id))
                            })
                            .cloned()
                            .collect()
                    })
                    .unwrap_or_default(),
                score: group
                    .filter(|g| g.strategy == GroupStrategy::Score)
                    .map(|g| score(g, choice)),
                weight: group
                    .and_then(|g| g.weights.get(&up.config.id))
                    .copied()
                    .unwrap_or(1),
            });
        }
        // Missing leased members remain visible after retirement/removal.
        if let Some(node) = &query.node
            && !rows.iter().any(|r| &r.node_id == node)
        {
            rows.push(GroupCandidate {
                node_id: node.clone(),
                provider_id: None,
                protocol: "unknown".into(),
                tags: vec![],
                eligible: false,
                reasons: vec!["missing_or_retired".into()],
                remaining_secs: None,
                latency_ms: None,
                weight: 1,
                member_groups: vec![],
                score: None,
            });
        }
        if let Some(group) = group {
            let mut present = rows
                .iter()
                .map(|r| r.node_id.clone())
                .collect::<HashSet<_>>();
            for id in group
                .members
                .iter()
                .chain(group.pinned_node.iter())
                .chain(&group.fallback_nodes)
            {
                if present.insert(id.clone()) {
                    rows.push(GroupCandidate {
                        node_id: id.clone(),
                        provider_id: None,
                        protocol: "unknown".into(),
                        tags: vec![],
                        eligible: false,
                        reasons: vec!["missing_or_retired".into()],
                        remaining_secs: None,
                        latency_ms: None,
                        weight: 1,
                        member_groups: vec![],
                        score: None,
                    });
                }
            }
        }
        let mut eligible = rows
            .iter()
            .enumerate()
            .filter_map(|(i, r)| r.eligible.then_some(i))
            .collect::<Vec<_>>();
        if let Some(group) = group {
            let order = if group.pinned_node.is_some() {
                &group.fallback_nodes
            } else {
                &group.members
            };
            let order = order
                .iter()
                .enumerate()
                .map(|(i, id)| (id.as_str(), i))
                .collect::<HashMap<_, _>>();
            eligible.sort_by_key(|&i| {
                order
                    .get(rows[i].node_id.as_str())
                    .copied()
                    .unwrap_or(usize::MAX)
            });
        }
        let now = Instant::now();
        let mut state = self.state.lock().unwrap();
        if commit
            && state
                .last_prune
                .is_none_or(|at| now.saturating_duration_since(at) >= Duration::from_secs(1))
        {
            state.sessions.retain(|_, b| b.until > now);
            state.last_prune = Some(now);
        }
        let scope = id.cloned().unwrap_or_else(|| "@all".into());
        let key = group.and_then(|group| session_key(group, query));
        let pinned = group
            .and_then(|g| g.pinned_node.as_ref())
            .and_then(|pin| eligible.iter().copied().find(|&i| rows[i].node_id == *pin));
        let bound = key
            .as_ref()
            .and_then(|key| state.sessions.get(key))
            .filter(|b| b.until > now)
            .and_then(|b| {
                eligible.iter().copied().find(|&i| {
                    rows[i].node_id == b.node
                        && choices
                            .get(i)
                            .is_some_and(|c| c.upstream.identity == b.identity)
                })
            });
        let cursor = *state.cursors.get(&scope).unwrap_or(&0);
        let selected = pinned.or(bound).or_else(|| {
            if eligible.is_empty() {
                return None;
            }
            match strategy {
                GroupStrategy::Fixed | GroupStrategy::Failover => eligible.first().copied(),
                GroupStrategy::Latency => eligible
                    .iter()
                    .copied()
                    .min_by_key(|&i| (rows[i].latency_ms.unwrap_or(u64::MAX), i)),
                GroupStrategy::Score => eligible.iter().copied().max_by(|&a, &b| {
                    rows[a]
                        .score
                        .as_ref()
                        .map(|s| s.total)
                        .cmp(&rows[b].score.as_ref().map(|s| s.total))
                        .then_with(|| b.cmp(&a))
                }),
                GroupStrategy::RoundRobin => {
                    Some(eligible[(cursor % eligible.len() as u64) as usize])
                }
                GroupStrategy::Weighted => {
                    let total = eligible
                        .iter()
                        .map(|&i| u64::from(rows[i].weight))
                        .sum::<u64>();
                    let mut point = cursor % total;
                    eligible.iter().copied().find(|&i| {
                        let weight = u64::from(rows[i].weight);
                        if point < weight {
                            true
                        } else {
                            point -= weight;
                            false
                        }
                    })
                }
            }
        });
        let session_reused = selected.is_some() && selected == bound;
        let mut reason = if unknown {
            "unknown_group"
        } else if selected.is_none() {
            "no_eligible_candidate"
        } else if pinned.is_some() {
            "pinned"
        } else if session_reused {
            "session"
        } else if group.is_some_and(|g| g.pinned_node.is_some()) {
            "explicit_fallback"
        } else {
            "strategy"
        }
        .to_owned();
        if commit && let Some(index) = selected {
            if !session_reused
                && pinned.is_none()
                && matches!(
                    strategy,
                    GroupStrategy::RoundRobin | GroupStrategy::Weighted
                )
            {
                *state.cursors.entry(scope.clone()).or_default() = cursor.wrapping_add(1);
            }
            if query.node.is_none() {
                state.last.insert(scope, rows[index].node_id.clone());
            }
            if let (Some(key), Some(session)) =
                (key.as_ref(), group.and_then(|g| g.session.as_ref()))
                && !session_reused
                && !state.retired
            {
                let duration =
                    choices[index]
                        .remaining
                        .map_or(Duration::from_secs(session.ttl_secs), |d| {
                            d.saturating_sub(Duration::from_secs(choices[index].minimum_lifetime))
                                .min(Duration::from_secs(session.ttl_secs))
                        });
                if state.sessions.len() < self.config.max_sessions
                    || state.sessions.contains_key(key)
                {
                    state.sessions.insert(
                        key.clone(),
                        Binding {
                            node: rows[index].node_id.clone(),
                            identity: choices[index].upstream.identity.clone(),
                            until: (now + Duration::from_secs(session.ttl_secs))
                                .min(choices[index].sampled + duration),
                        },
                    );
                } else {
                    reason.push_str("_session_capacity");
                }
            }
        }
        let remaining = key
            .as_ref()
            .and_then(|key| state.sessions.get(key))
            .filter(|b| selected.is_some_and(|i| rows[i].node_id == b.node) && b.until > now)
            .map(|b| b.until.saturating_duration_since(now).as_secs());
        let upstream = selected
            .and_then(|i| choices.get(i))
            .map(|c| c.upstream.clone());
        let selected = selected.map(|i| rows[i].node_id.clone());
        let total = rows.len();
        let count = eligible.len();
        rows.sort_by(|a, b| a.node_id.cmp(&b.node_id));
        let limit = query.limit.unwrap_or(50).clamp(1, 200);
        let mut page = rows
            .into_iter()
            .filter(|r| query.after.as_ref().is_none_or(|after| r.node_id > *after))
            .take(limit + 1)
            .collect::<Vec<_>>();
        let more = page.len() > limit;
        page.truncate(limit);
        let next_after = more.then(|| page.last().unwrap().node_id.clone());
        (
            SelectionDecision {
                group: id.cloned(),
                strategy: match strategy {
                    GroupStrategy::Fixed => "fixed",
                    GroupStrategy::Failover => "failover",
                    GroupStrategy::Latency => "latency",
                    GroupStrategy::RoundRobin => "round_robin",
                    GroupStrategy::Weighted => "weighted",
                    GroupStrategy::Score => "score",
                }
                .into(),
                selected,
                reason,
                session_reused,
                session_remaining_secs: remaining,
                candidates: page,
                total,
                eligible: count,
                next_after,
                sessions: state.sessions.len(),
            },
            upstream,
        )
    }
}
fn score(group: &GroupConfig, choice: &Choice) -> hps_domain::CandidateScore {
    use hps_domain::{CandidateScore, ScoreComponent};
    let policy = &group.score;
    let weight_sum = u64::from(policy.latency_weight)
        + u64::from(policy.lifetime_weight)
        + u64::from(policy.priority_weight);
    let values = [
        (
            "fresh_health_probe_latency_ms",
            choice.latency,
            choice.latency.map(|ms| {
                1000u64.saturating_sub(ms.saturating_mul(1000) / policy.latency_ceiling_ms)
            }),
            policy.latency_weight,
        ),
        (
            "admission_remaining_lifetime_secs",
            choice.remaining.map(|d| d.as_secs()),
            choice
                .remaining
                .map(|d| d.as_secs().saturating_mul(1000) / policy.lifetime_target_secs),
            policy.lifetime_weight,
        ),
        (
            "configured_operator_priority",
            group
                .weights
                .get(&choice.upstream.config.id)
                .map(|v| u64::from(*v)),
            group
                .weights
                .get(&choice.upstream.config.id)
                .map(|v| u64::from(*v)),
            policy.priority_weight,
        ),
    ];
    let components = values
        .into_iter()
        .filter(|(_, _, _, weight)| *weight > 0)
        .map(|(source, observed, normalized, weight)| {
            let normalized = normalized.map(|n| n.min(1000) as u16);
            let missing_penalty = if normalized.is_none() {
                policy.missing_evidence_penalty
            } else {
                0
            };
            let contribution = (u64::from(normalized.unwrap_or(0)) * u64::from(weight)
                / weight_sum.max(1)) as i64
                - i64::from(missing_penalty);
            ScoreComponent {
                source: source.into(),
                observed,
                normalized,
                weight,
                contribution,
                missing_penalty,
            }
        })
        .collect::<Vec<_>>();
    CandidateScore {
        total: components.iter().map(|c| c.contribution).sum(),
        components,
    }
}
fn session_key(group: &GroupConfig, query: &SelectionQuery) -> Option<String> {
    let session = group.session.as_ref()?;
    let source = query.source_ip?;
    let mut hash = Sha256::new();
    hash.update(group.id.as_bytes());
    hash.update([0]);
    hash.update(query.inbound.as_deref().unwrap_or("unknown").as_bytes());
    hash.update([0]);
    hash.update(source.to_string().as_bytes());
    if session.key == SessionKey::SourceDestination {
        hash.update([0]);
        hash.update(
            query
                .destination_host
                .as_ref()?
                .to_ascii_lowercase()
                .as_bytes(),
        );
        hash.update(query.destination_port?.to_be_bytes());
    }
    Some(format!("{:x}", hash.finalize()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use hps_config::{GroupSession, ProxyProtocol, UpstreamConfig};
    fn choices() -> Vec<Choice> {
        ["a", "b", "c"]
            .into_iter()
            .map(|id| Choice {
                upstream: Arc::new(
                    Upstream::prepare(UpstreamConfig {
                        id: id.into(),
                        protocol: ProxyProtocol::Http,
                        host: format!("{id}.example"),
                        port: 80,
                        enabled: true,
                        credentials: None,
                        expires_at_unix: None,
                        ca_certificate: None,
                        tls_server_name: None,
                    })
                    .unwrap(),
                ),
                reasons: vec![],
                remaining: Some(Duration::from_secs(30)),
                minimum_lifetime: 10,
                sampled: Instant::now(),
                latency: Some(if id == "b" { 5 } else { 20 }),
            })
            .collect()
    }
    fn selector(group: GroupConfig) -> Selector {
        Selector::new(SelectionConfig {
            default_group: Some("g".into()),
            groups: vec![group],
            ..Default::default()
        })
    }
    fn group(strategy: GroupStrategy) -> GroupConfig {
        GroupConfig {
            id: "g".into(),
            strategy,
            include_all: true,
            ..Default::default()
        }
    }
    #[test]
    fn fixed_failure_stays_in_explicit_ordered_fallback_and_never_crosses_membership() {
        let mut g = group(GroupStrategy::Fixed);
        g.pinned_node = Some("a".into());
        let s = selector(g.clone());
        assert!(!s.allows_refill(
            &SelectionQuery::default(),
            "source",
            Some(ProxyProtocol::Http)
        ));
        let mut nodes = choices();
        nodes[0].reasons.push("disabled".into());
        assert!(
            s.decide(nodes, &SelectionQuery::default(), true)
                .0
                .selected
                .is_none()
        );
        g.fallback_nodes = vec!["c".into(), "b".into()];
        let s = selector(g.clone());
        let mut nodes = choices();
        nodes[0].reasons.push("expired".into());
        let d = s.decide(nodes, &SelectionQuery::default(), true).0;
        assert_eq!(d.selected.as_deref(), Some("c"));
        assert_eq!(d.reason, "explicit_fallback");
        g.include_all = false;
        g.members = vec!["b".into()];
        let s = selector(g);
        assert_eq!(
            s.decide(choices(), &SelectionQuery::default(), true)
                .0
                .selected
                .as_deref(),
            Some("b")
        );
        assert_eq!(
            s.decide(
                choices(),
                &SelectionQuery {
                    transport: SelectionTransport::Udp,
                    ..Default::default()
                },
                false
            )
            .0
            .eligible,
            0
        );
    }
    #[test]
    fn weighted_readonly_preview_latency_missing_evidence_tags_and_empty_groups_are_explained() {
        let mut g = group(GroupStrategy::Weighted);
        g.include_all = false;
        g.members = vec!["a".into(), "b".into()];
        g.weights.insert("b".into(), 3);
        let s = selector(g);
        for _ in 0..3 {
            assert_eq!(
                s.decide(choices(), &SelectionQuery::default(), false)
                    .0
                    .selected
                    .as_deref(),
                Some("a")
            );
        }
        let results = (0..8)
            .map(|_| {
                s.decide(choices(), &SelectionQuery::default(), true)
                    .0
                    .selected
                    .unwrap()
            })
            .collect::<Vec<_>>();
        assert_eq!(results, vec!["a", "b", "b", "b", "a", "b", "b", "b"]);
        let s = selector(group(GroupStrategy::Latency));
        assert_eq!(
            s.decide(choices(), &SelectionQuery::default(), false)
                .0
                .selected
                .as_deref(),
            Some("b")
        );
        let mut nodes = choices();
        for n in &mut nodes {
            n.latency = None;
        }
        assert!(
            s.decide(nodes, &SelectionQuery::default(), false)
                .0
                .candidates
                .iter()
                .all(|c| c.reasons.contains(&"no_fresh_latency".into()))
        );
        let mut g = group(GroupStrategy::Failover);
        g.tags = vec!["premium".into()];
        let mut policy = SelectionConfig {
            default_group: Some("g".into()),
            groups: vec![g],
            ..Default::default()
        };
        policy.node_tags.insert("c".into(), vec!["premium".into()]);
        assert_eq!(
            Selector::new(policy)
                .decide(choices(), &SelectionQuery::default(), false)
                .0
                .selected
                .as_deref(),
            Some("c")
        );
        let s = selector(GroupConfig {
            id: "g".into(),
            members: vec!["lease:42".into()],
            ..Default::default()
        });
        let d = s.decide(choices(), &SelectionQuery::default(), false).0;
        assert!(d.selected.is_none());
        assert!(
            d.candidates
                .iter()
                .any(|c| c.node_id == "lease:42" && c.reasons == ["missing_or_retired"])
        );
    }
    #[test]
    fn sessions_are_bounded_by_lifetime_and_capacity_rechecked_and_isolated_by_inbound() {
        let mut g = group(GroupStrategy::RoundRobin);
        g.session = Some(GroupSession {
            key: SessionKey::SourceDestination,
            ttl_secs: 3600,
        });
        let mut policy = SelectionConfig {
            default_group: Some("g".into()),
            groups: vec![g],
            max_sessions: 1,
            ..Default::default()
        };
        policy.inbound_groups.insert("in".into(), "g".into());
        let s = Selector::new(policy);
        let q = SelectionQuery {
            source_ip: Some("127.0.0.1".parse().unwrap()),
            inbound: Some("in".into()),
            destination_host: Some("site.example".into()),
            destination_port: Some(443),
            ..Default::default()
        };
        let d = s.decide(choices(), &q, true).0;
        assert_eq!(d.selected.as_deref(), Some("a"));
        assert!(d.session_remaining_secs.is_some_and(|s| s <= 20));
        assert!(s.decide(choices(), &q, true).0.session_reused);
        let mut nodes = choices();
        nodes[0].reasons.push("health_unavailable".into());
        assert_ne!(s.decide(nodes, &q, true).0.selected.as_deref(), Some("a"));
        let mut other = q.clone();
        other.inbound = Some("other".into());
        let d = s.decide(choices(), &other, true).0;
        assert!(!d.session_reused);
        assert!(d.reason.ends_with("session_capacity"));
        assert_eq!(d.sessions, 1);
        s.state
            .lock()
            .unwrap()
            .sessions
            .values_mut()
            .for_each(|b| b.until = Instant::now());
        assert!(!s.decide(choices(), &q, true).0.session_reused);
        let d = s
            .decide(
                choices(),
                &SelectionQuery {
                    group: Some("missing".into()),
                    ..q
                },
                false,
            )
            .0;
        assert!(d.selected.is_none());
        assert_eq!(d.reason, "unknown_group");
        s.retire();
        assert_eq!(s.decide(choices(), &other, true).0.sessions, 0);
    }
    #[test]
    fn nested_candidates_keep_child_filters_pin_fallback_and_parent_udp_scope() {
        let mut child = group(GroupStrategy::Fixed);
        child.id = "child".into();
        child.pinned_node = Some("a".into());
        child.fallback_nodes = vec!["b".into()];
        let parent = GroupConfig {
            id: "parent".into(),
            groups: vec!["child".into()],
            ..Default::default()
        };
        let selector = Selector::new(SelectionConfig {
            groups: vec![parent, child],
            default_group: Some("parent".into()),
            ..Default::default()
        });
        let decision = selector
            .decide(choices(), &SelectionQuery::default(), false)
            .0;
        assert_eq!(decision.selected.as_deref(), Some("a"));
        assert_eq!(decision.eligible, 1);
        assert_eq!(decision.candidates[0].member_groups, vec!["child"]);
        let mut failed = choices();
        failed[0].reasons.push("health_unavailable".into());
        let decision = selector.decide(failed, &SelectionQuery::default(), false).0;
        assert_eq!(decision.selected.as_deref(), Some("b"));
        assert_eq!(decision.eligible, 1);
        let decision = selector
            .decide(
                choices(),
                &SelectionQuery {
                    transport: SelectionTransport::Udp,
                    ..Default::default()
                },
                false,
            )
            .0;
        assert!(decision.selected.is_none());
    }
    #[test]
    fn advanced_score_lists_sources_components_and_missing_evidence_penalties() {
        let mut group = group(GroupStrategy::Score);
        group.weights.insert("a".into(), 1000);
        group.weights.insert("b".into(), 100);
        let selector = selector(group);
        let mut nodes = choices();
        nodes[0].latency = None;
        let decision = selector.decide(nodes, &SelectionQuery::default(), false).0;
        let a = decision
            .candidates
            .iter()
            .find(|n| n.node_id == "a")
            .unwrap()
            .score
            .as_ref()
            .unwrap();
        assert_eq!(a.components[0].source, "fresh_health_probe_latency_ms");
        assert!(a.components[0].observed.is_none());
        assert_eq!(a.components[0].missing_penalty, 250);
        assert_eq!(
            a.total,
            a.components.iter().map(|c| c.contribution).sum::<i64>()
        );
        assert!(decision.candidates.iter().all(|n| {
            n.score
                .as_ref()
                .unwrap()
                .components
                .iter()
                .all(|c| !c.source.contains("cost") && !c.source.contains("region"))
        }));
        assert_eq!(decision.selected.as_deref(), Some("b"));
    }
}
