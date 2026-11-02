//! Bounded, process-local state transitions from observed daemon facts.
use super::*;
use hps_domain::{AlertFeed, AlertNotice, AlertQuery, AlertSave, AlertSettings, RouteAction};
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    time::{Duration, Instant},
};
const LIMIT: usize = 128;
struct Condition {
    observed: bool,
    notified: bool,
    last: Option<Instant>,
    suppressed: u64,
}
#[derive(Default)]
pub(super) struct Runtime {
    state: Mutex<State>,
}
#[derive(Default)]
struct State {
    sequence: u64,
    notices: VecDeque<AlertNotice>,
    conditions: BTreeMap<String, Condition>,
}
struct Fact {
    key: String,
    problem: bool,
    code: &'static str,
    message: &'static str,
}
impl State {
    fn update(
        &mut self,
        now: Instant,
        wall: i64,
        settings: &AlertSettings,
        facts: Vec<Fact>,
        active: BTreeSet<String>,
    ) {
        if !settings.enabled {
            self.conditions.clear();
            return;
        }
        self.conditions.retain(|key, _| active.contains(key));
        for fact in facts {
            let condition = self
                .conditions
                .entry(fact.key.clone())
                .or_insert(Condition {
                    observed: false,
                    notified: false,
                    last: None,
                    suppressed: 0,
                });
            if condition.observed != fact.problem {
                condition.observed = fact.problem;
                if condition.last.is_some_and(|at| {
                    now.saturating_duration_since(at)
                        < Duration::from_secs(settings.cooldown_secs.into())
                }) {
                    condition.suppressed = condition.suppressed.saturating_add(1);
                }
            }
            if condition.observed == condition.notified
                || condition.last.is_some_and(|at| {
                    now.saturating_duration_since(at)
                        < Duration::from_secs(settings.cooldown_secs.into())
                })
            {
                continue;
            }
            condition.notified = condition.observed;
            condition.last = Some(now);
            self.sequence = self.sequence.saturating_add(1);
            self.notices.push_back(AlertNotice {
                sequence: self.sequence,
                occurred_at: wall,
                key: fact.key,
                state: if fact.problem { "problem" } else { "recovered" }.into(),
                code: fact.code.into(),
                message: if fact.problem {
                    fact.message
                } else {
                    "此前告警条件已恢复"
                }
                .into(),
                suppressed_changes: std::mem::take(&mut condition.suppressed),
            });
            while self.notices.len() > LIMIT {
                self.notices.pop_front();
            }
        }
    }
}
impl Supervisor {
    pub(super) async fn sample_alerts(&self, failures: Option<(u64, u64)>) {
        let current = self.current();
        let settings = current.config.alerts.clone();
        let revision = current.revision;
        if !settings.enabled {
            self.alerts.state.lock().unwrap().conditions.clear();
            return;
        }
        let mut facts = vec![];
        let mut active = BTreeSet::new();
        let mut fact = |key: String, problem: bool, code: &'static str, message: &'static str| {
            active.insert(key.clone());
            facts.push(Fact {
                key,
                problem,
                code,
                message,
            });
        };
        if settings.no_exit {
            let needs_proxy =
                |action: &RouteAction| !matches!(action, RouteAction::Direct | RouteAction::Reject);
            let proxy_required = needs_proxy(&current.config.proxy.routing.default_action)
                || current
                    .config
                    .proxy
                    .routing
                    .rules
                    .iter()
                    .any(|r| r.enabled && needs_proxy(&r.action));
            let status = current.engine.status();
            fact(
                "no_exit".into(),
                proxy_required
                    && status.inbounds.iter().any(|i| i.running)
                    && status.eligible_upstreams == 0,
                "no_eligible_exit",
                "代理入口运行中，但当前没有满足全局准入条件的上游节点",
            );
        }
        if settings.failure_rate
            && let Some((failed, total)) = failures
        {
            fact(
                "failure_rate".into(),
                total >= u64::from(settings.failure_min_count)
                    && failed.saturating_mul(100)
                        >= total.saturating_mul(settings.failure_percent.into()),
                "business_failure_rate",
                "最近约一分钟的实际业务失败率达到已配置阈值；策略拒绝不计入失败率",
            );
        }
        if settings.provider_failure {
            let ids = current
                .config
                .providers
                .iter()
                .filter(|p| p.enabled)
                .map(|p| p.id.clone())
                .collect::<Vec<_>>();
            for id in &ids {
                active.insert(format!("provider:{id}"));
            }
            if let Ok(tasks) = self.store.latest_provider_tasks(ids).await {
                for task in tasks {
                    if !current.config.providers.iter().any(|p| {
                        p.enabled
                            && p.id == task.provider_id
                            && hps_provider::fingerprint(p) == task.config_fingerprint
                    }) {
                        active.remove(&format!("provider:{}", task.provider_id));
                        continue;
                    }
                    if matches!(
                        task.state.as_str(),
                        "failed" | "outcome_unknown" | "completed"
                    ) {
                        facts.push(Fact{key:format!("provider:{}",task.provider_id),problem:matches!(task.state.as_str(),"failed"|"outcome_unknown"),code:"provider_extraction_failure",message:"已启用供应商最近一次提取失败或结果不确定；通知不会重放采购请求"});
                    }
                }
            }
        }
        if settings.capture_failure && current.config.linux_capture.is_some() {
            active.insert("linux_capture".into());
            if let Ok(status) =
                tokio::time::timeout(Duration::from_secs(2), self.linux_capture_status()).await
            {
                facts.push(Fact {
                    key: "linux_capture".into(),
                    problem: !status.ready,
                    code: "capture_not_ready",
                    message: "已配置的 Linux 透明代理当前未就绪；通知不修改网络配置",
                });
            }
        }
        if settings.capture_failure && current.config.dns.capture.is_some() {
            let attached = current
                .capture
                .as_ref()
                .and_then(|capture| capture.status().ok())
                .is_some_and(|s| s.attached);
            active.insert("dns_capture".into());
            facts.push(Fact {
                key: "dns_capture".into(),
                problem: !attached,
                code: "dns_capture_not_ready",
                message: "已配置的 Linux DNS 捕获当前未附着",
            });
        }
        if self.current().revision != revision {
            return;
        }
        self.alerts.state.lock().unwrap().update(
            Instant::now(),
            (hps_storage::tasks::unix_ms() / 1000) as i64,
            &settings,
            facts,
            active,
        );
    }
    pub fn alert_feed(&self, query: AlertQuery) -> Result<AlertFeed, ApplicationError> {
        let limit = query.limit.unwrap_or(50);
        if !(1..=100).contains(&limit) {
            return Err(ApplicationError::TaskInput);
        }
        let state = self.alerts.state.lock().unwrap();
        let current = self.current();
        let after = query.after.unwrap_or(0);
        let oldest = state.notices.front().map(|n| n.sequence);
        let records = state
            .notices
            .iter()
            .filter(|n| n.sequence > after)
            .cloned()
            .collect::<Vec<_>>();
        let skip = if query.after.is_none() {
            records.len().saturating_sub(limit as usize)
        } else {
            0
        };
        Ok(AlertFeed {
            epoch:self.event_epoch().into(), oldest_sequence:oldest, latest_sequence:state.sequence,
            truncated:oldest.is_some_and(|n|after.saturating_add(1)<n)||after>state.sequence,
            notices:records.into_iter().skip(skip).take(limit as usize).collect(),
            settings:current.config.alerts.clone(),running_revision:current.revision,retention_limit:LIMIT,resets_on_restart:true,
            source:"observed_runtime_state_transitions_monotonic_cooldown_failure_window_approximately_60_seconds".into()
        })
    }
    pub async fn save_alerts(
        self: &Arc<Self>,
        input: AlertSave,
    ) -> Result<ConfigRevision, ApplicationError> {
        let workspace = self.store.workspace().await?;
        if workspace.desired.revision.id != input.expected_revision
            || self.current().revision != input.expected_revision
        {
            return Err(StorageError::Conflict.into());
        }
        let mut config = self.current().base_config.clone();
        config.alerts = input.settings;
        self.create_revision(ConfigInput {
            toml: toml::to_string(&config).map_err(|_| ApplicationError::Invalid)?,
            expected_revision: input.expected_revision,
            expected_draft_version: None,
            note: "Alert settings candidate".into(),
            use_draft_credentials: false,
        })
        .await
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn facts(problem: bool) -> Vec<Fact> {
        vec![Fact {
            key: "no_exit".into(),
            problem,
            code: "no_eligible_exit",
            message: "safe fixed message",
        }]
    }
    fn active() -> BTreeSet<String> {
        BTreeSet::from(["no_exit".into()])
    }
    #[test]
    fn state_change_cooldown_recovery_and_retention_are_monotonic_and_bounded() {
        let mut state = State::default();
        let settings = AlertSettings {
            cooldown_secs: 5,
            ..Default::default()
        };
        let now = Instant::now();
        state.update(now, 100, &settings, facts(false), active());
        assert!(state.notices.is_empty());
        state.update(now, 100, &settings, facts(true), active());
        assert_eq!(state.notices.len(), 1);
        state.update(
            now + Duration::from_secs(1),
            1,
            &settings,
            facts(true),
            active(),
        );
        assert_eq!(state.notices.len(), 1);
        state.update(
            now + Duration::from_secs(2),
            2,
            &settings,
            facts(false),
            active(),
        );
        assert_eq!(state.notices.len(), 1);
        state.update(
            now + Duration::from_secs(5),
            3,
            &settings,
            facts(false),
            active(),
        );
        assert_eq!(state.notices.len(), 2);
        assert_eq!(state.notices[1].state, "recovered");
        assert_eq!(state.notices[1].suppressed_changes, 1);
        for i in 1..300 {
            state.update(
                now + Duration::from_secs(5 + i * 5),
                i as i64,
                &settings,
                facts(i % 2 == 1),
                active(),
            );
        }
        assert_eq!(state.notices.len(), LIMIT);
        assert_eq!(state.conditions.len(), 1);
        state.update(
            now,
            500,
            &AlertSettings {
                enabled: false,
                ..settings
            },
            facts(true),
            active(),
        );
        assert!(state.conditions.is_empty());
    }
    #[tokio::test]
    async fn actual_no_exit_alert_recovers_when_inbound_stops_and_settings_require_activation() {
        let mut config = DaemonConfig::default();
        config.health.enabled = false;
        config.proxy.inbounds.push(hps_config::InboundConfig {
            id: "alert".into(),
            protocol: hps_config::ProxyProtocol::Http,
            listen: "127.0.0.1:0".parse().unwrap(),
            enabled: true,
            tls: None,
            max_connections: 8,
            bytes_per_second: 0,
        });
        let service = Supervisor::boot(
            Store::open(None, config).unwrap(),
            StartupOverrides::default(),
        )
        .await
        .unwrap();
        service.sample_alerts(None).await;
        let first = service.alert_feed(AlertQuery::default()).unwrap();
        assert_eq!(first.notices.len(), 1);
        assert_eq!(first.notices[0].code, "no_eligible_exit");
        service.current().engine.stop("alert").await.unwrap();
        {
            let mut state = service.alerts.state.lock().unwrap();
            state.conditions.get_mut("no_exit").unwrap().last =
                Some(Instant::now() - Duration::from_secs(61));
        }
        service.sample_alerts(None).await;
        assert_eq!(
            service.alert_feed(AlertQuery::default()).unwrap().notices[1].state,
            "recovered"
        );
        let revision = service
            .save_alerts(AlertSave {
                expected_revision: 1,
                settings: AlertSettings {
                    enabled: false,
                    ..Default::default()
                },
            })
            .await
            .unwrap();
        assert!(service.current().config.alerts.enabled);
        service
            .activate(ConfigActivate {
                revision: revision.id,
                expected_revision: 1,
                defer_until_restart: false,
            })
            .await
            .unwrap();
        assert!(!service.current().config.alerts.enabled);
        service.shutdown().await.unwrap();
    }
}
