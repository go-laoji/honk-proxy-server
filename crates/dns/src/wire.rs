use crate::{DnsError, Result};
use hickory_proto::{
    op::{Message, MessageType, OpCode, Query, ResponseCode},
    rr::{DNSClass, Name, RData, RecordType},
};
use std::net::IpAddr;
use std::str::FromStr;

/// Follow the queried owner and its CNAME chain only. An unrelated A/AAAA in
/// an answer section is not evidence about the queried name.
pub fn addresses(message: &Message) -> Vec<IpAddr> {
    if message.metadata.response_code != ResponseCode::NoError {
        return vec![];
    }
    let Some(question) = message.queries.first() else {
        return vec![];
    };
    let mut owner = question.name().clone();
    let mut visited = std::collections::HashSet::new();
    for _ in 0..16 {
        if !visited.insert(owner.clone()) {
            return vec![];
        }
        let ips = message
            .answers
            .iter()
            .filter(|r| same_name(&r.name, &owner) && r.dns_class == DNSClass::IN)
            .filter_map(|r| match r.data {
                RData::A(a) => Some(IpAddr::V4(a.0)),
                RData::AAAA(a) => Some(IpAddr::V6(a.0)),
                _ => None,
            })
            .collect::<Vec<_>>();
        if !ips.is_empty() {
            return ips;
        }
        let Some(next) = message.answers.iter().find_map(|r| match &r.data {
            RData::CNAME(name) if same_name(&r.name, &owner) && r.dns_class == DNSClass::IN => {
                Some(name.0.clone())
            }
            _ => None,
        }) else {
            return vec![];
        };
        owner = next;
    }
    vec![]
}
fn same_name(left: &Name, right: &Name) -> bool {
    left.to_ascii()
        .trim_end_matches('.')
        .eq_ignore_ascii_case(right.to_ascii().trim_end_matches('.'))
}
fn positive(message: &Message) -> bool {
    let Some(question) = message.queries.first() else {
        return false;
    };
    match question.query_type() {
        RecordType::A => return addresses(message).iter().any(IpAddr::is_ipv4),
        RecordType::AAAA => return addresses(message).iter().any(IpAddr::is_ipv6),
        _ => {}
    }
    let mut owner = question.name().clone();
    for _ in 0..16 {
        if message.answers.iter().any(|r| {
            same_name(&r.name, &owner)
                && r.dns_class == DNSClass::IN
                && r.data.record_type() == question.query_type()
        }) {
            return true;
        }
        let Some(next) = message.answers.iter().find_map(|r| match &r.data {
            RData::CNAME(name) if same_name(&r.name, &owner) && r.dns_class == DNSClass::IN => {
                Some(name.0.clone())
            }
            _ => None,
        }) else {
            return false;
        };
        owner = next;
    }
    false
}

pub fn query(name: &str, kind: &str) -> Result<Message> {
    if name.len() > 253 || !hps_config::valid_query_type(kind) {
        return Err(DnsError::InvalidQuery);
    }
    let mut name = Name::from_ascii(name).map_err(|_| DnsError::InvalidQuery)?;
    name.set_fqdn(true);
    let kind = RecordType::from_str(kind).map_err(|_| DnsError::InvalidQuery)?;
    let mut message = Message::query();
    message.metadata.recursion_desired = true;
    message.queries.push(Query::query(name, kind));
    Ok(message)
}
pub fn validate_query(message: &Message) -> Result<()> {
    if message.metadata.message_type != MessageType::Query
        || message.metadata.op_code != OpCode::Query
        || message.metadata.truncation
        || message.metadata.response_code != ResponseCode::NoError
        || message.queries.len() != 1
        || !message.answers.is_empty()
        || !message.authorities.is_empty()
        || !message.additionals.is_empty()
        || message.signature.is_some()
        || message.edns.as_ref().is_some_and(|e| e.version() != 0)
    {
        return Err(DnsError::InvalidQuery);
    }
    let question = &message.queries[0];
    if question.query_class() != DNSClass::IN
        || !hps_config::valid_query_type(&question.query_type().to_string())
        || message.to_vec().map_or(true, |wire| wire.len() > 4096)
    {
        return Err(DnsError::InvalidQuery);
    }
    Ok(())
}
pub fn validate_response(request: &Message, response: &Message) -> Result<()> {
    if response.metadata.id != request.metadata.id
        || response.metadata.message_type != MessageType::Response
        || response.metadata.op_code != OpCode::Query
        || response.queries != request.queries
        || response.answers.len() + response.authorities.len() + response.additionals.len() > 512
        || response.signature.is_some()
    {
        return Err(DnsError::InvalidResponse);
    }
    Ok(())
}
/// Conservative lifetime: never raise a wire TTL. NXDOMAIN/NODATA requires an SOA
/// and uses min(SOA TTL, SOA.MINIMUM), with CNAME TTLs also bounding the answer.
pub fn cache_ttl(message: &Message) -> Option<u32> {
    if message.metadata.truncation {
        return None;
    }
    let soa = message
        .authorities
        .iter()
        .filter_map(|r| match &r.data {
            RData::SOA(soa) if r.dns_class == DNSClass::IN => Some(r.ttl.min(soa.minimum)),
            _ => None,
        })
        .min();
    let answers = message.answers.iter().map(|r| r.ttl).min();
    let positive = positive(message);
    let ttl = match message.metadata.response_code {
        ResponseCode::NoError if positive => answers?,
        ResponseCode::NoError | ResponseCode::NXDomain => soa?.min(answers.unwrap_or(u32::MAX)),
        _ => return None,
    };
    (ttl > 0).then_some(ttl.min(86400))
}
pub fn age(message: &Message, seconds: u32, id: u16) -> Message {
    let mut result = message.clone();
    let negative = message.metadata.response_code == ResponseCode::NXDomain
        || (message.metadata.response_code == ResponseCode::NoError && !positive(message));
    result.metadata.id = id;
    for record in result
        .answers
        .iter_mut()
        .chain(&mut result.authorities)
        .chain(&mut result.additionals)
    {
        if negative && let RData::SOA(soa) = &record.data {
            record.ttl = record.ttl.min(soa.minimum);
        }
        record.ttl = record.ttl.saturating_sub(seconds);
    }
    result
}
