use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use crate::registry::SkillId;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ClaimRequest {
    pub lease_owner: Uuid,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NormalizedAgentTaskUpdate {
    pub event_type: String,
    pub payload: Value,
    pub occurred_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TaskUpdateRequest {
    pub update_id: Uuid,
    pub context_revision: i64,
    pub update: NormalizedAgentTaskUpdate,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ActiveDecision {
    pub decision_id: Uuid,
    pub title: String,
    pub summary: String,
    pub source_event_ids: Vec<Uuid>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RoomContextPacket {
    pub task_id: Uuid,
    pub room_id: Uuid,
    pub requester_id: Uuid,
    pub agent_id: Uuid,
    pub skill_id: SkillId,
    pub context_revision: i64,
    pub issued_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub input: String,
    pub events: Vec<Value>,
    pub active_decisions: Vec<ActiveDecision>,
    pub canonical_sha256: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ContextResponse {
    pub packet: RoomContextPacket,
    pub lease_token: Uuid,
}

impl ContextResponse {
    pub fn lease(&self) -> AgentTaskLease {
        AgentTaskLease {
            task_id: self.packet.task_id,
            lease_token: self.lease_token,
            expires_at: self.packet.expires_at,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AgentTaskLease {
    pub task_id: Uuid,
    pub lease_token: Uuid,
    pub expires_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TaskUpdateResponse {
    pub events: Vec<Value>,
}
