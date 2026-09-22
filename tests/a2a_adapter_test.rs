#[cfg(feature = "reference-agent-server")]
use a2a::{
    Message, Part, Role, SendMessageRequest,
    jsonrpc::{JsonRpcId, JsonRpcRequest, methods},
};
#[cfg(feature = "reference-agent-server")]
use a2a_pb::protojson_conv;
#[cfg(feature = "reference-agent-server")]
use axum::{
    body::Body,
    http::{Request, StatusCode, header::AUTHORIZATION},
};
#[cfg(feature = "reference-agent-server")]
use http_body_util::BodyExt;
#[cfg(feature = "reference-agent-server")]
use thought_khoral_agent_gateway::reference_agent::local_router;
use thought_khoral_agent_gateway::{
    RoomContextPacket,
    a2a_adapter::A2aAdapter,
    reference_agent::{canonical_packet_sha256, stream_for_packet},
};
#[cfg(feature = "reference-agent-server")]
use tower::ServiceExt;

fn packet() -> RoomContextPacket {
    serde_json::from_str(include_str!("../fixtures/context-packet.json"))
        .expect("fixture must be a room context packet")
}

// Exercise the patched official transport at its HTTP boundary, including
// frames it skips before yielding a typed protocol event.
#[tokio::test]
async fn official_transport_bounds_raw_frames_comments_and_partial_eof() {
    use a2a_client::{Transport, jsonrpc::JsonRpcTransport};
    use futures::TryStreamExt;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    for body in [
        "x".repeat(65_537),
        ": comment\n\n".repeat(9),
        format!(":{}\n\n", "x".repeat(60_000)).repeat(3),
        "data: {\"unfinished\":".to_owned(),
    ] {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0_u8; 16384];
            assert!(socket.read(&mut request).await.unwrap() > 0);
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            let _ = socket.write_all(response.as_bytes()).await;
        });
        let transport = JsonRpcTransport::new(reqwest::Client::new(), format!("http://{address}"));
        let request = a2a::SendMessageRequest {
            message: a2a::Message::new(a2a::Role::User, vec![a2a::Part::text("test")]),
            configuration: None,
            metadata: None,
            tenant: None,
        };
        let mut stream = transport
            .send_streaming_message(&Default::default(), &request)
            .await
            .unwrap();
        assert!(
            stream.try_next().await.is_err(),
            "oversized or truncated SSE must fail"
        );
        server.await.unwrap();
    }
}

// This catches a gateway that admits a completed A2A artifact without proving
// it is bound to the claimed packet and cites only visible packet events.
#[test]
fn adapter_accepts_only_the_expected_bound_a2a_stream() {
    let packet = packet();
    let events = A2aAdapter::validate_stream(&packet, stream_for_packet(&packet).unwrap())
        .expect("fixed reference-agent stream must be admitted");

    assert_eq!(events.len(), 4);
    assert_eq!(
        events[1].progress_text(),
        Some("Reading authorized room context")
    );
    assert_eq!(events[2].progress_text(), Some("Preparing cited result"));

    let mut invalid = stream_for_packet(&packet).unwrap();
    let completed = invalid.last_mut().expect("completed event");
    let a2a::event::StreamResponse::Task(task) = completed else {
        panic!("completed event must be a task");
    };
    let artifact = task.artifacts.as_mut().unwrap().first_mut().unwrap();
    let a2a::PartContent::Data(data) = &mut artifact.parts[0].content else {
        panic!("artifact must be JSON data");
    };
    data["result"]["citations"] = serde_json::json!(["99000000-0000-4000-8000-000000000001"]);
    assert!(A2aAdapter::validate_stream(&packet, invalid).is_err());

    let mut mismatched_binding = stream_for_packet(&packet).unwrap();
    let a2a::event::StreamResponse::Task(task) = mismatched_binding.last_mut().unwrap() else {
        panic!("completed event must be a task");
    };
    let a2a::PartContent::Data(data) = &mut task.artifacts.as_mut().unwrap()[0].parts[0].content
    else {
        panic!("artifact must be JSON data");
    };
    data["packet"]["contextRevision"] = serde_json::json!(packet.context_revision + 1);
    assert!(A2aAdapter::validate_stream(&packet, mismatched_binding).is_err());
}

// A valid JSON result is not sufficient: the gateway must admit only the
// unique deterministic result for the claimed packet, including its entire
// citation set and the exact action-item parse.
#[test]
fn adapter_rejects_well_formed_but_non_deterministic_terminal_results() {
    let context_packet = packet();

    let mut forged_summary = stream_for_packet(&context_packet).unwrap();
    let a2a::event::StreamResponse::Task(task) = forged_summary.last_mut().unwrap() else {
        panic!("completed event must be a task");
    };
    let a2a::PartContent::Data(data) = &mut task.artifacts.as_mut().unwrap()[0].parts[0].content
    else {
        panic!("artifact must be JSON data");
    };
    data["result"]["summary"] = serde_json::json!("A plausible but forged summary.");
    assert!(A2aAdapter::validate_stream(&context_packet, forged_summary).is_err());

    let mut missing_summary_citation = stream_for_packet(&context_packet).unwrap();
    let a2a::event::StreamResponse::Task(task) = missing_summary_citation.last_mut().unwrap()
    else {
        panic!("completed event must be a task");
    };
    let a2a::PartContent::Data(data) = &mut task.artifacts.as_mut().unwrap()[0].parts[0].content
    else {
        panic!("artifact must be JSON data");
    };
    data["result"]["citations"] = serde_json::json!(["88000000-0000-4000-8000-000000000001"]);
    assert!(A2aAdapter::validate_stream(&context_packet, missing_summary_citation).is_err());

    let mut action_packet = packet();
    action_packet.skill_id = "extract-action-items".to_owned();
    action_packet.events[1]["payload"]["skillId"] = serde_json::json!(action_packet.skill_id);
    action_packet.canonical_sha256 = canonical_packet_sha256(&action_packet).unwrap();
    let mut forged_actions = stream_for_packet(&action_packet).unwrap();
    let a2a::event::StreamResponse::Task(task) = forged_actions.last_mut().unwrap() else {
        panic!("completed event must be a task");
    };
    let a2a::PartContent::Data(data) = &mut task.artifacts.as_mut().unwrap()[0].parts[0].content
    else {
        panic!("artifact must be JSON data");
    };
    data["result"]["actionItems"][0]["text"] = serde_json::json!("Arbitrary agent output");
    assert!(A2aAdapter::validate_stream(&action_packet, forged_actions).is_err());

    let mut wrong_action_citation = stream_for_packet(&action_packet).unwrap();
    let a2a::event::StreamResponse::Task(task) = wrong_action_citation.last_mut().unwrap() else {
        panic!("completed event must be a task");
    };
    let a2a::PartContent::Data(data) = &mut task.artifacts.as_mut().unwrap()[0].parts[0].content
    else {
        panic!("artifact must be JSON data");
    };
    data["result"]["citations"] = serde_json::json!(["88000000-0000-4000-8000-000000000001"]);
    assert!(A2aAdapter::validate_stream(&action_packet, wrong_action_citation).is_err());
}

// Status stream entries are protocol data, not advisory progress. Their task,
// context, role, and one text part must all be bound before room updates are
// emitted.
#[test]
fn adapter_rejects_unbound_or_non_agent_a2a_status_events() {
    let packet = packet();

    let mut wrong_submitted_context = stream_for_packet(&packet).unwrap();
    let a2a::event::StreamResponse::Task(task) = &mut wrong_submitted_context[0] else {
        panic!("submitted event must be a task");
    };
    task.context_id = "other-context".to_owned();
    assert!(A2aAdapter::validate_stream(&packet, wrong_submitted_context).is_err());

    let mut wrong_working_context = stream_for_packet(&packet).unwrap();
    let a2a::event::StreamResponse::StatusUpdate(update) = &mut wrong_working_context[1] else {
        panic!("working event must be a status update");
    };
    update.context_id = "other-context".to_owned();
    assert!(A2aAdapter::validate_stream(&packet, wrong_working_context).is_err());

    let mut non_agent_message = stream_for_packet(&packet).unwrap();
    let a2a::event::StreamResponse::StatusUpdate(update) = &mut non_agent_message[2] else {
        panic!("working event must be a status update");
    };
    update.status.message.as_mut().unwrap().role = a2a::Role::User;
    assert!(A2aAdapter::validate_stream(&packet, non_agent_message).is_err());

    let mut extra_working_part = stream_for_packet(&packet).unwrap();
    let a2a::event::StreamResponse::StatusUpdate(update) = &mut extra_working_part[2] else {
        panic!("working event must be a status update");
    };
    update
        .status
        .message
        .as_mut()
        .unwrap()
        .parts
        .push(a2a::Part::text("unexpected second part"));
    assert!(A2aAdapter::validate_stream(&packet, extra_working_part).is_err());

    let mut wrong_completed_context = stream_for_packet(&packet).unwrap();
    let a2a::event::StreamResponse::Task(task) = wrong_completed_context.last_mut().unwrap() else {
        panic!("completed event must be a task");
    };
    task.context_id = "other-context".to_owned();
    assert!(A2aAdapter::validate_stream(&packet, wrong_completed_context).is_err());
}

// This catches a local endpoint that exposes the Agent Card or JSON-RPC
// transport without the dedicated agent-gateway bearer secret.
#[tokio::test]
#[cfg(feature = "reference-agent-server")]
async fn local_a2a_routes_require_the_agent_gateway_bearer_secret() {
    let unauthorized = local_router("reference-agent-secret")
        .oneshot(
            Request::builder()
                .uri("/.well-known/agent-card.json")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);

    let authorized = local_router("reference-agent-secret")
        .oneshot(
            Request::builder()
                .uri("/.well-known/agent-card.json")
                .header(AUTHORIZATION, "Bearer reference-agent-secret")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(authorized.status(), StatusCode::OK);
    let body = authorized.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&body).unwrap()["name"],
        "Reference Agent"
    );
}

// This is an HTTP/JSON-RPC black-box assertion over the official server
// router. It catches a route that authenticates the card but not task streams,
// or serializes a stream outside the pinned A2A protocol.
#[tokio::test]
#[cfg(feature = "reference-agent-server")]
async fn local_jsonrpc_stream_requires_bearer_and_returns_the_fixed_four_events() {
    let packet = packet();
    let mut message = Message::new(
        Role::User,
        vec![Part::text(
            serde_json::to_string(&serde_json::json!({ "packet": packet })).unwrap(),
        )],
    );
    message.task_id = Some(packet.task_id.to_string());
    message.context_id = Some(format!("{}:{}", packet.room_id, packet.context_revision));
    let request = JsonRpcRequest::new(
        JsonRpcId::Number(1),
        methods::SEND_STREAMING_MESSAGE,
        Some(
            protojson_conv::to_value(&SendMessageRequest {
                message,
                configuration: None,
                metadata: None,
                tenant: None,
            })
            .unwrap(),
        ),
    );
    let request_body = serde_json::to_vec(&request).unwrap();
    let response = local_router("reference-agent-secret")
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/jsonrpc")
                .header(AUTHORIZATION, "Bearer reference-agent-secret")
                .header("accept", "text/event-stream")
                .header("content-type", "application/json")
                .body(Body::from(request_body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert!(
        response
            .headers()
            .get("content-type")
            .unwrap()
            .as_bytes()
            .starts_with(b"text/event-stream")
    );
    let stream = String::from_utf8(
        response
            .into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes()
            .to_vec(),
    )
    .unwrap();
    assert_eq!(stream.matches("data:").count(), 4, "{stream}");
    assert!(stream.contains("TASK_STATE_SUBMITTED"));
    assert!(stream.contains("Reading authorized room context"));
    assert!(stream.contains("Preparing cited result"));
    assert!(stream.contains("TASK_STATE_COMPLETED"));
}
