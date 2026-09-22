//! Validation for data that may become a governed room-task update.
//!
//! These checks are deliberately pure. In particular, handoff URLs are parsed
//! as data and are never passed to an HTTP client, shell, or browser.

use std::collections::{BTreeSet, HashSet};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;
use url::Url;
use uuid::Uuid;

use crate::RoomContextPacket;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Handoff {
    pub task_id: Uuid,
    pub context_revision: i64,
    pub instruction: String,
    pub url: String,
    pub host: String,
    pub expires_at: DateTime<Utc>,
}

#[derive(Debug, Error)]
pub enum UpdateValidationError {
    #[error("result does not match the packet skill")]
    SkillMismatch,
    #[error("result has an invalid shape")]
    InvalidResult,
    #[error("result cites a source outside the packet")]
    UnknownCitation,
    #[error("handoff is not bound to the packet task and revision")]
    HandoffBindingMismatch,
    #[error("handoff instruction is empty or too long")]
    InvalidHandoffInstruction,
    #[error("handoff URL must be HTTPS without userinfo")]
    InvalidHandoffUrl,
    #[error("handoff host is not its URL host or is not registered")]
    UnregisteredHandoffHost,
    #[error("handoff has expired")]
    ExpiredHandoff,
}

/// Validates the only two terminal result shapes understood by the room
/// contract and makes citations a strict subset of packet event identifiers.
pub fn validate_result(
    packet: &RoomContextPacket,
    result: &Value,
) -> Result<(), UpdateValidationError> {
    let object = result
        .as_object()
        .ok_or(UpdateValidationError::InvalidResult)?;
    let kind = required_string(object, "kind", 1, 64)?;
    match (packet.skill_id.as_str(), kind) {
        ("summarize-context", "context-summary.v1") => {
            require_exact_keys(object, &["kind", "summary", "citations"])?;
            let _ = required_string(object, "summary", 1, 8_000)?;
        }
        ("extract-action-items", "action-items.v1") => {
            require_exact_keys(object, &["kind", "actionItems", "citations"])?;
            let action_items = object
                .get("actionItems")
                .and_then(Value::as_array)
                .filter(|items| items.len() <= 20)
                .ok_or(UpdateValidationError::InvalidResult)?;
            for item in action_items {
                let item = item
                    .as_object()
                    .ok_or(UpdateValidationError::InvalidResult)?;
                if !item
                    .keys()
                    .all(|key| matches!(key.as_str(), "text" | "owner" | "due"))
                {
                    return Err(UpdateValidationError::InvalidResult);
                }
                let _ = required_string(item, "text", 1, 2_000)?;
                for optional in ["owner", "due"] {
                    if let Some(value) = item.get(optional) {
                        let value = value
                            .as_str()
                            .filter(|value| !value.trim().is_empty() && value.len() <= 256)
                            .ok_or(UpdateValidationError::InvalidResult)?;
                        if value != value.trim() {
                            return Err(UpdateValidationError::InvalidResult);
                        }
                    }
                }
            }
        }
        _ => return Err(UpdateValidationError::SkillMismatch),
    }

    let citations = object
        .get("citations")
        .and_then(Value::as_array)
        .filter(|citations| citations.len() <= 100)
        .ok_or(UpdateValidationError::InvalidResult)?;
    let visible = packet_event_ids(packet);
    let mut unique = HashSet::new();
    for citation in citations {
        let citation = citation
            .as_str()
            .and_then(|value| Uuid::parse_str(value).ok())
            .ok_or(UpdateValidationError::InvalidResult)?;
        if !unique.insert(citation) || !visible.contains(&citation) {
            return Err(UpdateValidationError::UnknownCitation);
        }
    }
    Ok(())
}

/// Validates a possible future handoff without navigating to it. The caller
/// receives no transport capability from this function.
pub fn validate_handoff(
    packet: &RoomContextPacket,
    allowed_hosts: &BTreeSet<String>,
    handoff: &Handoff,
    now: DateTime<Utc>,
) -> Result<(), UpdateValidationError> {
    if handoff.task_id != packet.task_id || handoff.context_revision != packet.context_revision {
        return Err(UpdateValidationError::HandoffBindingMismatch);
    }
    if handoff.instruction.trim().is_empty()
        || handoff.instruction != handoff.instruction.trim()
        || handoff.instruction.len() > 2_000
    {
        return Err(UpdateValidationError::InvalidHandoffInstruction);
    }
    if handoff.expires_at <= now || packet.expires_at <= now {
        return Err(UpdateValidationError::ExpiredHandoff);
    }

    let url = Url::parse(&handoff.url).map_err(|_| UpdateValidationError::InvalidHandoffUrl)?;
    if url.scheme() != "https"
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
    {
        return Err(UpdateValidationError::InvalidHandoffUrl);
    }
    let Some(url_host) = url.host_str() else {
        return Err(UpdateValidationError::InvalidHandoffUrl);
    };
    let host = handoff.host.to_ascii_lowercase();
    if host != handoff.host || url_host != host || !allowed_hosts.contains(&host) {
        return Err(UpdateValidationError::UnregisteredHandoffHost);
    }
    Ok(())
}

pub(crate) fn packet_event_ids(packet: &RoomContextPacket) -> HashSet<Uuid> {
    packet
        .events
        .iter()
        .filter_map(|event| event.get("eventId"))
        .filter_map(Value::as_str)
        .filter_map(|value| Uuid::parse_str(value).ok())
        .collect()
}

fn required_string<'a>(
    object: &'a serde_json::Map<String, Value>,
    key: &str,
    minimum: usize,
    maximum: usize,
) -> Result<&'a str, UpdateValidationError> {
    let value = object
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| value.len() >= minimum && value.len() <= maximum)
        .filter(|value| value == &value.trim())
        .ok_or(UpdateValidationError::InvalidResult)?;
    Ok(value)
}

fn require_exact_keys(
    object: &serde_json::Map<String, Value>,
    expected: &[&str],
) -> Result<(), UpdateValidationError> {
    (object.len() == expected.len() && expected.iter().all(|key| object.contains_key(*key)))
        .then_some(())
        .ok_or(UpdateValidationError::InvalidResult)
}
