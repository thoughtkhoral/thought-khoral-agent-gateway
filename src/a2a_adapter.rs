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
        let (sender, mut receiver) = tokio::sync::mpsc::channel(1);
        let collect = async {
            let mut events = Vec::new();
            while let Some(event) = receiver.recv().await {
                events.push(event);
            }
            events
        };
        let (result, events) = tokio::join!(self.invoke_stream(packet, sender), collect);
        result?;
        Ok(events)
    }

    pub async fn invoke_stream(
        &self,
        packet: &RoomContextPacket,
        sender: tokio::sync::mpsc::Sender<A2aTaskEvent>,
    ) -> Result<(), A2aAdapterError> {
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
        let mut stream = transport
            .send_streaming_message(&params, &request)
            .await
            .map_err(|_| A2aAdapterError::Transport)?;
        let mut ordinal = 0;
        while let Some(event) = stream
            .try_next()
            .await
            .map_err(|_| A2aAdapterError::InvalidResponse)?
        {
            let event = Self::validate_event(packet, &event, ordinal)?;
            sender
                .send(event)
                .await
                .map_err(|_| A2aAdapterError::Transport)?;
            ordinal += 1;
        }
        if ordinal != 4 {
            return Err(A2aAdapterError::InvalidResponse);
        }
        Ok(())
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
        stream
            .iter()
            .enumerate()
            .map(|(ordinal, event)| Self::validate_event(packet, event, ordinal as u64))
            .collect()
    }

    fn validate_event(
        packet: &RoomContextPacket,
        event: &StreamResponse,
        ordinal: u64,
    ) -> Result<A2aTaskEvent, A2aAdapterError> {
        let task_id = packet.task_id.to_string();
        let context_id = format!("{}:{}", packet.room_id, packet.context_revision);
        match ordinal {
            0 => match event {
                StreamResponse::Task(task)
                    if task.id == task_id
                        && task.context_id == context_id
                        && task.status.state == TaskState::Submitted
                        && task.status.message.is_none()
                        && task.status.timestamp.is_none()
                        && task.artifacts.is_none()
                        && task.history.is_none()
                        && task.metadata.is_none() =>
                {
                    Ok(A2aTaskEvent::Submitted { ordinal: 0 })
                }
                _ => Err(A2aAdapterError::InvalidResponse),
            },
            1 => working_event(
                event,
                &task_id,
                &context_id,
                1,
                "Reading authorized room context",
            ),
            2 => working_event(event, &task_id, &context_id, 2, "Preparing cited result"),
            3 => completed_event(packet, event, &task_id, &context_id),
            _ => Err(A2aAdapterError::InvalidResponse),
        }
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
        .timeout(std::time::Duration::from_secs(10))
        .redirect(Policy::none())
        .build()
        .map_err(|_| A2aAdapterError::Transport)
}

fn working_event(
    event: &StreamResponse,
    task_id: &str,
    context_id: &str,
    ordinal: u64,
    expected_text: &str,
) -> Result<A2aTaskEvent, A2aAdapterError> {
    let StreamResponse::StatusUpdate(update) = event else {
        return Err(A2aAdapterError::InvalidResponse);
    };
    if update.task_id != task_id
        || update.context_id != context_id
        || update.status.state != TaskState::Working
        || update.status.timestamp.is_some()
        || update.metadata.is_some()
    {
        return Err(A2aAdapterError::InvalidResponse);
    }
    let message = update
        .status
        .message
        .as_ref()
        .ok_or(A2aAdapterError::InvalidResponse)?;
    if message.message_id != format!("{task_id}:{expected_text}")
        || message.context_id.as_deref() != Some(context_id)
        || message.task_id.as_deref() != Some(task_id)
        || message.role != Role::Agent
        || message.parts.len() != 1
        || message.metadata.is_some()
        || message.extensions.is_some()
        || message.reference_task_ids.is_some()
    {
        return Err(A2aAdapterError::InvalidResponse);
    }
    let part = &message.parts[0];
    let text = part
        .as_text()
        .filter(|text| *text == expected_text)
        .filter(|_| part.filename.is_none() && part.media_type.is_none() && part.metadata.is_none())
        .ok_or(A2aAdapterError::InvalidResponse)?;
    Ok(A2aTaskEvent::Working {
        ordinal,
        text: text.to_owned(),
    })
}

fn completed_event(
    packet: &RoomContextPacket,
    event: &StreamResponse,
    task_id: &str,
    context_id: &str,
) -> Result<A2aTaskEvent, A2aAdapterError> {
    let StreamResponse::Task(task) = event else {
        return Err(A2aAdapterError::InvalidResponse);
    };
    if task.id != task_id
        || task.context_id != context_id
        || task.status.state != TaskState::Completed
        || task.status.message.is_some()
        || task.status.timestamp.is_some()
        || task.history.is_some()
        || task.metadata.is_some()
    {
        return Err(A2aAdapterError::InvalidResponse);
    }
    let artifact = task
        .artifacts
        .as_ref()
        .filter(|artifacts| artifacts.len() == 1)
        .and_then(|artifacts| artifacts.first())
        .filter(|artifact| {
            artifact.artifact_id == format!("{task_id}:result")
                && artifact.name.as_deref() == Some("governed-room-result")
                && artifact.description.as_deref()
                    == Some("Deterministic result bound to the authorized context packet.")
                && artifact.parts.len() == 1
                && artifact.metadata.is_none()
                && artifact.extensions.is_none()
        })
        .ok_or(A2aAdapterError::InvalidResponse)?;
    let part = &artifact.parts[0];
    let PartContent::Data(data) = &part.content else {
        return Err(A2aAdapterError::InvalidResponse);
    };
    if part.filename.is_some()
        || part.media_type.is_some()
        || part.metadata.is_some()
        || data.as_object().is_none_or(|data| {
            data.len() != 2 || !data.contains_key("packet") || !data.contains_key("result")
        })
    {
        return Err(A2aAdapterError::InvalidResponse);
    }
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
