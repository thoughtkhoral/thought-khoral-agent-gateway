use std::{
    collections::{BTreeSet, VecDeque},
    error::Error,
    fmt,
    sync::Arc,
    time::Duration as StdDuration,
};

use async_trait::async_trait;
use chrono::{Duration, Utc};
use serde_json::Value;
use thought_khoral_agent_gateway::{
    ClaimRequest, ContextResponse, RoomContextPacket, TaskUpdateRequest, TaskUpdateResponse,
    a2a_adapter::A2aTaskEvent,
    dispatcher::{A2aTaskRunner, Dispatcher, RoomTaskBroker},
    reference_agent::{canonical_packet_sha256, stream_for_packet},
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
        StdDuration::ZERO,
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
        StdDuration::ZERO,
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
        "execution_failed"
    );
}

// A malformed packet is a caller-visible invalid input, while
// every other terminal failure remains inside the published contract enum.
#[tokio::test]
async fn invalid_packet_failure_uses_the_published_invalid_task_input_code() {
    let mut packet = packet();
    packet.issued_at = Utc::now() + Duration::minutes(2);
    packet.expires_at = Utc::now() + Duration::minutes(1);
    packet.canonical_sha256 = canonical_packet_sha256(&packet).unwrap();
    let broker = RecordingBroker::new(packet.clone());
    let runner = FixedRunner::new(Vec::new());
    let dispatcher = Dispatcher::new(
        broker.clone(),
        runner,
        Uuid::from_u128(0x71000000000040008000000000000003),
        StdDuration::ZERO,
    );

    dispatcher.run_once().await.unwrap();
    let updates = broker.updates().await;
    assert_eq!(updates.len(), 1);
    assert_eq!(updates[0].update.event_type, "agent.task.failed");
    assert_eq!(
        updates[0].update.payload["failure"]["code"],
        "invalid_task_input"
    );
}

// The real room contract restricts external failure codes to this exact enum.
// This fixture-level assertion binds the worker regression tests to the
// published schema rather than to a locally invented error vocabulary.
#[test]
fn dispatched_failure_codes_are_accepted_by_the_published_room_contract() {
    let schema: Value = serde_json::from_str(include_str!(
        "../../thought-khoral-contracts/schemas/room-event.schema.json"
    ))
    .expect("published room contract must be valid JSON");
    assert_eq!(
        schema.pointer("/$defs/externalTaskFailedPayload/properties/failure/properties/code/enum"),
        Some(&serde_json::json!([
            "invalid_task_input",
            "execution_failed"
        ]))
    );
}

// A transport may persist an update and lose its response. The worker must
// replay the identical update ID while its lease is live so the gateway's
// duplicate acceptance can finish the task rather than stranding it.
#[tokio::test]
async fn dispatcher_retries_dropped_progress_and_success_responses_with_the_same_update_ids() {
    let packet = packet();
    let broker = RecordingBroker::with_lost_responses(
        packet.clone(),
        ["agent.task.progressed", "agent.task.succeeded"],
    );
    let runner = FixedRunner::new(
        A2aTaskEvent::from_stream(&packet, stream_for_packet(&packet).unwrap()).unwrap(),
    );
    let dispatcher = Dispatcher::new(
        broker.clone(),
        runner,
        Uuid::from_u128(0x71000000000040008000000000000004),
        StdDuration::ZERO,
    );

    let outcome = dispatcher.run_once().await.unwrap();
    assert_eq!(outcome.submitted_updates, 4);
    assert_eq!(broker.fetches().await, 1);
    let accepted = broker.updates().await;
    assert_eq!(accepted.len(), 4);
    assert_eq!(
        accepted.last().unwrap().update.event_type,
        "agent.task.succeeded"
    );

    let attempts = broker.attempts().await;
    assert_eq!(attempts.len(), 6);
    assert_same_normalized_update(&attempts[0], &attempts[1]);
    assert_same_normalized_update(&attempts[4], &attempts[5]);

    // The fake follows the room gateway: a task becomes non-claimable once
    // its terminal update has been accepted, even if its first HTTP reply was
    // lost. A later poll therefore does not execute it again.
    assert_eq!(dispatcher.run_once().await.unwrap().submitted_updates, 0);
}

fn assert_same_normalized_update(first: &TaskUpdateRequest, retry: &TaskUpdateRequest) {
    assert_eq!(first.update_id, retry.update_id);
    assert_eq!(first.context_revision, retry.context_revision);
    assert_eq!(first.update.event_type, retry.update.event_type);
    assert_eq!(first.update.payload, retry.update.payload);
    assert_eq!(first.update.occurred_at, retry.update.occurred_at);
}

// The room gateway compares duplicate update timestamps as well as identifiers
// and payloads. The test fake itself applies that exact rule when retrying a
// request after a dropped HTTP response.
#[tokio::test]
async fn repeated_poll_does_not_reclaim_an_already_terminal_task() {
    let packet = packet();
    let broker = RecordingBroker::new(packet.clone());
    let runner = FixedRunner::new(
        A2aTaskEvent::from_stream(&packet, stream_for_packet(&packet).unwrap()).unwrap(),
    );
    let dispatcher = Dispatcher::new(
        broker.clone(),
        runner,
        Uuid::from_u128(0x71000000000040008000000000000005),
        StdDuration::ZERO,
    );

    dispatcher.run_once().await.unwrap();
    assert_eq!(dispatcher.run_once().await.unwrap().submitted_updates, 0);
    assert_eq!(broker.updates().await.len(), 4);
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
    attempts: Arc<Mutex<Vec<TaskUpdateRequest>>>,
    state: Arc<Mutex<RecordingBrokerState>>,
}

struct RecordingBrokerState {
    claimed: bool,
    terminal: bool,
    lose_response_for: VecDeque<String>,
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
            attempts: Arc::new(Mutex::new(Vec::new())),
            state: Arc::new(Mutex::new(RecordingBrokerState {
                claimed: false,
                terminal: false,
                lose_response_for: VecDeque::new(),
            })),
        }
    }

    fn with_lost_responses(
        packet: RoomContextPacket,
        event_types: impl IntoIterator<Item = &'static str>,
    ) -> Self {
        let mut broker = Self::new(packet);
        broker.state = Arc::new(Mutex::new(RecordingBrokerState {
            claimed: false,
            terminal: false,
            lose_response_for: event_types.into_iter().map(str::to_owned).collect(),
        }));
        broker
    }

    async fn fetches(&self) -> usize {
        *self.fetch_count.lock().await
    }

    async fn updates(&self) -> Vec<TaskUpdateRequest> {
        self.updates.lock().await.clone()
    }

    async fn attempts(&self) -> Vec<TaskUpdateRequest> {
        self.attempts.lock().await.clone()
    }
}

#[async_trait]
impl RoomTaskBroker for RecordingBroker {
    type Error = RecordingBrokerError;

    async fn claim(&self, _request: ClaimRequest) -> Result<Option<ContextResponse>, Self::Error> {
        let mut state = self.state.lock().await;
        if state.claimed || state.terminal {
            Ok(None)
        } else {
            state.claimed = true;
            Ok(Some(self.context.clone()))
        }
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
        self.attempts.lock().await.push(request.clone());

        let mut accepted = self.updates.lock().await;
        if let Some(existing) = accepted
            .iter()
            .find(|existing| existing.update_id == request.update_id)
        {
            return same_request(existing, &request)
                .then_some(TaskUpdateResponse {
                    events: Vec::<Value>::new(),
                })
                .ok_or(RecordingBrokerError::ConflictingDuplicate);
        }

        let mut state = self.state.lock().await;
        let lost_response = state
            .lose_response_for
            .front()
            .is_some_and(|event_type| event_type == &request.update.event_type);
        accepted.push(request.clone());
        if matches!(
            request.update.event_type.as_str(),
            "agent.task.succeeded" | "agent.task.failed"
        ) {
            state.terminal = true;
        }
        if lost_response {
            state.lose_response_for.pop_front();
            return Err(RecordingBrokerError::ResponseLost);
        }
        Ok(TaskUpdateResponse {
            events: Vec::<Value>::new(),
        })
    }
}

fn same_request(left: &TaskUpdateRequest, right: &TaskUpdateRequest) -> bool {
    left.update_id == right.update_id
        && left.context_revision == right.context_revision
        && left.update.event_type == right.update.event_type
        && left.update.payload == right.update.payload
        && left.update.occurred_at == right.update.occurred_at
}

#[derive(Debug)]
enum RecordingBrokerError {
    ResponseLost,
    ConflictingDuplicate,
}

impl fmt::Display for RecordingBrokerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ResponseLost => formatter.write_str("response was lost after persistence"),
            Self::ConflictingDuplicate => formatter.write_str("duplicate differs from original"),
        }
    }
}

impl Error for RecordingBrokerError {}

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
    type Error = std::convert::Infallible;

    async fn invoke(&self, _packet: &RoomContextPacket) -> Result<Vec<A2aTaskEvent>, Self::Error> {
        Ok(self.events.clone())
    }
}
