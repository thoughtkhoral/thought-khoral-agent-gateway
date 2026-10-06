use serde_json::{Value, json};
use thought_khoral_agent_gateway::conversation_validation::{
    Admission, canonical_bytes, context_digest, parse_json, validate_card, validate_packet,
    validate_result,
};
fn packet() -> Value {
    serde_json::from_str(include_str!(
        "../contracts/agent-conversation-v1/fixtures/valid/baseline-hidden-sequence-gap.json"
    ))
    .unwrap()
}
fn admission() -> Admission {
    Admission::new(
        "catalog-1".into(),
        "fixed-1".into(),
        [("model-a".into(), vec!["effort-medium".into()])].into(),
    )
    .unwrap()
}
#[test]
fn rejects_duplicate_keys_float_and_unsafe_integer() {
    for s in [
        r#"{"a":1,"a":2}"#,
        r#"{"a":1.0}"#,
        r#"{"a":9007199254740992}"#,
        r#"{"a":-0}"#,
    ] {
        assert!(parse_json(s.as_bytes()).is_err(), "{s}");
    }
}
#[test]
fn canonical_unicode_matches_published_vector() {
    let v: Value = serde_json::from_str(include_str!(
        "../contracts/agent-conversation-v1/fixtures/valid/unicode-canonical-vector.json"
    ))
    .unwrap();
    assert_eq!(
        String::from_utf8(canonical_bytes(&v["value"]).unwrap()).unwrap(),
        v["canonical"]
    );
}
#[test]
fn packet_rejects_digest_generation_and_model_drift() {
    let mut p = packet();
    let a = admission();
    validate_packet(&p, &a, false).unwrap();
    p["model"] = json!("unapproved");
    assert!(validate_packet(&p, &a, false).is_err());
    let mut p = packet();
    p["conversation"]["generation"] = json!(2);
    assert!(validate_packet(&p, &a, false).is_err());
    let mut p = packet();
    p["context"]["digest"] = json!("a".repeat(64));
    assert!(validate_packet(&p, &a, false).is_err());
}
#[test]
fn result_rejects_unknown_citations_settings_and_crossbindings() {
    let p = packet();
    let mut r = json!({"kind":"conversation-reply.v1","conversationId":p["conversation"]["id"],"generation":p["conversation"]["generation"],"assistantText":"done","consumedRevision":p["context"]["revision"],"contextDigest":p["context"]["digest"],"citations":[],"effectiveSettings":null,"usage":null});
    validate_result(&r, &p, &Default::default()).unwrap();
    for field in ["generation", "consumedRevision"] {
        let mut bad = r.clone();
        bad[field] = json!(999);
        assert!(validate_result(&bad, &p, &Default::default()).is_err());
    }
    r["citations"] = json!(["ffffffff-ffff-4fff-8fff-ffffffffffff"]);
    assert!(validate_result(&r, &p, &Default::default()).is_err());
    r["citations"] = json!([]);
    r["usage"] = json!({"lastTotalTokens":1,"modelContextWindow":null,"reportedAt":"2026-10-05T12:00:05Z","model":"model-a","freshness":"fresh"});
    assert!(validate_result(&r, &p, &Default::default()).is_err());
    r["usage"] = Value::Null;
    r["threadId"] = json!("private");
    assert!(validate_result(&r, &p, &Default::default()).is_err());
}
#[test]
fn card_admission_is_exact() {
    let c = thought_khoral_agent_gateway::registry::codex_card();
    validate_card(&c).unwrap();
    for (pointer, value) in [
        ("/name", json!("other")),
        ("/skills/0/id", json!("other")),
        ("/supportedInterfaces/0/url", json!("http://evil.invalid")),
        ("/capabilities/streaming", json!(true)),
        ("/capabilities/extensions/0/uri", json!("unknown")),
    ] {
        let mut bad = c.clone();
        *bad.pointer_mut(pointer).unwrap() = value;
        assert!(validate_card(&bad).is_err(), "{pointer}");
    }
}
#[test]
fn context_digest_detects_changed_transcript() {
    let mut p = packet();
    assert_eq!(context_digest(&p).unwrap(), p["context"]["digest"]);
    p["context"]["entries"][0]["text"] = json!("changed");
    assert_ne!(context_digest(&p).unwrap(), p["context"]["digest"]);
}

#[test]
fn nullable_unconfirmed_settings_and_separate_reroute_metadata_are_accepted() {
    let p = packet();
    let mut r: Value = serde_json::from_str(include_str!(
        "../contracts/agent-conversation-v1/fixtures/valid/result-last-usage.json"
    ))
    .unwrap();
    r["effectiveSettings"] = json!({"model":"model-a","reasoningEffort":null,"confirmation":"unconfirmed","reroutedModel":null});
    validate_result(&r, &p, &Default::default()).unwrap();
    r["effectiveSettings"]["reroutedModel"] = json!("reported-runtime-model");
    r["usage"] = Value::Null;
    validate_result(&r, &p, &Default::default()).unwrap();
    r["effectiveSettings"]["confirmation"] = json!("confirmed");
    assert!(validate_result(&r, &p, &Default::default()).is_err());
    r["effectiveSettings"]["reasoningEffort"] = p["reasoningEffort"].clone();
    r["effectiveSettings"]["model"] = json!("unapproved-model");
    assert!(validate_result(&r, &p, &Default::default()).is_err());
}
