//! Validation for data that may become a governed room-task update.
//!
//! These checks are deliberately pure. In particular, handoff URLs are parsed
//! as data and are never passed to an HTTP client, shell, or browser.

use std::collections::BTreeSet;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;
use url::Url;
use uuid::Uuid;

use crate::{RoomContextPacket, reference_agent::expected_result};

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
    #[error("result is not the exact deterministic result for the packet")]
    InvalidResult,
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

/// Admits only the exact result deterministically recomputed from the verified
/// packet. This compares summary text, every cited event ID, and the parsed
/// action list; a structurally valid subset or arbitrary replacement is not a
/// room-safe result.
pub fn validate_result(
    packet: &RoomContextPacket,
    result: &Value,
) -> Result<(), UpdateValidationError> {
    (expected_result(packet).ok().as_ref() == Some(result))
        .then_some(())
        .ok_or(UpdateValidationError::InvalidResult)
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
