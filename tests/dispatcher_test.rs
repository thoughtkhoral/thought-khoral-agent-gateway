use std::{collections::BTreeSet, convert::Infallible, sync::Arc, time::Duration as StdDuration};

use async_trait::async_trait;
use chrono::{Duration, Utc};
use serde_json::Value;
use thought_khoral_agent_gateway::{
    ClaimRequest, ContextResponse, RoomContextPacket, TaskUpdateRequest, TaskUpdateResponse,
    a2a_adapter::A2aTaskEvent,
    dispatcher::{A2aTaskRunner, Dispatcher, RoomTaskBroker},
    reference_agent::stream_for_packet,
    update_validation::{Handoff, validate_handoff},
};
use tokio::sync::Mutex;
use uuid::Uuid;

fn packet() -> RoomContextPacket {
    serde_json::from_str(include_str!("../fixtures/context-packet.json"))
        .expect("fixture must be a room context packet")
}

// This catches non-idempotent update identifiers, skipping the required
// context reread, changed event ordering, and duplicate meaningful progress.
#[tokio::test]
async fn dispatcher_claims_fetches_and_emits_one_ordered_normalized_terminal_stream() {
    let packet = packet();
    let broker = RecordingBroker::new(packet.clone());
    let runner = FixedRunner::new(
        A2aTaskEvent::from_stream(&packet, stream_for_packet(&packet).unwrap()).unwrap(),
    );
    let dispatcher = Dispatcher::new(
        broker.clone(),
        runner,
        Uuid::from_u128(0x71000000000040008000000000000001),
        StdDuration::from_secs(1),
    );

    let outcome = dispatcher
        .run_once()
        .await
        .expect("valid task must dispatch");
    assert_eq!(outcome.submitted_updates, 4);
    assert_eq!(broker.fetches().await, 1);
    let updates = broker.updates().await;
    assert_eq!(
        updates
            .iter()
            .map(|update| update.update.event_type.as_str())
            .collect::<Vec<_>>(),
        vec![
            "agent.task.progressed",
            "agent.task.progressed",
            "agent.task.progressed",
            "agent.task.succeeded"
        ]
    );
    assert_eq!(
        updates[1].update.payload["text"],
        "Reading authorized room context"
    );
    assert_eq!(updates[2].update.payload["text"], "Preparing cited result");
    assert!(
        updates
            .windows(2)
            .all(|pair| pair[0].update_id != pair[1].update_id)
    );
    assert_eq!(
        updates[3].update.payload["result"]["citations"],
        serde_json::json!([
            "88000000-0000-4000-8000-000000000001",
            "88000000-0000-4000-8000-000000000002",
            "88000000-0000-4000-8000-000000000003"
        ])
    );
}

// This catches forwarding a malformed agent result to the room gateway. A
// leased task must receive only the contract-safe terminal failure.
#[tokio::test]
async fn invalid_agent_response_creates_only_a_safe_terminal_failure() {
    let packet = packet();
    let broker = RecordingBroker::new(packet.clone());
    let mut events =
        A2aTaskEvent::from_stream(&packet, stream_for_packet(&packet).unwrap()).unwrap();
    events[3].set_result(serde_json::json!({
        "kind": "context-summary.v1",
        "summary": "forged",
        "citations": ["99000000-0000-4000-8000-000000000001"]
    }));
    let dispatcher = Dispatcher::new(
        broker.clone(),
        FixedRunner::new(events),
        Uuid::from_u128(0x71000000000040008000000000000002),
        StdDuration::from_secs(1),
    );

    dispatcher
        .run_once()
        .await
        .expect("safe failure must be submitted");
    let updates = broker.updates().await;
    assert_eq!(updates.len(), 1);
    assert_eq!(updates[0].update.event_type, "agent.task.failed");
    assert_eq!(
        updates[0].update.payload["failure"]["code"],
        "invalid_agent_response"
    );
}

// The room gateway compares duplicate update timestamps as well as identifiers
// and payloads. This catches a retry that derives a stable UUID but calls the
// wall clock again before submitting the same normalized event.
#[tokio::test]
async fn dispatch_retries_use_byte_for_byte_idempotent_normalized_updates() {
    let packet = packet();
    let broker = RecordingBroker::new(packet.clone());
    let runner = FixedRunner::new(
        A2aTaskEvent::from_stream(&packet, stream_for_packet(&packet).unwrap()).unwrap(),
    );
    let dispatcher = Dispatcher::new(
        broker.clone(),
        runner,
        Uuid::from_u128(0x71000000000040008000000000000003),
        StdDuration::from_secs(1),
    );

    dispatcher.run_once().await.unwrap();
    tokio::time::sleep(StdDuration::from_millis(2)).await;
    dispatcher.run_once().await.unwrap();

    let updates = broker.updates().await;
    assert_eq!(updates.len(), 8);
    for (first, retry) in updates[..4].iter().zip(&updates[4..]) {
        assert_eq!(first.update_id, retry.update_id);
        assert_eq!(first.context_revision, retry.context_revision);
        assert_eq!(first.update.event_type, retry.update.event_type);
        assert_eq!(first.update.payload, retry.update.payload);
        assert_eq!(first.update.occurred_at, retry.update.occurred_at);
    }
}

// This catches accepting a handoff which can redirect the worker, has expired,
// or does not bind exactly to the already-authorized packet.
#[test]
fn handoff_is_data_only_and_must_be_bound_unexpired_https_and_allowlisted() {
    let packet = packet();
    let hosts = BTreeSet::from(["approved.example".to_owned()]);
    let handoff = Handoff {
        task_id: packet.task_id,
        context_revision: packet.context_revision,
        instruction: "Complete the human confirmation.".to_owned(),
        url: "https://approved.example/continue?token=opaque".to_owned(),
        host: "approved.example".to_owned(),
        expires_at: Utc::now() + Duration::minutes(1),
    };
    validate_handoff(&packet, &hosts, &handoff, Utc::now()).expect("valid data-only handoff");

    let mut wrong_binding = handoff.clone();
    wrong_binding.task_id = Uuid::new_v4();
    assert!(validate_handoff(&packet, &hosts, &wrong_binding, Utc::now()).is_err());
    let mut insecure_url = handoff.clone();
    insecure_url.url = "http://approved.example/continue".to_owned();
    assert!(validate_handoff(&packet, &hosts, &insecure_url, Utc::now()).is_err());
    let mut expired = handoff;
    expired.expires_at = Utc::now() - Duration::seconds(1);
    assert!(validate_handoff(&packet, &hosts, &expired, Utc::now()).is_err());
}

#[derive(Clone)]
struct RecordingBroker {
    context: ContextResponse,
    fetch_count: Arc<Mutex<usize>>,
    updates: Arc<Mutex<Vec<TaskUpdateRequest>>>,
}

impl RecordingBroker {
    fn new(packet: RoomContextPacket) -> Self {
        Self {
            context: ContextResponse {
                lease_token: Uuid::from_u128(0x72000000000040008000000000000001),
                packet,
            },
            fetch_count: Arc::new(Mutex::new(0)),
            updates: Arc::new(Mutex::new(Vec::new())),
        }
    }

    async fn fetches(&self) -> usize {
        *self.fetch_count.lock().await
    }

    async fn updates(&self) -> Vec<TaskUpdateRequest> {
        self.updates.lock().await.clone()
    }
}

#[async_trait]
impl RoomTaskBroker for RecordingBroker {
    type Error = Infallible;

    async fn claim(&self, _request: ClaimRequest) -> Result<Option<ContextResponse>, Self::Error> {
        Ok(Some(self.context.clone()))
    }

    async fn fetch_context(
        &self,
        _task_id: Uuid,
        _lease_token: Uuid,
    ) -> Result<ContextResponse, Self::Error> {
        *self.fetch_count.lock().await += 1;
        Ok(self.context.clone())
    }

    async fn submit_update(
        &self,
        _task_id: Uuid,
        _lease_token: Uuid,
        request: TaskUpdateRequest,
    ) -> Result<TaskUpdateResponse, Self::Error> {
        self.updates.lock().await.push(request);
        Ok(TaskUpdateResponse {
            events: Vec::<Value>::new(),
        })
    }
}

struct FixedRunner {
    events: Vec<A2aTaskEvent>,
}

impl FixedRunner {
    fn new(events: Vec<A2aTaskEvent>) -> Self {
        Self { events }
    }
}

#[async_trait]
impl A2aTaskRunner for FixedRunner {
    type Error = Infallible;

    async fn invoke(&self, _packet: &RoomContextPacket) -> Result<Vec<A2aTaskEvent>, Self::Error> {
        Ok(self.events.clone())
    }
}
