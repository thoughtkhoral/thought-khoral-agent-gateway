//! One-lease deterministic room-task dispatcher.

use std::{error::Error, time::Duration};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde_json::json;
use thiserror::Error;
use uuid::Uuid;

use crate::{
    ClaimRequest, ContextResponse, NormalizedAgentTaskUpdate, RoomContextPacket, RoomGatewayClient,
    TaskUpdateRequest, TaskUpdateResponse,
    a2a_adapter::{A2aAdapter, A2aTaskEvent},
    reference_agent::{PacketError, verify_packet},
    update_validation::validate_result,
};

const UPDATE_NAMESPACE: Uuid = Uuid::from_u128(0x74686f756768746b_686f72616c000005);

#[async_trait]
pub trait RoomTaskBroker: Send + Sync {
    type Error: Error + Send + Sync + 'static;

    async fn claim(&self, request: ClaimRequest) -> Result<Option<ContextResponse>, Self::Error>;
    async fn fetch_context(
        &self,
        task_id: Uuid,
        lease_token: Uuid,
    ) -> Result<ContextResponse, Self::Error>;
    async fn submit_update(
        &self,
        task_id: Uuid,
        lease_token: Uuid,
        request: TaskUpdateRequest,
    ) -> Result<TaskUpdateResponse, Self::Error>;
}

#[async_trait]
impl RoomTaskBroker for RoomGatewayClient {
    type Error = crate::RoomClientError;

    async fn claim(&self, request: ClaimRequest) -> Result<Option<ContextResponse>, Self::Error> {
        RoomGatewayClient::claim(self, request).await
    }

    async fn fetch_context(
        &self,
        task_id: Uuid,
        lease_token: Uuid,
    ) -> Result<ContextResponse, Self::Error> {
        RoomGatewayClient::fetch_context(self, task_id, lease_token).await
    }

    async fn submit_update(
        &self,
        task_id: Uuid,
        lease_token: Uuid,
        request: TaskUpdateRequest,
    ) -> Result<TaskUpdateResponse, Self::Error> {
        RoomGatewayClient::submit_update(self, task_id, lease_token, request).await
    }
}

#[async_trait]
pub trait A2aTaskRunner: Send + Sync {
    type Error: Error + Send + Sync + 'static;

    async fn invoke(&self, packet: &RoomContextPacket) -> Result<Vec<A2aTaskEvent>, Self::Error>;
}

#[async_trait]
impl A2aTaskRunner for A2aAdapter {
    type Error = crate::a2a_adapter::A2aAdapterError;

    async fn invoke(&self, packet: &RoomContextPacket) -> Result<Vec<A2aTaskEvent>, Self::Error> {
        A2aAdapter::invoke(self, packet).await
    }
}

pub struct Dispatcher<B, A> {
    broker: B,
    a2a: A,
    lease_owner: Uuid,
    coalesce_interval: Duration,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DispatchOutcome {
    pub submitted_updates: usize,
}

#[derive(Debug, Error)]
pub enum DispatchError {
    #[error("room task broker operation failed")]
    Broker,
    #[error("local A2A invocation failed")]
    A2a,
}

impl<B, A> Dispatcher<B, A>
where
    B: RoomTaskBroker,
    A: A2aTaskRunner,
{
    pub fn new(broker: B, a2a: A, lease_owner: Uuid, coalesce_interval: Duration) -> Self {
        Self {
            broker,
            a2a,
            lease_owner,
            coalesce_interval,
        }
    }

    /// Claims no more than one task. Every path after a claim either persists
    /// its verified progress and success or attempts a safe terminal failure.
    pub async fn run_once(&self) -> Result<DispatchOutcome, DispatchError> {
        let Some(claimed) = self
            .broker
            .claim(ClaimRequest {
                lease_owner: self.lease_owner,
            })
            .await
            .map_err(|_| DispatchError::Broker)?
        else {
            return Ok(DispatchOutcome {
                submitted_updates: 0,
            });
        };
        let lease_token = claimed.lease_token;
        let fallback_packet = claimed.packet.clone();
        let fetched = match self
            .broker
            .fetch_context(fallback_packet.task_id, lease_token)
            .await
        {
            Ok(fetched) => fetched,
            Err(_) => {
                return self
                    .submit_failure(&fallback_packet, lease_token, "execution_failed", 0)
                    .await;
            }
        };
        if fetched.lease_token != lease_token
            || fetched.packet.task_id != fallback_packet.task_id
            || fetched.packet.room_id != fallback_packet.room_id
            || fetched.packet.context_revision != fallback_packet.context_revision
            || fetched.packet.canonical_sha256 != fallback_packet.canonical_sha256
        {
            return self
                .submit_failure(&fallback_packet, lease_token, "execution_failed", 0)
                .await;
        }
        let packet = fetched.packet;
        if let Err(error) = verify_packet(&packet) {
            let code = match error {
                PacketError::Expired => "context_expired",
                _ => "execution_failed",
            };
            return self.submit_failure(&packet, lease_token, code, 0).await;
        }

        let events = match self.a2a.invoke(&packet).await {
            Ok(events) => events,
            Err(_) => {
                return self
                    .submit_failure(&packet, lease_token, "execution_failed", 0)
                    .await;
            }
        };
        if !validate_agent_events(&packet, &events) {
            return self
                .submit_failure(&packet, lease_token, "invalid_agent_response", 0)
                .await;
        }

        let mut submitted_updates = 0;
        let mut previous_progress: Option<(String, String, DateTime<Utc>)> = None;
        for event in events {
            match event {
                A2aTaskEvent::Submitted { ordinal } => {
                    self.submit_progress(
                        &packet,
                        lease_token,
                        ordinal,
                        "accepted",
                        "Task submitted",
                    )
                    .await?;
                    submitted_updates += 1;
                }
                A2aTaskEvent::Working { ordinal, text } => {
                    let occurred_at = update_occurred_at(&packet, ordinal);
                    let key = ("working".to_owned(), text.clone());
                    let repeated = previous_progress.as_ref().is_some_and(|previous| {
                        previous.0 == key.0
                            && previous.1 == key.1
                            && occurred_at
                                .signed_duration_since(previous.2)
                                .to_std()
                                .unwrap_or_default()
                                < self.coalesce_interval
                    });
                    if !repeated {
                        self.submit_progress(&packet, lease_token, ordinal, "working", &text)
                            .await?;
                        previous_progress = Some((key.0, key.1, occurred_at));
                        submitted_updates += 1;
                    }
                }
                A2aTaskEvent::Completed {
                    ordinal, result, ..
                } => {
                    self.submit_success(&packet, lease_token, ordinal, result)
                        .await?;
                    submitted_updates += 1;
                }
            }
        }
        Ok(DispatchOutcome { submitted_updates })
    }

    async fn submit_progress(
        &self,
        packet: &RoomContextPacket,
        lease_token: Uuid,
        ordinal: u64,
        phase: &str,
        text: &str,
    ) -> Result<(), DispatchError> {
        let update = NormalizedAgentTaskUpdate {
            event_type: "agent.task.progressed".to_owned(),
            payload: json!({ "phase": phase, "text": text }),
            occurred_at: update_occurred_at(packet, ordinal),
        };
        self.submit(
            packet,
            lease_token,
            ordinal,
            &format!("progressed:{phase}"),
            update,
        )
        .await
    }

    async fn submit_success(
        &self,
        packet: &RoomContextPacket,
        lease_token: Uuid,
        ordinal: u64,
        result: serde_json::Value,
    ) -> Result<(), DispatchError> {
        let update = NormalizedAgentTaskUpdate {
            event_type: "agent.task.succeeded".to_owned(),
            payload: json!({ "result": result }),
            occurred_at: update_occurred_at(packet, ordinal),
        };
        self.submit(packet, lease_token, ordinal, "succeeded", update)
            .await
    }

    async fn submit_failure(
        &self,
        packet: &RoomContextPacket,
        lease_token: Uuid,
        code: &'static str,
        ordinal: u64,
    ) -> Result<DispatchOutcome, DispatchError> {
        let update = NormalizedAgentTaskUpdate {
            event_type: "agent.task.failed".to_owned(),
            payload: json!({ "failure": { "code": code } }),
            occurred_at: update_occurred_at(packet, ordinal),
        };
        self.submit(
            packet,
            lease_token,
            ordinal,
            &format!("failed:{code}"),
            update,
        )
        .await?;
        Ok(DispatchOutcome {
            submitted_updates: 1,
        })
    }

    async fn submit(
        &self,
        packet: &RoomContextPacket,
        lease_token: Uuid,
        ordinal: u64,
        kind: &str,
        update: NormalizedAgentTaskUpdate,
    ) -> Result<(), DispatchError> {
        self.broker
            .submit_update(
                packet.task_id,
                lease_token,
                TaskUpdateRequest {
                    update_id: deterministic_update_id(packet.task_id, ordinal, kind),
                    context_revision: packet.context_revision,
                    update,
                },
            )
            .await
            .map(|_| ())
            .map_err(|_| DispatchError::Broker)
    }
}

pub fn deterministic_update_id(task_id: Uuid, ordinal: u64, kind: &str) -> Uuid {
    Uuid::new_v5(
        &UPDATE_NAMESPACE,
        format!("{task_id}:{ordinal}:{kind}").as_bytes(),
    )
}

fn update_occurred_at(packet: &RoomContextPacket, ordinal: u64) -> DateTime<Utc> {
    let milliseconds = ordinal.min(i64::MAX as u64) as i64;
    packet
        .issued_at
        .checked_add_signed(chrono::Duration::milliseconds(milliseconds))
        .unwrap_or(packet.issued_at)
}

fn validate_agent_events(packet: &RoomContextPacket, events: &[A2aTaskEvent]) -> bool {
    let [
        A2aTaskEvent::Submitted { ordinal: 0 },
        A2aTaskEvent::Working {
            ordinal: 1,
            text: first,
        },
        A2aTaskEvent::Working {
            ordinal: 2,
            text: second,
        },
        A2aTaskEvent::Completed {
            ordinal: 3,
            result,
            binding,
        },
    ] = events
    else {
        return false;
    };
    first == "Reading authorized room context"
        && second == "Preparing cited result"
        && binding.task_id == packet.task_id
        && binding.room_id == packet.room_id
        && binding.context_revision == packet.context_revision
        && binding.canonical_sha256 == packet.canonical_sha256
        && validate_result(packet, result).is_ok()
}
