use axum::{
    Json, Router,
    extract::{Request, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    net::{IpAddr, Ipv4Addr},
    sync::Arc,
};
use thought_khoral_agent_gateway::{
    GatewayConfig, RoomGatewayClient,
    conversation_dispatcher::{ConversationDispatcher, WorkerClient},
    conversation_validation::{Admission, PROFILE, context_digest},
};
use tokio::sync::Mutex;
static SERIAL: Mutex<()> = Mutex::const_new(());
#[derive(Clone)]
struct Fixture(Arc<Mutex<Data>>);
struct Data {
    packet: Value,
    claimed: bool,
    prior_receipts: BTreeMap<String, Value>,
    prior_authority: BTreeMap<String, StatusCode>,
    prior_authority_values: BTreeMap<String, Value>,
    receipt: Option<Value>,
    broker_ack: Value,
    sends: u32,
    cancels: u32,
    worker_acks: u32,
    updates: Vec<Value>,
    lost_ack: bool,
    lose_before_accept: bool,
    artifact_mode: Option<&'static str>,
    revoked: bool,
    slow: bool,
    drift: bool,
    citation: Option<String>,
    actual_settings: Value,
    actual_usage: Value,
    headers: Vec<String>,
}
fn reply(p: &Value) -> Value {
    json!({"kind":"conversation-reply.v1","conversationId":p["conversation"]["id"],"generation":p["conversation"]["generation"],"assistantText":"normalized reply","consumedRevision":p["context"]["revision"],"contextDigest":p["context"]["digest"],"citations":[],"effectiveSettings":null,"usage":null})
}
fn receipt(p: &Value, phase: &str) -> Value {
    json!({"taskId":p["taskId"],"conversationId":p["conversation"]["id"],"generation":p["conversation"]["generation"],"phase":phase,"result":if phase=="completed"{reply(p)}else{Value::Null},"acknowledgement":null,"error":null,"runtimeBinding":if phase=="completed"{json!({"threadId":"native-thread","turnId":"native-turn"})}else{Value::Null}})
}
async fn worker(State(f): State<Fixture>, request: Request) -> Response {
    let auth = request
        .headers()
        .get("authorization")
        .unwrap()
        .to_str()
        .unwrap()
        .to_owned();
    let path = request.uri().path().to_owned();
    let bytes = axum::body::to_bytes(request.into_body(), 2_097_152)
        .await
        .unwrap();
    let mut d = f.0.lock().await;
    d.headers.push(auth);
    if path.ends_with("agent-card.json") {
        if d.artifact_mode == Some("redirect") {
            return (
                StatusCode::FOUND,
                [("location", "http://forbidden.invalid/card")],
            )
                .into_response();
        }
        return Json(thought_khoral_agent_gateway::registry::codex_card()).into_response();
    }
    if path.ends_with("models") {
        if d.artifact_mode == Some("missing-model") {
            return Json(json!({"profileVersion":PROFILE,"catalogRevision":"catalog-1","data":[],"nextCursor":null})).into_response();
        }
        return Json(
            serde_json::from_str::<Value>(include_str!(
                "../contracts/agent-conversation-v1/fixtures/valid/catalog.json"
            ))
            .unwrap(),
        )
        .into_response();
    }
    if path.ends_with("/ack") {
        d.worker_acks += 1;
        return StatusCode::NO_CONTENT.into_response();
    }
    if path.contains("/receipts/") {
        return match &d.receipt {
            Some(v) => Json(v.clone()).into_response(),
            None => (
                StatusCode::BAD_REQUEST,
                Json(json!({"code":"invalid_task_input"})),
            )
                .into_response(),
        };
    }
    let rpc: Value = serde_json::from_slice(&bytes).unwrap();
    if rpc["method"] == "CancelTask" {
        d.cancels += 1;
        return Json(json!({"jsonrpc":"2.0","id":rpc["id"],"result":{}})).into_response();
    }
    assert_eq!(rpc["method"], "SendMessage");
    let packet = &rpc["params"]["message"]["parts"][0]["data"]["packet"];
    assert_eq!(packet, &d.packet);
    assert_eq!(rpc["params"]["message"]["taskId"], packet["taskId"]);
    assert_eq!(
        rpc["params"]["message"]["contextId"],
        packet["conversation"]["id"]
    );
    assert_eq!(
        rpc["params"]["message"]["parts"].as_array().unwrap().len(),
        1
    );
    d.sends += 1;
    let slow = d.slow;
    drop(d);
    if slow {
        tokio::time::sleep(std::time::Duration::from_millis(1500)).await
    }
    let mut d = f.0.lock().await;
    let mut r = receipt(&d.packet, "completed");
    if let Some(citation) = &d.citation {
        r["result"]["citations"] = json!([citation]);
    }
    if !d.actual_settings.is_null() {
        r["result"]["effectiveSettings"] = d.actual_settings.clone();
        r["result"]["usage"] = d.actual_usage.clone();
    }
    d.receipt = Some(r.clone());
    let mut binding = r["runtimeBinding"].clone();
    if d.drift {
        binding["turnId"] = json!("wrong-turn")
    };
    let mut output = json!({"jsonrpc":"2.0","id":rpc["id"],"result":{"task":{"id":d.packet["taskId"],"contextId":d.packet["conversation"]["id"],"status":{"state":"TASK_STATE_COMPLETED"},"artifacts":[{"artifactId":"reply","parts":[{"data":{"reply":r["result"],"runtimeBinding":binding}}]}]}}});
    match d.artifact_mode {
        Some("task") => output["result"]["task"]["id"] = json!(uuid::Uuid::new_v4()),
        Some("usage") => {
            output["result"]["task"]["artifacts"][0]["parts"][0]["data"]["reply"]["usage"] =
                json!({"forged":true})
        }
        Some("artifact") => output["result"]["task"]["artifacts"][0]["parts"]
            .as_array_mut()
            .unwrap()
            .push(json!({"text":"arbitrary"})),
        _ => (),
    }
    Json(output).into_response()
}
async fn broker(State(f): State<Fixture>, request: Request) -> Response {
    let path = request.uri().path().to_owned();
    if path.ends_with("/token") {
        return Json(json!({"access_token":"workload-token"})).into_response();
    }
    assert_eq!(
        request.headers().get("authorization").unwrap(),
        "Bearer workload-token"
    );
    if path.ends_with("context") || path.ends_with("authority") || path.ends_with("updates") {
        assert_eq!(
            request
                .headers()
                .get("x-thought-khoral-lease-token")
                .unwrap(),
            "fixture-lease"
        )
    };
    let bytes = axum::body::to_bytes(request.into_body(), 2_097_152)
        .await
        .unwrap();
    let mut d = f.0.lock().await;
    if path.ends_with("claim") {
        if d.claimed {
            return StatusCode::NO_CONTENT.into_response();
        }
        d.claimed = true;
        let claim: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(claim["leaseOwner"], "fixture-owner");
        return Json(json!({"packet":d.packet,"leaseToken":"fixture-lease"})).into_response();
    }
    let task = path.split('/').rev().nth(1).unwrap_or("");
    if path.ends_with("receipt") {
        if let Some(prior) = d.prior_receipts.get(task) {
            return Json(prior.clone()).into_response();
        }
        return Json(json!({"profileVersion":PROFILE,"taskId":d.packet["taskId"],"conversationId":d.packet["conversation"]["id"],"generation":d.packet["conversation"]["generation"],"state":if d.broker_ack.is_null(){"reserved"}else{"completed"},"acknowledgement":d.broker_ack,"result":if d.broker_ack.is_null(){Value::Null}else{reply(&d.packet)}})).into_response();
    }
    if path.ends_with("authority") {
        if let Some(value) = d.prior_authority_values.get(task) {
            return Json(value.clone()).into_response();
        }
        if let Some(status) = d.prior_authority.get(task) {
            return (*status).into_response();
        }
        if d.revoked {
            return StatusCode::FORBIDDEN.into_response();
        }
        return Json(json!({"profileVersion":PROFILE,"taskId":d.packet["taskId"],"generation":d.packet["conversation"]["generation"],"contextDigest":d.packet["context"]["digest"],"expiresAt":d.packet["expiresAt"],"authorizationExpiresAt":d.packet["authorizationExpiresAt"]})).into_response();
    }
    if path.ends_with("context") {
        return Json(d.packet.clone()).into_response();
    }
    let update: Value = serde_json::from_slice(&bytes).unwrap();
    d.updates.push(update.clone());
    if update["kind"] == "completed" {
        if d.lose_before_accept {
            d.lose_before_accept = false;
            return StatusCode::SERVICE_UNAVAILABLE.into_response();
        }
        d.broker_ack = json!({"profileVersion":PROFILE,"taskId":d.packet["taskId"],"conversationId":d.packet["conversation"]["id"],"generation":d.packet["conversation"]["generation"],"replyEventId":"ffffffff-ffff-4fff-8fff-ffffffffffff","replySequence":6,"textDigest":format!("{:x}",<sha2::Sha256 as sha2::Digest>::digest("normalized reply")),"consumedRevision":d.packet["context"]["revision"],"contextDigest":d.packet["context"]["digest"]});
        if d.lost_ack {
            d.lost_ack = false;
            return StatusCode::SERVICE_UNAVAILABLE.into_response();
        }
        return Json(d.broker_ack.clone()).into_response();
    }
    Json(json!({"accepted":true})).into_response()
}
async fn fixture_at(
    state: Option<std::path::PathBuf>,
) -> (
    Fixture,
    ConversationDispatcher,
    std::path::PathBuf,
    Vec<tokio::task::JoinHandle<()>>,
) {
    let ip = IpAddr::V4(Ipv4Addr::LOCALHOST);
    let mut p: Value = serde_json::from_str(include_str!(
        "../contracts/agent-conversation-v1/fixtures/valid/baseline-hidden-sequence-gap.json"
    ))
    .unwrap();
    let now = chrono::Utc::now();
    p["issuedAt"] = json!(now.to_rfc3339());
    for k in ["expiresAt", "authorizationExpiresAt", "leaseExpiresAt"] {
        p[k] = json!((now + chrono::Duration::seconds(180)).to_rfc3339())
    }
    p["leaseOwner"] = json!("fixture-owner");
    p["context"]["digest"] = json!(context_digest(&p).unwrap());
    let f = Fixture(Arc::new(Mutex::new(Data {
        packet: p,
        claimed: false,
        prior_receipts: BTreeMap::new(),
        prior_authority: BTreeMap::new(),
        prior_authority_values: BTreeMap::new(),
        receipt: None,
        broker_ack: Value::Null,
        sends: 0,
        cancels: 0,
        worker_acks: 0,
        updates: vec![],
        lost_ack: false,
        lose_before_accept: false,
        artifact_mode: None,
        revoked: false,
        slow: false,
        drift: false,
        citation: None,
        actual_settings: Value::Null,
        actual_usage: Value::Null,
        headers: vec![],
    })));
    let mut handles = vec![];
    for (port, router) in [
        (
            9091,
            Router::new()
                .route("/", post(worker))
                .route("/.well-known/agent-card.json", get(worker))
                .route("/control/v1/models", get(worker))
                .route("/control/v1/receipts/{task}", get(worker))
                .route("/control/v1/receipts/{task}/ack", post(worker)),
        ),
        (8080, Router::new().fallback(broker)),
    ] {
        let listener = tokio::net::TcpListener::bind((ip, port)).await.unwrap();
        let app = router.with_state(f.clone());
        handles.push(tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap()
        }));
    }
    let mut env = BTreeMap::new();
    for (k, v) in [
        (
            "THOUGHT_KHORAL_ROOM_GATEWAY_ORIGIN",
            "http://thought-khoral-room-gateway:8080",
        ),
        (
            "THOUGHT_KHORAL_KEYCLOAK_TOKEN_URL",
            "http://thought-khoral-keycloak:8080/realms/thought-khoral/protocol/openid-connect/token",
        ),
        (
            "THOUGHT_KHORAL_AGENT_GATEWAY_CLIENT_ID",
            "thought-khoral-agent-gateway",
        ),
        (
            "THOUGHT_KHORAL_AGENT_GATEWAY_CLIENT_SECRET",
            "client-secret",
        ),
        (
            "THOUGHT_KHORAL_REFERENCE_AGENT_INBOUND_SECRET",
            "reference-secret",
        ),
        (
            "THOUGHT_KHORAL_REFERENCE_AGENT_CARD_URL",
            "http://127.0.0.1:9090",
        ),
        ("THOUGHT_KHORAL_ALLOWED_HANDOFF_HOSTS", "example.test"),
        (
            "THOUGHT_KHORAL_REFERENCE_AGENT_HANDOFF_HOST",
            "example.test",
        ),
        ("THOUGHT_KHORAL_AGENT_POLL_MILLIS", "100"),
    ] {
        env.insert(k.to_owned(), v.to_owned());
    }
    let config = GatewayConfig::parse(env).unwrap();
    let admission = Admission::new(
        "catalog-1".into(),
        "fixed-1".into(),
        [
            ("model-a".into(), vec!["effort-medium".into()]),
            ("model-b".into(), vec!["effort-medium".into()]),
        ]
        .into(),
    )
    .unwrap();
    let room = RoomGatewayClient::from_config_with_loopback_dns(&config, ip).unwrap();
    let worker =
        WorkerClient::with_loopback_dns("distinct-worker-secret".into(), admission, ip).unwrap();
    let dir = state.unwrap_or_else(|| {
        std::env::temp_dir().join(format!("mediation-test-{}", uuid::Uuid::new_v4()))
    });
    let dispatcher =
        ConversationDispatcher::new(room, worker, dir.clone(), "fixture-owner".into()).unwrap();
    (f, dispatcher, dir, handles)
}
async fn fixture() -> (
    Fixture,
    ConversationDispatcher,
    std::path::PathBuf,
    Vec<tokio::task::JoinHandle<()>>,
) {
    fixture_at(None).await
}
async fn shutdown(handles: Vec<tokio::task::JoinHandle<()>>) {
    for h in handles {
        h.abort();
        let _ = h.await;
    }
}
#[tokio::test]
async fn continuation_cites_prior_disclosure_after_mediator_restart_but_new_generation_cannot() {
    let _serial = SERIAL.lock().await;
    let (f, d, path, handles) = fixture().await;
    let old_id = f.0.lock().await.packet["context"]["entries"][0]["eventId"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(d.run_once().await.unwrap());
    drop(d);
    shutdown(handles).await;
    let (f, d, _, handles) = fixture_at(Some(path.clone())).await;
    {
        let mut data = f.0.lock().await;
        let p = &mut data.packet;
        p["taskId"] = json!(uuid::Uuid::new_v4());
        p["conversation"]["mode"] = json!("continue");
        p["context"]["kind"] = json!("delta");
        p["context"]["baseRevision"] = json!(5);
        p["context"]["revision"] = json!(7);
        let mut trigger = p["context"]["entries"][2].clone();
        trigger["sequence"] = json!(7);
        trigger["eventId"] = json!(uuid::Uuid::new_v4());
        p["triggerEventId"] = trigger["eventId"].clone();
        p["context"]["entries"] = json!([trigger]);
        p["context"]["activeDecisions"] = json!([{"decisionId":"eeeeeeee-eeee-4eee-8eee-eeeeeeeeeeee","title":"prior decision","summary":"still current","sourceEventIds":[old_id.clone()]}]);
        p["context"]["digest"] = json!(context_digest(p).unwrap());
        data.citation = Some(old_id.clone());
    }
    assert!(d.run_once().await.unwrap());
    assert_eq!(f.0.lock().await.sends, 1);
    drop(d);
    shutdown(handles).await;
    let (f, d, _, handles) = fixture_at(Some(path.clone())).await;
    {
        let mut data = f.0.lock().await;
        data.packet["taskId"] = json!(uuid::Uuid::new_v4());
        data.packet["conversation"]["generation"] = json!(2);
        data.packet["context"]["entries"][0]["eventId"] = json!(uuid::Uuid::new_v4());
        data.packet["context"]["activeDecisions"] = json!([]);
        data.packet["context"]["digest"] = json!(context_digest(&data.packet).unwrap());
        data.citation = Some(old_id);
    }
    assert!(d.run_once().await.is_err());
    assert!(f.0.lock().await.broker_ack.is_null());
    shutdown(handles).await;
    std::fs::remove_dir_all(path).unwrap();
}
#[tokio::test]
async fn completed_lost_ack_reconciles_without_second_send() {
    let _serial = SERIAL.lock().await;
    let (f, d, path, handles) = fixture().await;
    f.0.lock().await.lost_ack = true;
    assert!(d.run_once().await.is_err());
    assert!(d.run_once().await.unwrap());
    let data = f.0.lock().await;
    assert_eq!(data.sends, 1);
    assert_eq!(data.worker_acks, 1);
    assert!(
        data.updates
            .iter()
            .all(|v| !v.to_string().contains("native-"))
    );
    assert!(
        data.headers
            .iter()
            .all(|h| h == "Bearer distinct-worker-secret")
    );
    drop(data);
    std::fs::remove_dir_all(path).unwrap();
    for handle in handles {
        handle.abort();
        let _ = handle.await;
    }
}
#[tokio::test]
async fn uncertain_running_receipt_fails_closed() {
    let _serial = SERIAL.lock().await;
    let (f, d, path, handles) = fixture().await;
    {
        let mut data = f.0.lock().await;
        data.receipt = Some(receipt(&data.packet, "running"));
    }
    d.run_once().await.unwrap();
    let data = f.0.lock().await;
    assert_eq!(data.sends, 0);
    assert_eq!(data.cancels, 1);
    assert_eq!(
        data.updates.last().unwrap()["data"]["code"],
        "conversation_interrupted"
    );
    drop(data);
    std::fs::remove_dir_all(path).unwrap();
    for handle in handles {
        handle.abort();
        let _ = handle.await;
    }
}
#[tokio::test]
async fn lost_authority_cancels_and_rejects_late_output() {
    let _serial = SERIAL.lock().await;
    let (f, d, path, handles) = fixture().await;
    f.0.lock().await.slow = true;
    let revoke = f.clone();
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        revoke.0.lock().await.revoked = true
    });
    assert!(d.run_once().await.is_err());
    tokio::time::sleep(std::time::Duration::from_millis(600)).await;
    let data = f.0.lock().await;
    assert_eq!(data.sends, 1);
    assert_eq!(data.cancels, 1);
    assert!(data.broker_ack.is_null());
    assert!(data.updates.iter().all(|u| u["kind"] != "completed"));
    drop(data);
    std::fs::remove_dir_all(path).unwrap();
    for handle in handles {
        handle.abort();
        let _ = handle.await;
    }
}
#[tokio::test]
async fn runtime_turn_drift_never_reaches_broker() {
    let _serial = SERIAL.lock().await;
    let (f, d, path, handles) = fixture().await;
    f.0.lock().await.drift = true;
    assert!(d.run_once().await.is_err());
    let data = f.0.lock().await;
    assert!(data.broker_ack.is_null());
    assert!(data.updates.iter().all(|u| u["kind"] != "completed"));
    drop(data);
    std::fs::remove_dir_all(path).unwrap();
    for handle in handles {
        handle.abort();
        let _ = handle.await;
    }
}

#[tokio::test]
async fn catalog_bridge_authenticates_admits_and_exposes_only_one_bounded_page() {
    use tower::ServiceExt;
    let _serial = SERIAL.lock().await;
    let (_f, _d, path, handles) = fixture().await;
    let admission = Admission::new(
        "catalog-1".into(),
        "fixed-1".into(),
        [
            ("model-a".into(), vec!["effort-medium".into()]),
            ("model-b".into(), vec!["effort-medium".into()]),
        ]
        .into(),
    )
    .unwrap();
    let worker = WorkerClient::with_loopback_dns(
        "distinct-worker-secret".into(),
        admission,
        IpAddr::V4(Ipv4Addr::LOCALHOST),
    )
    .unwrap();
    let router = thought_khoral_agent_gateway::conversation_catalog::catalog_service(
        worker,
        "distinct-catalog-bridge-secret-32-chars".into(),
    )
    .unwrap();
    for (path, secret, status) in [
        (
            "/internal/agent-conversations/v1/models",
            "wrong",
            StatusCode::UNAUTHORIZED,
        ),
        (
            "/internal/agent-conversations/v1/models?cursor=anything",
            "distinct-catalog-bridge-secret-32-chars",
            StatusCode::BAD_REQUEST,
        ),
        (
            "/internal/agent-conversations/v1/tasks/anything",
            "distinct-catalog-bridge-secret-32-chars",
            StatusCode::NOT_FOUND,
        ),
        (
            "/internal/agent-conversations/v1/models",
            "distinct-catalog-bridge-secret-32-chars",
            StatusCode::OK,
        ),
    ] {
        let response = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri(path)
                    .header("authorization", format!("Bearer {secret}"))
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), status);
        if status == StatusCode::OK {
            let bytes = axum::body::to_bytes(response.into_body(), 1_048_576)
                .await
                .unwrap();
            let page: Value = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(page["nextCursor"], Value::Null);
            assert_eq!(page["data"].as_array().unwrap().len(), 1);
        }
    }
    for duplicate in [false, true] {
        let mut req = Request::builder()
            .uri("/internal/agent-conversations/v1/models")
            .header(
                "authorization",
                "Bearer distinct-catalog-bridge-secret-32-chars",
            );
        if duplicate {
            req = req.header(
                "authorization",
                "Bearer distinct-catalog-bridge-secret-32-chars",
            );
        }
        let response = router
            .clone()
            .oneshot(
                req.body(axum::body::Body::from(if duplicate {
                    ""
                } else {
                    "unexpected"
                }))
                .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            response.status(),
            if duplicate {
                StatusCode::UNAUTHORIZED
            } else {
                StatusCode::BAD_REQUEST
            }
        );
    }
    std::fs::remove_dir_all(path).unwrap();
    for h in handles {
        h.abort();
        let _ = h.await;
    }
}

#[tokio::test]
async fn expired_claim_never_sends_and_deadline_cancels_bound_work() {
    let _serial = SERIAL.lock().await;
    let (f, d, path, handles) = fixture().await;
    {
        let mut data = f.0.lock().await;
        data.packet["expiresAt"] =
            json!((chrono::Utc::now() - chrono::Duration::seconds(1)).to_rfc3339());
    }
    assert!(d.run_once().await.is_err());
    assert_eq!(f.0.lock().await.sends, 0);
    shutdown(handles).await;
    std::fs::remove_dir_all(path).unwrap();
    let (f, d, path, handles) = fixture().await;
    {
        let mut data = f.0.lock().await;
        data.packet["expiresAt"] =
            json!((chrono::Utc::now() + chrono::Duration::milliseconds(700)).to_rfc3339());
        data.slow = true;
    }
    assert!(d.run_once().await.is_err());
    tokio::time::sleep(std::time::Duration::from_millis(600)).await;
    let data = f.0.lock().await;
    assert_eq!(data.sends, 1);
    assert_eq!(data.cancels, 1);
    assert!(data.broker_ack.is_null());
    drop(data);
    shutdown(handles).await;
    std::fs::remove_dir_all(path).unwrap();
}

#[tokio::test]
async fn terminal_transport_failure_replays_exact_stored_update_without_worker_send() {
    let _serial = SERIAL.lock().await;
    let (f, d, path, handles) = fixture().await;
    f.0.lock().await.lose_before_accept = true;
    assert!(d.run_once().await.is_err());
    assert!(d.run_once().await.unwrap());
    let data = f.0.lock().await;
    assert_eq!(data.sends, 1);
    assert_eq!(data.updates.len(), 2);
    assert_eq!(data.updates[0], data.updates[1]);
    drop(data);
    shutdown(handles).await;
    std::fs::remove_dir_all(path).unwrap();
}
#[tokio::test]
async fn task_usage_artifact_and_card_redirect_drift_are_rejected() {
    let _serial = SERIAL.lock().await;
    for mode in ["task", "usage", "artifact", "redirect", "missing-model"] {
        let (f, d, path, handles) = fixture().await;
        f.0.lock().await.artifact_mode = Some(mode);
        assert!(d.run_once().await.is_err(), "{mode}");
        assert!(f.0.lock().await.broker_ack.is_null());
        shutdown(handles).await;
        std::fs::remove_dir_all(path).unwrap();
    }
}

#[tokio::test]
async fn obsolete_or_temporarily_unreachable_completion_never_starves_another_room() {
    let _serial = SERIAL.lock().await;
    for scenario in ["failed", "revoked", "expired", "transient"] {
        let (f, d, path, handles) = fixture().await;
        f.0.lock().await.lose_before_accept = true;
        if scenario == "expired" {
            f.0.lock().await.packet["expiresAt"] =
                json!((chrono::Utc::now() + chrono::Duration::milliseconds(700)).to_rfc3339());
        }
        assert!(d.run_once().await.is_err());
        if scenario == "expired" {
            tokio::time::sleep(std::time::Duration::from_millis(800)).await;
        }
        let old_id;
        {
            let mut data = f.0.lock().await;
            let old = data.packet.clone();
            old_id = old["taskId"].as_str().unwrap().to_owned();
            data.prior_receipts.insert(old_id.clone(),json!({"profileVersion":PROFILE,"taskId":old["taskId"],"conversationId":old["conversation"]["id"],"generation":old["conversation"]["generation"],"state":if scenario=="failed"{"failed"}else{"running"},"acknowledgement":null,"result":null}));
            data.prior_authority.insert(
                old_id.clone(),
                if scenario == "transient" {
                    StatusCode::SERVICE_UNAVAILABLE
                } else {
                    StatusCode::FORBIDDEN
                },
            );
            if scenario == "expired" {
                data.prior_authority_values.insert(old_id.clone(),json!({"profileVersion":PROFILE,"taskId":old["taskId"],"generation":old["conversation"]["generation"],"contextDigest":old["context"]["digest"],"expiresAt":old["expiresAt"],"authorizationExpiresAt":old["authorizationExpiresAt"]}));
            }
            let now = chrono::Utc::now();
            data.packet["issuedAt"] = json!(now.to_rfc3339());
            for k in ["expiresAt", "authorizationExpiresAt", "leaseExpiresAt"] {
                data.packet[k] = json!((now + chrono::Duration::seconds(180)).to_rfc3339());
            }
            data.packet["taskId"] = json!(uuid::Uuid::new_v4());
            data.packet["roomId"] = json!(uuid::Uuid::new_v4());
            data.packet["conversation"]["id"] = json!(uuid::Uuid::new_v4());
            data.packet["context"]["digest"] = json!(context_digest(&data.packet).unwrap());
            data.claimed = false;
            data.receipt = None;
        }
        assert!(d.run_once().await.unwrap(), "{scenario}");
        let data = f.0.lock().await;
        assert_eq!(data.sends, 2, "{scenario}");
        assert!(!data.broker_ack.is_null());
        assert_eq!(
            data.updates
                .iter()
                .filter(|u| u["taskId"] == old_id)
                .count(),
            1,
            "old result must not resend"
        );
        drop(data);
        let record: Value =
            serde_json::from_slice(&std::fs::read(path.join(format!("{old_id}.json"))).unwrap())
                .unwrap();
        assert_eq!(
            record["phase"],
            if scenario == "transient" {
                "completed"
            } else {
                "quarantined"
            }
        );
        shutdown(handles).await;
        std::fs::remove_dir_all(path).unwrap();
    }
}

#[tokio::test]
async fn bound_actual_settings_and_usage_are_admission_checked_without_changing_selected_defaults()
{
    let _serial = SERIAL.lock().await;
    for scenario in ["admitted-change", "unknown-model", "wrong-usage-model"] {
        let (f, d, path, handles) = fixture().await;
        {
            let mut data = f.0.lock().await;
            data.actual_settings = json!({"model":if scenario=="unknown-model"{"unapproved"}else{"model-b"},"reasoningEffort":"effort-medium","confirmation":"confirmed","reroutedModel":null});
            data.actual_usage = if scenario == "unknown-model" {
                Value::Null
            } else {
                json!({"lastTotalTokens":1,"modelContextWindow":1000,"reportedAt":chrono::Utc::now().to_rfc3339(),"model":if scenario=="wrong-usage-model"{"model-a"}else{"model-b"},"freshness":"fresh"})
            };
        }
        let result = d.run_once().await;
        let data = f.0.lock().await;
        if scenario == "admitted-change" {
            assert!(result.unwrap());
            assert_eq!(
                data.updates.last().unwrap()["data"]["effectiveSettings"]["model"],
                "model-b"
            );
            assert_eq!(
                data.updates.last().unwrap()["data"]["usage"]["model"],
                "model-b"
            );
            assert_eq!(data.packet["model"], "model-a");
        } else {
            assert!(result.is_err());
            assert!(data.broker_ack.is_null());
        }
        assert_eq!(data.sends, 1);
        drop(data);
        shutdown(handles).await;
        std::fs::remove_dir_all(path).unwrap();
    }
}
