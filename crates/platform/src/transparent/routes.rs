use super::error;
use crate::rtnetlink::{self, attribute, attributes, u32n};
use hps_config::LinuxCaptureConfig;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs::{File, OpenOptions},
    io::{self, Read, Write},
    os::unix::fs::{MetadataExt, OpenOptionsExt},
    path::PathBuf,
};
const PROTOCOL: u8 = 200;
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Identity {
    table: u32,
    priority: u32,
    mark: u32,
    ipv4: bool,
    ipv6: bool,
}
impl Identity {
    fn from_config(config: &LinuxCaptureConfig) -> Self {
        Self {
            table: config.routing_table,
            priority: config.rule_priority,
            mark: config.capture_mark,
            ipv4: config.ipv4,
            ipv6: config.ipv6,
        }
    }
    fn valid(&self) -> bool {
        (256..=65535).contains(&self.table)
            && (1..32766).contains(&self.priority)
            && self.mark != 0
            && (self.ipv4 || self.ipv6)
    }
    fn families(&self) -> impl Iterator<Item = u8> {
        [
            (libc::AF_INET as u8, self.ipv4),
            (libc::AF_INET6 as u8, self.ipv6),
        ]
        .into_iter()
        .filter_map(|(family, enabled)| enabled.then_some(family))
    }
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Journal {
    format_version: u32,
    netns: u64,
    identities: Vec<Identity>,
}
#[derive(Debug)]
struct Record {
    header: [u8; 12],
    attrs: BTreeMap<u16, Vec<u8>>,
}
impl Record {
    fn number(&self, key: u16) -> Option<u32> {
        self.attrs
            .get(&key)
            .filter(|v| v.len() == 4)
            .map(|v| u32n(v))
    }
    fn table(&self) -> u32 {
        self.number(15).unwrap_or(u32::from(self.header[4]))
    }
    fn priority(&self) -> u32 {
        self.number(6).unwrap_or(0)
    }
}
fn dump(family: u8, rule: bool) -> io::Result<Vec<Record>> {
    let mut payload = [0; 12];
    payload[0] = family;
    let kind = if rule {
        libc::RTM_GETRULE
    } else {
        libc::RTM_GETROUTE
    };
    let expected = if rule {
        libc::RTM_NEWRULE
    } else {
        libc::RTM_NEWROUTE
    };
    let mut result = vec![];
    for message in rtnetlink::exchange(
        kind,
        (libc::NLM_F_REQUEST | libc::NLM_F_DUMP) as u16,
        &payload,
        true,
    )? {
        if message.kind != expected {
            continue;
        }
        if message.payload.len() < 12 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "short routing object",
            ));
        }
        if !rule && u32n(&message.payload[8..]) & 512 != 0 {
            continue;
        } // kernel route-cache clones
        result.push(Record {
            header: message.payload[..12].try_into().unwrap(),
            attrs: attributes(&message.payload, 12)?,
        });
    }
    Ok(result)
}
fn route_payload(identity: &Identity, family: u8, lo: u32) -> Vec<u8> {
    let scope = if family == libc::AF_INET as u8 {
        254
    } else {
        0
    };
    let mut payload = vec![family, 0, 0, 0, 0, PROTOCOL, scope, 2, 0, 0, 0, 0];
    attribute(&mut payload, 4, &lo.to_ne_bytes());
    attribute(&mut payload, 6, &identity.priority.to_ne_bytes());
    attribute(&mut payload, 15, &identity.table.to_ne_bytes());
    payload
}
fn rule_payload(identity: &Identity, family: u8) -> Vec<u8> {
    let mut payload = vec![family, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0];
    attribute(&mut payload, 6, &identity.priority.to_ne_bytes());
    attribute(&mut payload, 10, &identity.mark.to_ne_bytes());
    attribute(&mut payload, 16, &u32::MAX.to_ne_bytes());
    attribute(&mut payload, 15, &identity.table.to_ne_bytes());
    attribute(&mut payload, 21, &[PROTOCOL]);
    payload
}
fn is_route(record: &Record, identity: &Identity, family: u8, lo: u32) -> bool {
    let expected = route_payload(identity, family, lo);
    record.header[..4] == expected[..4]
        && record.header[5..8] == expected[5..8]
        && record.table() == identity.table
        && record.number(4) == Some(lo)
        && record.number(6) == Some(identity.priority)
        && record
            .attrs
            .keys()
            .all(|key| matches!(key, 1 | 4 | 6 | 12 | 15 | 20))
        && record
            .attrs
            .get(&1)
            .is_none_or(|value| value.iter().all(|b| *b == 0))
        && record.attrs.get(&20).is_none_or(|value| value == &[0])
}
fn is_rule(record: &Record, identity: &Identity, family: u8) -> bool {
    record.header[..4] == [family, 0, 0, 0]
        && record.header[5..] == [0, 0, 1, 0, 0, 0, 0]
        && record.table() == identity.table
        && record.priority() == identity.priority
        && record.number(10) == Some(identity.mark)
        && record.number(16) == Some(u32::MAX)
        && record
            .attrs
            .get(&21)
            .is_some_and(|value| value == &[PROTOCOL])
        && record
            .attrs
            .keys()
            .all(|key| matches!(key, 6 | 10 | 14 | 15 | 16 | 18 | 21))
        && record.number(14).is_none_or(|v| v == u32::MAX)
        && record
            .attrs
            .get(&18)
            .is_none_or(|v| v.iter().all(|b| *b == 0))
}
fn local_rule(record: &Record, family: u8) -> bool {
    record.header[..4] == [family, 0, 0, 0]
        && record.header[5..] == [0, 0, 1, 0, 0, 0, 0]
        && record.table() == 255
        && record.priority() == 0
        && record
            .attrs
            .keys()
            .all(|key| matches!(key, 6 | 14 | 15 | 21))
        && record.number(14).is_none_or(|v| v == u32::MAX)
        && record.attrs.get(&21).is_none_or(|v| v == &[2])
}
fn change(payload: &[u8], rule: bool, add: bool) -> io::Result<()> {
    let kind = match (rule, add) {
        (true, true) => libc::RTM_NEWRULE,
        (true, false) => libc::RTM_DELRULE,
        (false, true) => libc::RTM_NEWROUTE,
        (false, false) => libc::RTM_DELROUTE,
    };
    let flags = libc::NLM_F_REQUEST
        | libc::NLM_F_ACK
        | if add {
            libc::NLM_F_CREATE | libc::NLM_F_EXCL
        } else {
            0
        };
    rtnetlink::exchange(kind, flags as u16, payload, false).map(|_| ())
}
fn remove(identity: &Identity, lo: u32) -> io::Result<()> {
    for family in identity.families() {
        remove_object(identity, family, true, lo)?;
        remove_object(identity, family, false, lo)?;
    }
    Ok(())
}
fn remove_object(identity: &Identity, family: u8, rule: bool, lo: u32) -> io::Result<()> {
    for record in dump(family, rule)? {
        let exact = if rule {
            is_rule(&record, identity, family)
        } else {
            is_route(&record, identity, family, lo)
        };
        if exact {
            change(
                &if rule {
                    rule_payload(identity, family)
                } else {
                    route_payload(identity, family, lo)
                },
                rule,
                false,
            )?;
        } else {
            let marked = record.table() == identity.table
                && record.priority() == identity.priority
                && if rule {
                    record.attrs.get(&21).is_some_and(|v| v == &[PROTOCOL])
                        && record.number(10) == Some(identity.mark)
                } else {
                    record.header[5] == PROTOCOL
                };
            if marked {
                return Err(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    "owned routing marker changed; refusing broad deletion",
                ));
            }
        }
    }
    Ok(())
}
fn check_conflicts(identity: &Identity, owned: &[Identity], lo: u32) -> io::Result<()> {
    for family in [libc::AF_INET as u8, libc::AF_INET6 as u8] {
        for route in dump(family, false)? {
            if route.table() == identity.table
                && !owned.iter().any(|old| {
                    old.families().any(|f| f == family) && is_route(&route, old, family, lo)
                })
            {
                return Err(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    "private capture table contains a foreign route",
                ));
            }
        }
        for rule in dump(family, true)? {
            if owned
                .iter()
                .any(|old| old.families().any(|f| f == family) && is_rule(&rule, old, family))
            {
                continue;
            }
            let overlaps = rule.number(10).is_some_and(|mark| {
                let mask = rule.number(16).unwrap_or(u32::MAX);
                identity.mark & mask == mark & mask
            });
            if rule.priority() == identity.priority || overlaps {
                return Err(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    "capture rule priority or mark overlaps a foreign rule",
                ));
            }
            if identity.families().any(|f| f == family)
                && rule.priority() < identity.priority
                && !local_rule(&rule, family)
                && rule.header[7] != 3
            {
                return Err(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    format!(
                        "an earlier foreign routing rule at priority {} may bypass capture",
                        rule.priority()
                    ),
                ));
            }
        }
    }
    Ok(())
}
fn journal_path(netns: u64) -> PathBuf {
    PathBuf::from(format!("/run/hps/transparent-netns-{netns}.json"))
}
fn read_journal(netns: u64) -> io::Result<Vec<Identity>> {
    let path = journal_path(netns);
    let file = match OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC)
        .open(path)
    {
        Ok(file) => file,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(vec![]),
        Err(e) => return Err(e),
    };
    let metadata = file.metadata()?;
    if !metadata.is_file()
        || metadata.uid() != unsafe { libc::geteuid() }
        || metadata.mode() & 0o077 != 0
        || metadata.nlink() != 1
        || metadata.len() > 8192
    {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "capture ownership journal is not a bounded private file",
        ));
    }
    let mut bytes = vec![];
    file.take(8193).read_to_end(&mut bytes)?;
    let journal: Journal = serde_json::from_slice(&bytes).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid capture ownership journal",
        )
    })?;
    if journal.format_version != 1
        || journal.netns != netns
        || journal.identities.len() > 2
        || journal.identities.iter().any(|id| !id.valid())
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "capture ownership journal identity mismatch",
        ));
    }
    Ok(journal.identities)
}
fn write_journal(netns: u64, identities: &[Identity]) -> io::Result<()> {
    let path = journal_path(netns);
    static NONCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let nonce = NONCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let time = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let temporary = path.with_extension(format!("tmp-{}-{time}-{nonce}", std::process::id()));
    let bytes = serde_json::to_vec(&Journal {
        format_version: 1,
        netns,
        identities: identities.to_vec(),
    })
    .map_err(io::Error::other)?;
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(&temporary)?;
    let result = (|| {
        file.write_all(&bytes)?;
        file.sync_all()?;
        std::fs::rename(&temporary, &path)?;
        File::open(path.parent().unwrap())?.sync_all()
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result
}
pub(super) struct OwnedRoutes {
    identity: Identity,
    netns: u64,
    lo: u32,
    created: bool,
    created_routes: Vec<u8>,
    created_rules: Vec<u8>,
}
impl OwnedRoutes {
    pub(super) fn prepare(
        config: &LinuxCaptureConfig,
        netns: u64,
        _ownership: &File,
    ) -> Result<Self, String> {
        let identity = Identity::from_config(config);
        let lo = unsafe { libc::if_nametoindex(c"lo".as_ptr()) };
        if lo == 0 {
            return Err(error("loopback interface", io::Error::last_os_error()));
        }
        let old = read_journal(netns).map_err(|e| error("read ownership journal", e))?;
        check_conflicts(&identity, &old, lo).map_err(|e| error("routing conflict", e))?;
        let mut intent = old.clone();
        if !intent.contains(&identity) {
            intent.push(identity.clone());
        }
        if intent.len() > 2 {
            return Err("capture recovery journal has too many unfinished identities".into());
        }
        write_journal(netns, &intent).map_err(|e| error("write routing intent", e))?;
        for id in &old {
            remove(id, lo).map_err(|e| error("recover owned routing objects", e))?;
        }
        let mut routes = Self {
            identity,
            netns,
            lo,
            created: true,
            created_routes: vec![],
            created_rules: vec![],
        };
        let families = routes.identity.families().collect::<Vec<_>>();
        for family in families {
            change(&route_payload(&routes.identity, family, lo), false, true)
                .map_err(|e| error("create private local route", e))?;
            routes.created_routes.push(family);
            change(&rule_payload(&routes.identity, family), true, true)
                .map_err(|e| error("create marked rule", e))?;
            routes.created_rules.push(family);
        }
        if !routes.present()? {
            routes.cleanup()?;
            return Err("kernel did not retain the owned routing objects".into());
        }
        write_journal(netns, std::slice::from_ref(&routes.identity))
            .map_err(|e| error("commit routing ownership", e))?;
        Ok(routes)
    }
    pub(super) fn present(&self) -> Result<bool, String> {
        check_conflicts(
            &self.identity,
            std::slice::from_ref(&self.identity),
            self.lo,
        )
        .map_err(|e| error("routing conflict", e))?;
        for family in self.identity.families() {
            if !dump(family, true)
                .map_err(|e| error("inspect owned rules", e))?
                .iter()
                .any(|r| is_rule(r, &self.identity, family))
                || !dump(family, false)
                    .map_err(|e| error("inspect owned routes", e))?
                    .iter()
                    .any(|r| is_route(r, &self.identity, family, self.lo))
            {
                return Ok(false);
            }
        }
        Ok(self.created)
    }
    pub(super) fn cleanup(&mut self) -> Result<(), String> {
        if !self.created {
            return Ok(());
        }
        // Rollback only positive acknowledgments from this attempt. A failed
        // CREATE|EXCL must never make us delete a competing process's object.
        while let Some(family) = self.created_rules.last().copied() {
            remove_object(&self.identity, family, true, self.lo)
                .map_err(|e| error("remove owned marked rule", e))?;
            self.created_rules.pop();
        }
        while let Some(family) = self.created_routes.last().copied() {
            remove_object(&self.identity, family, false, self.lo)
                .map_err(|e| error("remove owned local route", e))?;
            self.created_routes.pop();
        }
        write_journal(self.netns, &[]).map_err(|e| error("clear routing journal", e))?;
        self.created = false;
        Ok(())
    }
}
impl Drop for OwnedRoutes {
    fn drop(&mut self) {
        let _ = self.cleanup();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Fixture {
        payload: Vec<u8>,
        rule: bool,
    }
    impl Fixture {
        fn add(payload: Vec<u8>, rule: bool) -> Self {
            change(&payload, rule, true).unwrap();
            Self { payload, rule }
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = change(&self.payload, self.rule, false);
        }
    }
    #[test]
    #[ignore = "requires an isolated privileged Linux network namespace; creates only exact test routing identities"]
    fn real_owned_routes_preserve_foreign_objects_recover_journal_and_remove_exact_identities() {
        let netns = crate::linux_objects::netns_cookie().unwrap();
        let ownership = crate::linux_objects::ownership("transparent", netns).unwrap();
        let config = LinuxCaptureConfig {
            object: "/unused/test-object".into(),
            local_cgroup: Some("/sys/fs/cgroup/test".into()),
            ..Default::default()
        };
        let identity = Identity::from_config(&config);
        let lo = unsafe { libc::if_nametoindex(c"lo".as_ptr()) };
        let mut foreign_route = route_payload(&identity, libc::AF_INET as u8, lo);
        foreign_route[5] = 99;
        let foreign_route = Fixture::add(foreign_route, false);
        assert!(
            OwnedRoutes::prepare(&config, netns, &ownership)
                .err()
                .unwrap()
                .contains("foreign route")
        );
        assert!(
            dump(libc::AF_INET as u8, false)
                .unwrap()
                .iter()
                .any(|r| r.table() == identity.table && r.header[5] == 99)
        );
        drop(foreign_route);
        let mut foreign_rule = rule_payload(&identity, libc::AF_INET as u8);
        // Keep its matching priority but use a different origin.
        for position in (12..foreign_rule.len()).step_by(4) {
            if position + 4 <= foreign_rule.len()
                && rtnetlink::u16n(&foreign_rule[position + 2..]) == 21
            {
                foreign_rule[position + 4] = 99;
                break;
            }
        }
        let foreign_rule = Fixture::add(foreign_rule, true);
        let conflict = OwnedRoutes::prepare(&config, netns, &ownership)
            .err()
            .unwrap();
        assert!(
            conflict.contains("foreign rule"),
            "{conflict}; {:?}",
            dump(libc::AF_INET as u8, true).unwrap()
        );
        assert!(
            dump(libc::AF_INET as u8, true)
                .unwrap()
                .iter()
                .any(|r| r.priority() == identity.priority && r.attrs.get(&21) == Some(&vec![99]))
        );
        drop(foreign_rule);
        let routes = OwnedRoutes::prepare(&config, netns, &ownership).unwrap();
        assert!(routes.present().unwrap());
        // Lose only user-space state to exercise persisted intent recovery; full
        // daemon SIGKILL/forwarding recovery is a separate later integration gate.
        std::mem::forget(routes);
        let mut recovered = OwnedRoutes::prepare(&config, netns, &ownership).unwrap();
        assert!(recovered.present().unwrap());
        recovered.cleanup().unwrap();
        assert!(read_journal(netns).unwrap().is_empty());
        for family in identity.families() {
            assert!(
                !dump(family, true)
                    .unwrap()
                    .iter()
                    .any(|r| is_rule(r, &identity, family))
            );
            assert!(
                !dump(family, false)
                    .unwrap()
                    .iter()
                    .any(|r| is_route(r, &identity, family, lo))
            );
        }
        println!(
            "{}",
            serde_json::json!({"scope":"owned_policy_routing_only","kernel":std::fs::read_to_string("/proc/sys/kernel/osrelease").unwrap().trim(),"netns":netns,"table":identity.table,"rule_priority":identity.priority,"mark":identity.mark,"ipv4":true,"ipv6":true,"foreign_route_preserved":true,"foreign_rule_preserved":true,"journal_recovered":true,"exact_cleanup":true,"forwarding_verified":false})
        );
    }
}
