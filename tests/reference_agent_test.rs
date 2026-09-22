use std::collections::BTreeSet;

use a2a::{PartContent, TaskState, event::StreamResponse};
use chrono::{Duration, Utc};
use thought_khoral_agent_gateway::{
    RoomContextPacket,
    reference_agent::{agent_card, canonical_packet_sha256, stream_for_packet},
};

fn packet() -> RoomContextPacket {
    serde_json::from_str(include_str!("../fixtures/context-packet.json"))
        .expect("fixture must be a room context packet")
}

#[test]
fn authoritative_packet_input_executes_without_a_contract_invalid_event_input() {
    let mut packet = packet();
    packet.skill_id = "extract-action-items".to_owned();
    packet.events[1]["payload"] = serde_json::json!({
        "taskId": packet.task_id, "agentId": packet.agent_id,
        "requesterId": packet.requester_id, "skillId": packet.skill_id,
        "contextRevision": packet.context_revision,
    });
    packet.canonical_sha256 = canonical_packet_sha256(&packet).unwrap();
    let stream = stream_for_packet(&packet).expect("broker packets carry input in task storage");
    assert_eq!(
        completed_result(&stream[3])["actionItems"][0]["text"],
        "Prepare rollout checklist"
    );
}

#[test]
fn large_room_summary_stays_within_the_published_bounds() {
    let mut packet = packet();
    for index in 10..250_u128 {
        let mut event = packet.events[0].clone();
        event["eventId"] = serde_json::json!(uuid::Uuid::from_u128(index));
        packet.events.push(event);
    }
    packet.active_decisions = (0..200)
        .map(|_| {
            let mut decision = packet.active_decisions[0].clone();
            decision.title = "決".repeat(500);
            decision
        })
        .collect();
    packet.canonical_sha256 = canonical_packet_sha256(&packet).unwrap();
    let stream = stream_for_packet(&packet).unwrap();
    let result = completed_result(&stream[3]);
    assert!(result["citations"].as_array().unwrap().len() <= 100);
    assert!(result["summary"].as_str().unwrap().chars().count() <= 8000);
    assert!(
        result["summary"]
            .as_str()
            .unwrap()
            .contains("200 active decisions")
    );
    assert_eq!(stream, stream_for_packet(&packet).unwrap());
}

// A change that emits a non-A2A stream, changes the fixed progress wording, or
// derives the summary from message contents rather than the authorized packet
// metadata must make this fail.
#[test]
fn summarize_context_emits_the_fixed_deterministic_a2a_stream() {
    let packet = packet();
    let first = stream_for_packet(&packet).expect("valid packet must execute");
    let second = stream_for_packet(&packet).expect("same valid packet must execute");

    assert_eq!(first, second);
    assert_eq!(first.len(), 4);
    assert!(matches!(
        first[0],
        StreamResponse::Task(ref task) if task.status.state == TaskState::Submitted
    ));
    assert_eq!(
        status_text(&first[1]),
        Some("Reading authorized room context")
    );
    assert_eq!(status_text(&first[2]), Some("Preparing cited result"));

    let result = completed_result(&first[3]);
    assert_eq!(result["kind"], "context-summary.v1");
    assert_eq!(
        result["summary"],
        "Room 10000000-0000-4000-8000-000000000001 revision 7 has 2 messages and active decisions: Ship after security review; Use packet citations."
    );
    assert_eq!(
        result["citations"],
        serde_json::json!([
            "88000000-0000-4000-8000-000000000001",
            "88000000-0000-4000-8000-000000000002",
            "88000000-0000-4000-8000-000000000003"
        ])
    );
}

// This catches an Agent Card that drifts from the Task 4 registration or adds
// a non-deterministic skill or transport.
#[test]
fn reference_agent_card_is_fixed_to_the_pinned_jsonrpc_skills() {
    let card = agent_card();
    assert_eq!(card.name, "Reference Agent");
    assert_eq!(card.supported_interfaces.len(), 1);
    assert_eq!(
        card.supported_interfaces[0].url,
        "http://127.0.0.1:9090/jsonrpc"
    );
    assert_eq!(card.supported_interfaces[0].protocol_binding, "JSONRPC");
    assert_eq!(card.supported_interfaces[0].protocol_version, "1.0");
    assert_eq!(
        card.skills
            .into_iter()
            .map(|skill| skill.id)
            .collect::<BTreeSet<_>>(),
        BTreeSet::from([
            "extract-action-items".to_owned(),
            "summarize-context".to_owned(),
        ])
    );
}

#[test]
fn context_packet_fixture_has_the_broker_canonical_hash() {
    let packet = packet();
    assert_eq!(
        canonical_packet_sha256(&packet).unwrap(),
        packet.canonical_sha256
    );
}

// This catches either accepting a packet whose serialized fields changed after
// the broker hash or treating an expired lease packet as executable context.
#[test]
fn reference_agent_rejects_tampered_and_expired_packets_before_execution() {
    let mut tampered = packet();
    tampered
        .input
        .push_str("\n- Injected | owner: Mallory | due: never");
    assert!(stream_for_packet(&tampered).is_err());

    let mut expired = packet();
    expired.issued_at = Utc::now() - Duration::minutes(2);
    expired.expires_at = Utc::now() - Duration::minutes(1);
    expired.canonical_sha256 = canonical_packet_sha256(&expired).unwrap();
    assert!(stream_for_packet(&expired).is_err());
}

// This catches accepting natural-language guesses, partial lines, or citations
// copied from unrelated packet events.
#[test]
fn extract_action_items_accepts_only_the_strict_line_grammar_and_cites_invocation() {
    let mut packet = packet();
    packet.skill_id = "extract-action-items".to_owned();
    packet.events[1]["payload"]["skillId"] = serde_json::json!(packet.skill_id);
    packet.canonical_sha256 = canonical_packet_sha256(&packet).expect("mutated fixture must hash");

    let stream = stream_for_packet(&packet).expect("valid action-item packet must execute");
    let result = completed_result(&stream[3]);
    assert_eq!(result["kind"], "action-items.v1");
    assert_eq!(
        result["actionItems"],
        serde_json::json!([{
            "text": "Prepare rollout checklist",
            "owner": "Maya",
            "due": "Friday"
        }])
    );
    assert_eq!(
        result["citations"],
        serde_json::json!(["88000000-0000-4000-8000-000000000002"])
    );
}

// The accepted grammar is deliberately literal. Whitespace-normalizing a
// nearly matching line would turn unconstrained prose into a governed action.
#[test]
fn extract_action_items_rejects_whitespace_variants_of_the_literal_grammar() {
    let mut packet = packet();
    packet.skill_id = "extract-action-items".to_owned();
    packet.input = concat!(
        "-  Two spaces after dash | owner: Maya | due: Friday\n",
        "- Owner starts with space | owner:  Maya | due: Friday\n",
        "- Due ends with space | owner: Maya | due: Friday "
    )
    .to_owned();
    let invocation = packet
        .events
        .iter_mut()
        .find(|event| event["eventType"] == "agent.task.requested")
        .expect("fixture has task invocation");
    invocation["payload"]["skillId"] = serde_json::json!(packet.skill_id);
    packet.canonical_sha256 = canonical_packet_sha256(&packet).unwrap();

    let stream = stream_for_packet(&packet).expect("packet itself remains authorized");
    let result = completed_result(&stream[3]);
    assert_eq!(result["actionItems"], serde_json::json!([]));
    assert_eq!(
        result["citations"],
        serde_json::json!(["88000000-0000-4000-8000-000000000002"])
    );
}

fn status_text(event: &StreamResponse) -> Option<&str> {
    let StreamResponse::StatusUpdate(update) = event else {
        return None;
    };
    update.status.message.as_ref()?.parts.first()?.as_text()
}

fn completed_result(event: &StreamResponse) -> serde_json::Value {
    let StreamResponse::Task(task) = event else {
        panic!("expected completed task");
    };
    assert_eq!(task.status.state, TaskState::Completed);
    let artifact = task
        .artifacts
        .as_ref()
        .and_then(|artifacts| artifacts.first())
        .expect("completed task must contain an artifact");
    let PartContent::Data(data) = &artifact.parts[0].content else {
        panic!("artifact must be JSON data");
    };
    data["result"].clone()
}
