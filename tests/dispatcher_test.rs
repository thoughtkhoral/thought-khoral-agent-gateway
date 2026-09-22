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

#[test]
fn contract_validation_uses_the_vendored_hash_locked_artifact() {
    use sha2::{Digest, Sha256};
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("contracts");
    let lock: Value = serde_json::from_slice(
        &std::fs::read(root.join("lock.json"))
            .expect("contract must be vendored in this repository"),
    )
    .unwrap();
    for name in [
        "room-event.schema.json",
        "envelope.schema.json",
        "rpc.schema.json",
    ] {
        let bytes = std::fs::read(root.join("n2n.room.v1/schemas").join(name)).unwrap();
        assert_eq!(
            format!("{:x}", Sha256::digest(bytes)),
            lock["schemas"][name]
        );
    }
}

#[tokio::test]
async fn renewed_leases_replay_identical_update_timestamps_and_ids() {
    let first = packet();
    let mut renewed = first.clone();
    renewed.issued_at += Duration::seconds(60);
    renewed.expires_at += Duration::seconds(60);
    renewed.canonical_sha256 = canonical_packet_sha256(&renewed).unwrap();
    let mut outputs = Vec::new();
    for packet in [first, renewed] {
        let broker = RecordingBroker::new(packet.clone());
        let runner = FixedRunner::new(
            A2aTaskEvent::from_stream(&packet, stream_for_packet(&packet).unwrap()).unwrap(),
        );
        Dispatcher::new(broker.clone(), runner, Uuid::new_v4(), StdDuration::ZERO)
            .run_once()
            .await
            .unwrap();
        outputs.push(broker.updates().await);
    }
    for (first, renewed) in outputs[0].iter().zip(&outputs[1]) {
        assert_same_normalized_update(first, renewed);
    }
}

#[tokio::test]
async fn deterministic_rejection_is_not_retried_and_becomes_a_terminal_failure() {
    let packet = packet();
    let mut broker = RecordingBroker::new(packet.clone());
    broker.reject_success = true;
    let runner = FixedRunner::new(
        A2aTaskEvent::from_stream(&packet, stream_for_packet(&packet).unwrap()).unwrap(),
    );
    let dispatcher = Dispatcher::new(
        broker.clone(),
        runner,
        Uuid::new_v4(),
        StdDuration::from_millis(10),
    );
    tokio::time::timeout(StdDuration::from_millis(500), dispatcher.run_once())
        .await
        .expect("a deterministic 422 must terminate promptly")
        .unwrap();
    assert_eq!(
        broker
            .attempts()
            .await
            .iter()
            .filter(|request| request.update.event_type == "agent.task.succeeded")
            .count(),
        1
    );
    assert_eq!(
        broker.updates().await.last().unwrap().update.event_type,
        "agent.task.failed"
    );
}

#[tokio::test]
async fn hanging_runner_is_cancelled_before_lease_expiry() {
    struct HangingRunner;
    #[async_trait]
    impl A2aTaskRunner for HangingRunner {
        type Error = std::convert::Infallible;
        async fn invoke(&self, _: &RoomContextPacket) -> Result<Vec<A2aTaskEvent>, Self::Error> {
            std::future::pending().await
        }
    }
    let mut packet = packet();
    packet.issued_at = Utc::now();
    packet.expires_at = Utc::now() + Duration::seconds(3);
    packet.canonical_sha256 = canonical_packet_sha256(&packet).unwrap();
    let broker = RecordingBroker::new(packet);
    let dispatcher = Dispatcher::new(
        broker.clone(),
        HangingRunner,
        Uuid::new_v4(),
        StdDuration::ZERO,
    );
    tokio::time::timeout(StdDuration::from_secs(2), dispatcher.run_once())
        .await
        .expect("lease-aware deadline must leave time to persist failure")
        .unwrap();
    assert_eq!(
        broker.updates().await.last().unwrap().update.event_type,
        "agent.task.failed"
    );
}

#[tokio::test]
async fn progress_is_persisted_while_the_agent_stream_is_still_open() {
    struct StreamingRunner(RecordingBroker);
    #[async_trait]
    impl A2aTaskRunner for StreamingRunner {
        type Error = std::convert::Infallible;
        async fn invoke(&self, _: &RoomContextPacket) -> Result<Vec<A2aTaskEvent>, Self::Error> {
            panic!("dispatcher must consume the incremental stream");
        }
        async fn invoke_stream(
            &self,
            packet: &RoomContextPacket,
            sender: tokio::sync::mpsc::Sender<A2aTaskEvent>,
        ) -> Result<(), Self::Error> {
            let events =
                A2aTaskEvent::from_stream(packet, stream_for_packet(packet).unwrap()).unwrap();
            sender.send(events[0].clone()).await.unwrap();
            tokio::time::timeout(StdDuration::from_millis(500), async {
                while self.0.updates().await.is_empty() {
                    tokio::task::yield_now().await;
                }
            })
            .await
            .expect("accepted progress must reach the broker before stream completion");
            for event in events.into_iter().skip(1) {
                sender.send(event).await.unwrap();
            }
            Ok(())
        }
    }
    let broker = RecordingBroker::new(packet());
    let outcome = Dispatcher::new(
        broker.clone(),
        StreamingRunner(broker.clone()),
        Uuid::new_v4(),
        StdDuration::ZERO,
    )
    .run_once()
    .await
    .unwrap();
    assert_eq!(outcome.submitted_updates, 4);
    assert_eq!(
        broker.updates().await.last().unwrap().update.event_type,
        "agent.task.succeeded"
    );
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
async fn invalid_terminal_response_preserves_admitted_progress_and_emits_safe_failure() {
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
    assert_eq!(updates.len(), 4);
    assert!(
        updates[..3]
            .iter()
            .all(|update| update.update.event_type == "agent.task.progressed")
    );
    assert_eq!(updates[3].update.event_type, "agent.task.failed");
    assert_eq!(
        updates[3].update.payload["failure"]["code"],
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

// The instances below originate from real dispatcher failure paths, not
// hand-written failure codes. They are hydrated exactly as the room gateway
// adds its external-task core before validating a persisted event. The pinned
// contract schema and its pinned envelope resource must accept both emitted
// codes and reject an internal-only code in the same instance shape.
#[tokio::test]
async fn emitted_failed_updates_validate_against_the_pinned_room_event_schema() {
    let execution_packet = packet();
    let execution_broker = RecordingBroker::new(execution_packet.clone());
    let mut invalid_events = A2aTaskEvent::from_stream(
        &execution_packet,
        stream_for_packet(&execution_packet).unwrap(),
    )
    .unwrap();
    invalid_events[3].set_result(serde_json::json!({
        "kind": "context-summary.v1",
        "summary": "forged",
        "citations": ["99000000-0000-4000-8000-000000000001"]
    }));
    Dispatcher::new(
        execution_broker.clone(),
        FixedRunner::new(invalid_events),
        Uuid::from_u128(0x71000000000040008000000000000006),
        StdDuration::ZERO,
    )
    .run_once()
    .await
    .unwrap();

    let mut input_packet = packet();
    input_packet.issued_at = Utc::now() + Duration::minutes(2);
    input_packet.expires_at = Utc::now() + Duration::minutes(1);
    input_packet.canonical_sha256 = canonical_packet_sha256(&input_packet).unwrap();
    let input_broker = RecordingBroker::new(input_packet.clone());
    Dispatcher::new(
        input_broker.clone(),
        FixedRunner::new(Vec::new()),
        Uuid::from_u128(0x71000000000040008000000000000007),
        StdDuration::ZERO,
    )
    .run_once()
    .await
    .unwrap();

    let validator = pinned_room_event_validator();
    let execution_update = only_failed_update(&execution_broker).await;
    let input_update = only_failed_update(&input_broker).await;
    let execution_event = persisted_external_task_event(&execution_packet, &execution_update);
    let input_event = persisted_external_task_event(&input_packet, &input_update);

    assert_eq!(
        execution_event.pointer("/payload/failure/code"),
        Some(&serde_json::json!("execution_failed"))
    );
    assert_eq!(
        input_event.pointer("/payload/failure/code"),
        Some(&serde_json::json!("invalid_task_input"))
    );
    assert!(
        validator.validate(&execution_event).is_ok(),
        "emitted execution_failed event must satisfy the pinned room-event schema"
    );
    assert!(
        validator.validate(&input_event).is_ok(),
        "emitted invalid_task_input event must satisfy the pinned room-event schema"
    );

    for unsupported_code in ["invalid_agent_response", "context_expired"] {
        let mut unsupported_internal_code = execution_event.clone();
        unsupported_internal_code["payload"]["failure"]["code"] =
            serde_json::json!(unsupported_code);
        assert!(
            validator.validate(&unsupported_internal_code).is_err(),
            "the same room-event schema must reject {unsupported_code}"
        );
    }
}

async fn only_failed_update(broker: &RecordingBroker) -> TaskUpdateRequest {
    let updates = broker
        .updates()
        .await
        .into_iter()
        .filter(|update| update.update.event_type == "agent.task.failed")
        .collect::<Vec<_>>();
    assert_eq!(updates.len(), 1);
    assert_eq!(updates[0].update.event_type, "agent.task.failed");
    updates.into_iter().next().unwrap()
}

fn pinned_room_event_validator() -> jsonschema::Validator {
    let schema: Value = serde_json::from_str(include_str!(
        "../contracts/n2n.room.v1/schemas/room-event.schema.json"
    ))
    .expect("pinned room-event schema must be valid JSON");
    let envelope: Value = serde_json::from_str(include_str!(
        "../contracts/n2n.room.v1/schemas/envelope.schema.json"
    ))
    .expect("pinned envelope schema must be valid JSON");
    jsonschema::options()
        .with_draft(jsonschema::Draft::Draft202012)
        .should_validate_formats(true)
        .with_resource(
            "https://n2n.redhat.com/schemas/n2n.room.v1/envelope.schema.json",
            jsonschema::Resource::from_contents(envelope),
        )
        .build(&schema)
        .expect("pinned room-event schema must compile without retrieval")
}

fn persisted_external_task_event(packet: &RoomContextPacket, update: &TaskUpdateRequest) -> Value {
    let mut payload = update.update.payload.clone();
    let object = payload
        .as_object_mut()
        .expect("dispatcher updates are JSON objects");
    object.insert("taskId".to_owned(), serde_json::json!(packet.task_id));
    object.insert("agentId".to_owned(), serde_json::json!(packet.agent_id));
    object.insert(
        "requesterId".to_owned(),
        serde_json::json!(packet.requester_id),
    );
    object.insert("skillId".to_owned(), serde_json::json!(packet.skill_id));
    object.insert(
        "contextRevision".to_owned(),
        serde_json::json!(packet.context_revision),
    );
    serde_json::json!({
        "contractVersion": "n2n.room.v1",
        "requestId": update.update_id,
        "roomId": packet.room_id,
        "occurredAt": update.update.occurred_at,
        "sequence": 1,
        "eventId": update.update_id,
        "eventType": update.update.event_type,
        "actor": { "id": packet.agent_id, "role": "agent" },
        "payload": payload,
    })
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
    reject_success: bool,
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
            reject_success: false,
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
    fn retryable(&self, error: &Self::Error) -> bool {
        matches!(error, RecordingBrokerError::ResponseLost)
    }

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
        if self.reject_success && request.update.event_type == "agent.task.succeeded" {
            return Err(RecordingBrokerError::Rejected);
        }

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
    Rejected,
    ResponseLost,
    ConflictingDuplicate,
}

impl fmt::Display for RecordingBrokerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Rejected => formatter.write_str("422 unprocessable entity"),
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
