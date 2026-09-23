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
    reference_agent::verify_packet,
    update_validation::validate_result,
};

const UPDATE_NAMESPACE: Uuid = Uuid::from_u128(0x74686f756768746b_686f72616c000005);

#[async_trait]
pub trait RoomTaskBroker: Send + Sync {
    type Error: Error + Send + Sync + 'static;
    fn retryable(&self, _error: &Self::Error) -> bool {
        true
    }

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
    fn retryable(&self, error: &Self::Error) -> bool {
        error.is_retryable()
    }

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
    async fn invoke_stream(
        &self,
        packet: &RoomContextPacket,
        sender: tokio::sync::mpsc::Sender<A2aTaskEvent>,
    ) -> Result<(), Self::Error> {
        for event in self.invoke(packet).await? {
            if sender.send(event).await.is_err() {
                break;
            }
        }
        Ok(())
    }
}

#[async_trait]
impl A2aTaskRunner for A2aAdapter {
    type Error = crate::a2a_adapter::A2aAdapterError;

    async fn invoke(&self, packet: &RoomContextPacket) -> Result<Vec<A2aTaskEvent>, Self::Error> {
        A2aAdapter::invoke(self, packet).await
    }
    async fn invoke_stream(
        &self,
        packet: &RoomContextPacket,
        sender: tokio::sync::mpsc::Sender<A2aTaskEvent>,
    ) -> Result<(), Self::Error> {
        A2aAdapter::invoke_stream(self, packet, sender).await
    }
}

pub struct Dispatcher<B, A> {
    broker: B,
    a2a: A,
    lease_owner: Uuid,
    retry_interval: Duration,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DispatchOutcome {
    pub submitted_updates: usize,
}

#[derive(Debug, Error)]
pub enum DispatchError {
    #[error("room task broker operation failed")]
    Broker,
    #[error("room task broker rejected the update permanently")]
    Rejected,
    #[error("local A2A invocation failed")]
    A2a,
}

#[derive(Clone, Copy)]
enum TerminalFailure {
    ContextRefetchFailed,
    ContextBindingMismatch,
    InvalidContextPacket,
    LocalA2aInvocation,
    A2aResponseAdmission,
}

impl TerminalFailure {
    const fn as_str(self) -> &'static str {
        match self {
            Self::InvalidContextPacket => "invalid_task_input",
            Self::ContextRefetchFailed
            | Self::ContextBindingMismatch
            | Self::LocalA2aInvocation
            | Self::A2aResponseAdmission => "execution_failed",
        }
    }
}

impl<B, A> Dispatcher<B, A>
where
    B: RoomTaskBroker,
    A: A2aTaskRunner,
{
    /// `retry_interval` controls replay spacing after a submission response is
    /// lost. It deliberately does not coalesce protocol events: the fixed A2A
    /// stream has four distinct, ordered updates that must all reach the room.
    pub fn new(broker: B, a2a: A, lease_owner: Uuid, retry_interval: Duration) -> Self {
        Self {
            broker,
            a2a,
            lease_owner,
            retry_interval,
        }
    }

    /// Keep polling through transient broker outages without restarting the
    /// process. Permanent rejections still surface to the container runtime.
    pub async fn run_forever(&self, poll_interval: Duration) -> Result<(), DispatchError> {
        let base_delay = poll_interval.max(Duration::from_millis(250));
        let mut retry_delay = base_delay;
        loop {
            match self.run_once().await {
                Ok(_) => {
                    retry_delay = base_delay;
                    tokio::time::sleep(poll_interval).await;
                }
                Err(DispatchError::Broker) => {
                    eprintln!("room task broker temporarily unavailable; retrying");
                    let jitter = Duration::from_millis((Uuid::new_v4().as_u128() % 250) as u64);
                    tokio::time::sleep(jittered_retry_delay(retry_delay, jitter)).await;
                    retry_delay = retry_delay.saturating_mul(2).min(Duration::from_secs(8));
                }
                Err(error) => return Err(error),
            }
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
            .map_err(|error| {
                if self.broker.retryable(&error) {
                    DispatchError::Broker
                } else {
                    DispatchError::Rejected
                }
            })?
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
                    .submit_failure(
                        &fallback_packet,
                        lease_token,
                        TerminalFailure::ContextRefetchFailed,
                        0,
                    )
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
                .submit_failure(
                    &fallback_packet,
                    lease_token,
                    TerminalFailure::ContextBindingMismatch,
                    0,
                )
                .await;
        }
        let packet = fetched.packet;
        if verify_packet(&packet).is_err() {
            return self
                .submit_failure(
                    &packet,
                    lease_token,
                    TerminalFailure::InvalidContextPacket,
                    0,
                )
                .await;
        }

        let remaining = (packet.expires_at - Utc::now() - chrono::Duration::seconds(2))
            .to_std()
            .unwrap_or(Duration::ZERO)
            .min(Duration::from_secs(10));
        let (sender, mut receiver) = tokio::sync::mpsc::channel(1);
        let invocation = tokio::time::timeout(remaining, self.a2a.invoke_stream(&packet, sender));
        tokio::pin!(invocation);
        let mut invocation_done = false;
        let mut received = 0;
        let mut terminal = None;
        let mut submitted_updates = 0;
        loop {
            let event = tokio::select! {
                result = &mut invocation, if !invocation_done => {
                    if !matches!(result, Ok(Ok(()))) {
                        eprintln!("A2A invocation failed for task {} after {received} events: {result:?}", packet.task_id);
                        return self.submit_failure(&packet, lease_token, TerminalFailure::LocalA2aInvocation, received).await;
                    }
                    invocation_done = true;
                    continue;
                }
                event = receiver.recv() => match event {
                    Some(event) => event,
                    None => break,
                }
            };
            if !validate_agent_event(&packet, &event, received) {
                eprintln!(
                    "dispatcher rejected A2A event for task {} at ordinal {received}",
                    packet.task_id
                );
                return self
                    .submit_failure(
                        &packet,
                        lease_token,
                        TerminalFailure::A2aResponseAdmission,
                        received,
                    )
                    .await;
            }
            received += 1;
            let result = match event {
                A2aTaskEvent::Submitted { ordinal } => {
                    self.submit_progress(
                        &packet,
                        lease_token,
                        ordinal,
                        "accepted",
                        "Task submitted",
                    )
                    .await
                }
                A2aTaskEvent::Working { ordinal, text } => {
                    self.submit_progress(&packet, lease_token, ordinal, "working", &text)
                        .await
                }
                A2aTaskEvent::Completed {
                    ordinal, result, ..
                } => {
                    terminal = Some((ordinal, result));
                    continue;
                }
            };
            if result.is_err() {
                eprintln!(
                    "broker rejected progress for task {} at ordinal {}: {result:?}",
                    packet.task_id,
                    received - 1
                );
                return self
                    .submit_failure(
                        &packet,
                        lease_token,
                        TerminalFailure::A2aResponseAdmission,
                        received,
                    )
                    .await;
            }
            submitted_updates += 1;
        }
        if !invocation_done && !matches!(invocation.await, Ok(Ok(()))) {
            return self
                .submit_failure(
                    &packet,
                    lease_token,
                    TerminalFailure::LocalA2aInvocation,
                    received,
                )
                .await;
        }
        let Some((ordinal, result)) = terminal.filter(|_| received == 4) else {
            return self
                .submit_failure(
                    &packet,
                    lease_token,
                    TerminalFailure::A2aResponseAdmission,
                    received,
                )
                .await;
        };
        if self
            .submit_success(&packet, lease_token, ordinal, result)
            .await
            .is_err()
        {
            return self
                .submit_failure(
                    &packet,
                    lease_token,
                    TerminalFailure::A2aResponseAdmission,
                    received,
                )
                .await;
        }
        submitted_updates += 1;
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
        failure: TerminalFailure,
        ordinal: u64,
    ) -> Result<DispatchOutcome, DispatchError> {
        let update = NormalizedAgentTaskUpdate {
            event_type: "agent.task.failed".to_owned(),
            payload: json!({ "failure": { "code": failure.as_str() } }),
            occurred_at: update_occurred_at(packet, ordinal),
        };
        self.submit(
            packet,
            lease_token,
            ordinal,
            &format!("failed:{}", failure.as_str()),
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
        let request = TaskUpdateRequest {
            update_id: deterministic_update_id(packet.task_id, ordinal, kind),
            context_revision: packet.context_revision,
            update,
        };
        loop {
            if Utc::now() >= packet.expires_at {
                return Err(DispatchError::Broker);
            }
            let remaining = (packet.expires_at - Utc::now())
                .to_std()
                .unwrap_or(Duration::ZERO);
            match tokio::time::timeout(
                remaining.min(Duration::from_secs(5)),
                self.broker
                    .submit_update(packet.task_id, lease_token, request.clone()),
            )
            .await
            {
                Ok(Ok(_)) => return Ok(()),
                Ok(Err(error)) if !self.broker.retryable(&error) => {
                    return Err(DispatchError::Rejected);
                }
                _ => {}
            }

            if self.retry_interval.is_zero() {
                tokio::task::yield_now().await;
            } else {
                tokio::time::sleep(self.retry_interval).await;
            }
        }
    }
}

fn jittered_retry_delay(base: Duration, jitter: Duration) -> Duration {
    base.saturating_add(jitter).min(Duration::from_secs(8))
}

pub fn deterministic_update_id(task_id: Uuid, ordinal: u64, kind: &str) -> Uuid {
    Uuid::new_v5(
        &UPDATE_NAMESPACE,
        format!("{task_id}:{ordinal}:{kind}").as_bytes(),
    )
}

fn update_occurred_at(packet: &RoomContextPacket, ordinal: u64) -> DateTime<Utc> {
    let milliseconds = ordinal.min(i64::MAX as u64) as i64;
    // Invocation time survives lease renewal; packet issue time does not.
    let invoked_at = packet
        .events
        .iter()
        .find(|event| {
            event["eventType"] == "agent.task.requested"
                && event["payload"]["taskId"] == json!(packet.task_id)
        })
        .and_then(|event| event["occurredAt"].as_str())
        .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
        .map(|time| time.with_timezone(&Utc))
        .unwrap_or(packet.issued_at);
    invoked_at
        .checked_add_signed(chrono::Duration::milliseconds(milliseconds))
        .unwrap_or(invoked_at)
}

fn validate_agent_event(packet: &RoomContextPacket, event: &A2aTaskEvent, ordinal: u64) -> bool {
    if event.ordinal() != ordinal {
        return false;
    }
    match event {
        A2aTaskEvent::Submitted { ordinal: 0 } => true,
        A2aTaskEvent::Working { ordinal: 1, text } => text == "Reading authorized room context",
        A2aTaskEvent::Working { ordinal: 2, text } => text == "Preparing cited result",
        A2aTaskEvent::Completed {
            ordinal: 3,
            result,
            binding,
        } => {
            binding.task_id == packet.task_id
                && binding.room_id == packet.room_id
                && binding.context_revision == packet.context_revision
                && binding.canonical_sha256 == packet.canonical_sha256
                && validate_result(packet, result).is_ok()
        }
        _ => false,
    }
}

#[cfg(test)]
mod retry_delay_tests {
    use super::*;

    #[test]
    fn jitter_never_exceeds_the_retry_cap() {
        assert_eq!(
            jittered_retry_delay(Duration::from_secs(8), Duration::from_millis(249)),
            Duration::from_secs(8)
        );
        assert_eq!(
            jittered_retry_delay(Duration::from_millis(250), Duration::from_millis(249)),
            Duration::from_millis(499)
        );
    }
}
