//! The deliberately small deterministic Reference Agent.
//!
//! It accepts one already-authorized, hash-verified room packet as A2A JSON
//! text. It has no filesystem, database, tool, model, URL-fetch, shell, or
//! external-network capability.

#[cfg(feature = "reference-agent-server")]
use std::sync::Arc;

#[cfg(feature = "reference-agent-server")]
use a2a::PartContent;
use a2a::{
    AgentCapabilities, AgentCard, AgentInterface, AgentSkill, Artifact, Message, Part, Role,
    TRANSPORT_PROTOCOL_JSONRPC, Task, TaskState, TaskStatus,
    event::{StreamResponse, TaskStatusUpdateEvent},
};
#[cfg(feature = "reference-agent-server")]
use a2a_server::{AgentExecutor, DefaultRequestHandler, InMemoryTaskStore, StaticAgentCard};
#[cfg(feature = "reference-agent-server")]
use axum::{
    Router,
    extract::{Request, State},
    http::{StatusCode, header::AUTHORIZATION},
    middleware::{self, Next},
    response::Response,
};
use chrono::{DateTime, Utc};
#[cfg(feature = "reference-agent-server")]
use futures::stream::{self, BoxStream};
use serde::Serialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use thiserror::Error;
use uuid::Uuid;

use crate::{ActiveDecision, RoomContextPacket};

pub const REFERENCE_AGENT_ID: Uuid = Uuid::from_u128(0x74686f756768746b_686f72616c000003);
pub const REFERENCE_AGENT_ENDPOINT: &str = "http://127.0.0.1:9090/jsonrpc";
pub const REFERENCE_AGENT_CARD: &str = "http://127.0.0.1:9090/.well-known/agent-card.json";

#[derive(Debug, Error)]
pub enum PacketError {
    #[error("packet has an unregistered agent or skill")]
    UnknownAgentOrSkill,
    #[error("packet revision or time range is invalid")]
    InvalidTimeRange,
    #[error("packet has expired")]
    Expired,
    #[error("packet hash does not match its canonical fields")]
    HashMismatch,
    #[error("packet contains duplicate or malformed event identifiers")]
    InvalidEventIds,
    #[error("packet has no matching task invocation event")]
    MissingInvocation,
    #[error("packet task invocation differs from the authorized input")]
    InvocationMismatch,
    #[error("A2A message is missing or is not a user message with one part")]
    InvalidA2aMessage,
    #[error("A2A message part is not JSON packet text")]
    InvalidA2aPart,
    #[error("A2A packet text cannot be decoded")]
    InvalidA2aPacket,
    #[error("A2A task identifier does not match the packet task")]
    A2aTaskMismatch,
    #[error("packet cannot be canonically serialized")]
    Canonicalization,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CanonicalPacket<'a> {
    task_id: Uuid,
    room_id: Uuid,
    requester_id: Uuid,
    agent_id: Uuid,
    skill_id: &'a str,
    context_revision: i64,
    issued_at: DateTime<Utc>,
    expires_at: DateTime<Utc>,
    input: &'a str,
    events: &'a [Value],
    active_decisions: &'a [ActiveDecision],
}

/// Recreates the broker's explicit canonical representation. The unkeyed hash
/// detects accidental or tampered transport changes; authorization remains at
/// the room gateway and lease boundary.
pub fn canonical_packet_sha256(packet: &RoomContextPacket) -> Result<String, PacketError> {
    let canonical = CanonicalPacket {
        task_id: packet.task_id,
        room_id: packet.room_id,
        requester_id: packet.requester_id,
        agent_id: packet.agent_id,
        skill_id: &packet.skill_id,
        context_revision: packet.context_revision,
        issued_at: packet.issued_at,
        expires_at: packet.expires_at,
        input: &packet.input,
        events: &packet.events,
        active_decisions: &packet.active_decisions,
    };
    serde_json::to_vec(&canonical)
        .map(|bytes| format!("{:x}", Sha256::digest(bytes)))
        .map_err(|_| PacketError::Canonicalization)
}

pub fn verify_packet(packet: &RoomContextPacket) -> Result<(), PacketError> {
    verify_packet_at(packet, Utc::now())
}

pub fn verify_packet_at(packet: &RoomContextPacket, now: DateTime<Utc>) -> Result<(), PacketError> {
    if packet.agent_id != REFERENCE_AGENT_ID
        || !matches!(
            packet.skill_id.as_str(),
            "summarize-context" | "extract-action-items"
        )
        || packet.context_revision < 0
    {
        return Err(PacketError::UnknownAgentOrSkill);
    }
    if packet.issued_at > packet.expires_at {
        return Err(PacketError::InvalidTimeRange);
    }
    if packet.expires_at <= now {
        return Err(PacketError::Expired);
    }
    if canonical_packet_sha256(packet)? != packet.canonical_sha256 {
        return Err(PacketError::HashMismatch);
    }

    let mut ids = std::collections::HashSet::new();
    for event in &packet.events {
        let Some(event_id) = event
            .get("eventId")
            .and_then(Value::as_str)
            .and_then(|value| Uuid::parse_str(value).ok())
        else {
            return Err(PacketError::InvalidEventIds);
        };
        if !ids.insert(event_id) {
            return Err(PacketError::InvalidEventIds);
        }
    }

    let invocation = invocation_event(packet).ok_or(PacketError::MissingInvocation)?;
    if invocation.pointer("/payload/input").and_then(Value::as_str) != Some(packet.input.as_str()) {
        return Err(PacketError::InvocationMismatch);
    }
    Ok(())
}

/// Produces a stable A2A stream for an authorized packet. No task output is
/// derived from message bodies, decision summaries, or arbitrary URLs.
pub fn stream_for_packet(packet: &RoomContextPacket) -> Result<Vec<StreamResponse>, PacketError> {
    verify_packet(packet)?;
    let task_id = packet.task_id.to_string();
    let context_id = format!("{}:{}", packet.room_id, packet.context_revision);
    let result = expected_result(packet)?;
    let binding = json!({
        "taskId": packet.task_id,
        "roomId": packet.room_id,
        "contextRevision": packet.context_revision,
        "canonicalSha256": packet.canonical_sha256,
    });
    let artifact = Artifact {
        artifact_id: format!("{}:result", packet.task_id),
        name: Some("governed-room-result".to_owned()),
        description: Some(
            "Deterministic result bound to the authorized context packet.".to_owned(),
        ),
        parts: vec![Part::data(json!({ "packet": binding, "result": result }))],
        metadata: None,
        extensions: None,
    };

    Ok(vec![
        StreamResponse::Task(Task {
            id: task_id.clone(),
            context_id: context_id.clone(),
            status: TaskStatus {
                state: TaskState::Submitted,
                message: None,
                timestamp: None,
            },
            artifacts: None,
            history: None,
            metadata: None,
        }),
        working_update(&task_id, &context_id, "Reading authorized room context"),
        working_update(&task_id, &context_id, "Preparing cited result"),
        StreamResponse::Task(Task {
            id: task_id,
            context_id,
            status: TaskStatus {
                state: TaskState::Completed,
                message: None,
                timestamp: None,
            },
            artifacts: Some(vec![artifact]),
            history: None,
            metadata: None,
        }),
    ])
}

pub fn agent_card() -> AgentCard {
    AgentCard {
        name: "Reference Agent".to_owned(),
        description: "Deterministic, packet-bound room task processor.".to_owned(),
        version: a2a::VERSION.to_owned(),
        supported_interfaces: vec![AgentInterface::new(
            REFERENCE_AGENT_ENDPOINT,
            TRANSPORT_PROTOCOL_JSONRPC,
        )],
        capabilities: AgentCapabilities {
            streaming: Some(true),
            push_notifications: Some(false),
            extensions: None,
            extended_agent_card: None,
        },
        default_input_modes: vec!["application/json".to_owned()],
        default_output_modes: vec!["application/json".to_owned()],
        skills: vec![
            skill(
                "extract-action-items",
                "Extract action items",
                "Extracts strictly formatted action items from the authorized task input.",
            ),
            skill(
                "summarize-context",
                "Summarize context",
                "Summarizes authorized room metadata deterministically.",
            ),
        ],
        provider: None,
        documentation_url: None,
        icon_url: None,
        security_schemes: None,
        security_requirements: None,
        signatures: None,
    }
}

/// Builds only the two local A2A routes. The caller must bind this router to a
/// loopback address; the supplied secret guards both card discovery and RPC.
#[cfg(feature = "reference-agent-server")]
pub fn local_router(secret: impl Into<String>) -> Router {
    let handler = Arc::new(DefaultRequestHandler::new(
        ReferenceAgentExecutor,
        InMemoryTaskStore::new(),
    ));
    let card = Arc::new(StaticAgentCard::new(agent_card()));
    Router::new()
        .nest("/jsonrpc", a2a_server::jsonrpc::jsonrpc_router(handler))
        .merge(a2a_server::agent_card::agent_card_router(card))
        .layer(middleware::from_fn_with_state(
            Arc::<str>::from(secret.into()),
            require_bearer,
        ))
}

#[cfg(feature = "reference-agent-server")]
struct ReferenceAgentExecutor;

#[cfg(feature = "reference-agent-server")]
impl AgentExecutor for ReferenceAgentExecutor {
    fn execute(
        &self,
        context: a2a_server::ExecutorContext,
    ) -> BoxStream<'static, Result<StreamResponse, a2a::A2AError>> {
        let result = packet_from_message(context.message.as_ref())
            .and_then(|packet| {
                (context.task_id == packet.task_id.to_string())
                    .then_some(packet)
                    .ok_or(PacketError::A2aTaskMismatch)
            })
            .and_then(|packet| stream_for_packet(&packet));
        match result {
            Ok(events) => Box::pin(stream::iter(events.into_iter().map(Ok))),
            Err(error) => Box::pin(stream::once(async move {
                Err(a2a::A2AError::invalid_request(error.to_string()))
            })),
        }
    }

    fn cancel(
        &self,
        _context: a2a_server::ExecutorContext,
    ) -> BoxStream<'static, Result<StreamResponse, a2a::A2AError>> {
        Box::pin(stream::once(async {
            Err(a2a::A2AError::invalid_request(
                "Reference Agent tasks cannot be canceled through A2A",
            ))
        }))
    }
}

#[cfg(feature = "reference-agent-server")]
async fn require_bearer(
    State(secret): State<Arc<str>>,
    request: Request,
    next: Next,
) -> Result<Response, StatusCode> {
    let authorized = request
        .headers()
        .get(AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .is_some_and(|provided| provided == secret.as_ref());
    if authorized {
        Ok(next.run(request).await)
    } else {
        Err(StatusCode::UNAUTHORIZED)
    }
}

#[cfg(feature = "reference-agent-server")]
fn packet_from_message(message: Option<&Message>) -> Result<RoomContextPacket, PacketError> {
    let message = message.ok_or(PacketError::InvalidA2aMessage)?;
    if message.role != Role::User || message.parts.len() != 1 {
        return Err(PacketError::InvalidA2aMessage);
    }
    let PartContent::Text(data) = &message.parts[0].content else {
        return Err(PacketError::InvalidA2aPart);
    };
    let data: Value = serde_json::from_str(data).map_err(|_| PacketError::InvalidA2aPacket)?;
    serde_json::from_value::<RoomContextPacket>(
        data.get("packet")
            .cloned()
            .ok_or(PacketError::InvalidA2aPacket)?,
    )
    .map_err(|_| PacketError::InvalidA2aPacket)
}

/// Recomputes the sole allowed terminal result for a verified packet. This is
/// shared by the local server and the gateway admission check so a syntactically
/// valid but agent-invented result cannot cross the room boundary.
pub(crate) fn expected_result(packet: &RoomContextPacket) -> Result<Value, PacketError> {
    verify_packet(packet)?;
    match packet.skill_id.as_str() {
        "summarize-context" => Ok(summary_result(packet)),
        "extract-action-items" => action_items_result(packet),
        _ => Err(PacketError::UnknownAgentOrSkill),
    }
}

fn summary_result(packet: &RoomContextPacket) -> Value {
    let message_count = packet
        .events
        .iter()
        .filter(|event| event.get("eventType").and_then(Value::as_str) == Some("message.created"))
        .count();
    let titles = if packet.active_decisions.is_empty() {
        "none".to_owned()
    } else {
        packet
            .active_decisions
            .iter()
            .map(|decision| decision.title.as_str())
            .collect::<Vec<_>>()
            .join("; ")
    };
    let citations = packet
        .events
        .iter()
        .filter_map(|event| event.get("eventId").and_then(Value::as_str))
        .collect::<Vec<_>>();
    json!({
        "kind": "context-summary.v1",
        "summary": format!(
            "Room {} revision {} has {} messages and active decisions: {titles}.",
            packet.room_id, packet.context_revision, message_count
        ),
        "citations": citations,
    })
}

fn action_items_result(packet: &RoomContextPacket) -> Result<Value, PacketError> {
    let invocation = invocation_event(packet).ok_or(PacketError::MissingInvocation)?;
    let input = invocation
        .pointer("/payload/input")
        .and_then(Value::as_str)
        .ok_or(PacketError::InvocationMismatch)?;
    let action_items = input
        .lines()
        .filter_map(parse_action_item)
        .take(20)
        .collect::<Vec<_>>();
    let invocation_id = invocation
        .get("eventId")
        .and_then(Value::as_str)
        .ok_or(PacketError::MissingInvocation)?;
    Ok(json!({
        "kind": "action-items.v1",
        "actionItems": action_items,
        "citations": [invocation_id],
    }))
}

fn parse_action_item(line: &str) -> Option<Value> {
    let parts = line.strip_prefix("- ")?.split(" | ").collect::<Vec<_>>();
    let [text, owner, due] = parts.as_slice() else {
        return None;
    };
    let owner = owner.strip_prefix("owner: ")?;
    let due = due.strip_prefix("due: ")?;
    (literal_action_field(text, 2_000)
        && literal_action_field(owner, 256)
        && literal_action_field(due, 256))
    .then(|| json!({ "text": text, "owner": owner, "due": due }))
}

fn literal_action_field(value: &str, maximum: usize) -> bool {
    !value.is_empty() && value.len() <= maximum && value == value.trim()
}

fn skill(id: &str, name: &str, description: &str) -> AgentSkill {
    AgentSkill {
        id: id.to_owned(),
        name: name.to_owned(),
        description: description.to_owned(),
        tags: vec!["deterministic".to_owned(), "room".to_owned()],
        examples: None,
        input_modes: Some(vec!["application/json".to_owned()]),
        output_modes: Some(vec!["application/json".to_owned()]),
        security_requirements: None,
    }
}

fn invocation_event(packet: &RoomContextPacket) -> Option<&Value> {
    packet.events.iter().find(|event| {
        event.get("eventType").and_then(Value::as_str) == Some("agent.task.requested")
            && event
                .pointer("/payload/taskId")
                .and_then(Value::as_str)
                .and_then(|value| Uuid::parse_str(value).ok())
                == Some(packet.task_id)
    })
}

fn working_update(task_id: &str, context_id: &str, text: &str) -> StreamResponse {
    StreamResponse::StatusUpdate(TaskStatusUpdateEvent {
        task_id: task_id.to_owned(),
        context_id: context_id.to_owned(),
        status: TaskStatus {
            state: TaskState::Working,
            message: Some(Message {
                message_id: format!("{task_id}:{text}"),
                context_id: Some(context_id.to_owned()),
                task_id: Some(task_id.to_owned()),
                role: Role::Agent,
                parts: vec![Part::text(text)],
                metadata: None,
                extensions: None,
                reference_task_ids: None,
            }),
            timestamp: None,
        },
        metadata: None,
    })
}
