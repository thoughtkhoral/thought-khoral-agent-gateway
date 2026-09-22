//! Fixed-loopback A2A invocation and strict response admission.

use std::collections::HashMap;

use a2a::{
    AgentCard, Message, Part, PartContent, Role, SendMessageRequest, TaskState,
    event::StreamResponse,
};
use a2a_client::{Transport, jsonrpc::JsonRpcTransport};
use futures::TryStreamExt;
use reqwest::redirect::Policy;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use thiserror::Error;
use uuid::Uuid;

use crate::{
    RegisteredAgent, RoomContextPacket,
    reference_agent::{REFERENCE_AGENT_CARD, REFERENCE_AGENT_ENDPOINT, verify_packet},
    reference_registration,
    update_validation::validate_result,
};

#[derive(Clone, Debug, PartialEq)]
pub enum A2aTaskEvent {
    Submitted {
        ordinal: u64,
    },
    Working {
        ordinal: u64,
        text: String,
    },
    Completed {
        ordinal: u64,
        result: Value,
        binding: PacketBinding,
    },
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PacketBinding {
    pub task_id: Uuid,
    pub room_id: Uuid,
    pub context_revision: i64,
    pub canonical_sha256: String,
}

impl A2aTaskEvent {
    pub fn from_stream(
        packet: &RoomContextPacket,
        stream: Vec<StreamResponse>,
    ) -> Result<Vec<Self>, A2aAdapterError> {
        A2aAdapter::validate_stream(packet, stream)
    }

    pub fn ordinal(&self) -> u64 {
        match self {
            Self::Submitted { ordinal }
            | Self::Working { ordinal, .. }
            | Self::Completed { ordinal, .. } => *ordinal,
        }
    }

    pub fn progress_text(&self) -> Option<&str> {
        match self {
            Self::Working { text, .. } => Some(text),
            Self::Submitted { .. } | Self::Completed { .. } => None,
        }
    }

    pub fn set_result(&mut self, result: Value) {
        if let Self::Completed {
            result: current, ..
        } = self
        {
            *current = result;
        }
    }
}

pub struct A2aAdapter {
    bearer_secret: String,
}

impl A2aAdapter {
    pub fn new(bearer_secret: impl Into<String>) -> Result<Self, A2aAdapterError> {
        let bearer_secret = bearer_secret.into();
        if bearer_secret.is_empty() {
            return Err(A2aAdapterError::EmptyBearerSecret);
        }
        Ok(Self { bearer_secret })
    }

    /// Authenticated local discovery is revalidated against Task 4's immutable
    /// registration before the worker begins polling.
    pub async fn resolve_pinned_card(&self) -> Result<RegisteredAgent, A2aAdapterError> {
        let card = local_http_client()?
            .get(REFERENCE_AGENT_CARD)
            .bearer_auth(&self.bearer_secret)
            .send()
            .await
            .map_err(|_| A2aAdapterError::Transport)?
            .error_for_status()
            .map_err(|_| A2aAdapterError::Transport)?
            .json::<AgentCard>()
            .await
            .map_err(|_| A2aAdapterError::InvalidResponse)?;
        RegisteredAgent::from_pinned_card(reference_registration(), card)
            .map_err(|_| A2aAdapterError::InvalidResponse)
    }

    pub async fn invoke(
        &self,
        packet: &RoomContextPacket,
    ) -> Result<Vec<A2aTaskEvent>, A2aAdapterError> {
        verify_packet(packet).map_err(|_| A2aAdapterError::InvalidPacket)?;
        let mut message = Message::new(
            Role::User,
            vec![
                Part::text(
                    serde_json::to_string(&json!({ "packet": packet }))
                        .map_err(|_| A2aAdapterError::InvalidPacket)?,
                )
                .with_media_type("application/json"),
            ],
        );
        // The generated message ID is irrelevant to results, while these two
        // fields make the request itself bound to the packet task/revision.
        message.task_id = Some(packet.task_id.to_string());
        message.context_id = Some(format!("{}:{}", packet.room_id, packet.context_revision));
        let request = SendMessageRequest {
            message,
            configuration: None,
            metadata: None,
            tenant: None,
        };
        let mut params = HashMap::new();
        params.insert(
            "authorization".to_owned(),
            vec![format!("Bearer {}", self.bearer_secret)],
        );
        params.insert("A2A-Version".to_owned(), vec![a2a::VERSION.to_owned()]);
        let transport =
            JsonRpcTransport::new(local_http_client()?, REFERENCE_AGENT_ENDPOINT.to_owned());
        let stream = transport
            .send_streaming_message(&params, &request)
            .await
            .map_err(|_| A2aAdapterError::Transport)?;
        let events = stream
            .try_collect::<Vec<_>>()
            .await
            .map_err(|_| A2aAdapterError::InvalidResponse)?;
        Self::validate_stream(packet, events)
    }

    /// Accepts exactly the four events emitted by the local Reference Agent.
    /// Each terminal artifact carries a full packet binding, preventing a
    /// response for another task/revision/hash from reaching the room gateway.
    pub fn validate_stream(
        packet: &RoomContextPacket,
        stream: Vec<StreamResponse>,
    ) -> Result<Vec<A2aTaskEvent>, A2aAdapterError> {
        verify_packet(packet).map_err(|_| A2aAdapterError::InvalidPacket)?;
        if stream.len() != 4 {
            return Err(A2aAdapterError::InvalidResponse);
        }
        let task_id = packet.task_id.to_string();
        let submitted = match &stream[0] {
            StreamResponse::Task(task)
                if task.id == task_id
                    && task.status.state == TaskState::Submitted
                    && task.artifacts.is_none() =>
            {
                A2aTaskEvent::Submitted { ordinal: 0 }
            }
            _ => return Err(A2aAdapterError::InvalidResponse),
        };
        let first_working =
            working_event(&stream[1], &task_id, 1, "Reading authorized room context")?;
        let second_working = working_event(&stream[2], &task_id, 2, "Preparing cited result")?;
        let completed = completed_event(packet, &stream[3], &task_id)?;
        Ok(vec![submitted, first_working, second_working, completed])
    }
}

#[derive(Debug, Error)]
pub enum A2aAdapterError {
    #[error("agent-gateway bearer secret must not be empty")]
    EmptyBearerSecret,
    #[error("only a hash-verified unexpired context packet may be invoked")]
    InvalidPacket,
    #[error("local A2A response differs from the pinned deterministic protocol")]
    InvalidResponse,
    #[error("local A2A transport failed")]
    Transport,
}

fn local_http_client() -> Result<reqwest::Client, A2aAdapterError> {
    reqwest::Client::builder()
        .no_proxy()
        .redirect(Policy::none())
        .build()
        .map_err(|_| A2aAdapterError::Transport)
}

fn working_event(
    event: &StreamResponse,
    task_id: &str,
    ordinal: u64,
    expected_text: &str,
) -> Result<A2aTaskEvent, A2aAdapterError> {
    let StreamResponse::StatusUpdate(update) = event else {
        return Err(A2aAdapterError::InvalidResponse);
    };
    let text = update
        .status
        .message
        .as_ref()
        .and_then(|message| message.parts.first())
        .and_then(Part::as_text)
        .filter(|text| *text == expected_text)
        .ok_or(A2aAdapterError::InvalidResponse)?;
    if update.task_id != task_id || update.status.state != TaskState::Working {
        return Err(A2aAdapterError::InvalidResponse);
    }
    Ok(A2aTaskEvent::Working {
        ordinal,
        text: text.to_owned(),
    })
}

fn completed_event(
    packet: &RoomContextPacket,
    event: &StreamResponse,
    task_id: &str,
) -> Result<A2aTaskEvent, A2aAdapterError> {
    let StreamResponse::Task(task) = event else {
        return Err(A2aAdapterError::InvalidResponse);
    };
    if task.id != task_id || task.status.state != TaskState::Completed {
        return Err(A2aAdapterError::InvalidResponse);
    }
    let artifact = task
        .artifacts
        .as_ref()
        .filter(|artifacts| artifacts.len() == 1)
        .and_then(|artifacts| artifacts.first())
        .filter(|artifact| artifact.parts.len() == 1)
        .ok_or(A2aAdapterError::InvalidResponse)?;
    let PartContent::Data(data) = &artifact.parts[0].content else {
        return Err(A2aAdapterError::InvalidResponse);
    };
    let binding = serde_json::from_value::<PacketBinding>(
        data.get("packet")
            .cloned()
            .ok_or(A2aAdapterError::InvalidResponse)?,
    )
    .map_err(|_| A2aAdapterError::InvalidResponse)?;
    if binding.task_id != packet.task_id
        || binding.room_id != packet.room_id
        || binding.context_revision != packet.context_revision
        || binding.canonical_sha256 != packet.canonical_sha256
    {
        return Err(A2aAdapterError::InvalidResponse);
    }
    let result = data
        .get("result")
        .cloned()
        .ok_or(A2aAdapterError::InvalidResponse)?;
    validate_result(packet, &result).map_err(|_| A2aAdapterError::InvalidResponse)?;
    Ok(A2aTaskEvent::Completed {
        ordinal: 3,
        result,
        binding,
    })
}
