use std::collections::BTreeMap;

use thought_khoral_agent_gateway::GatewayConfig;

#[test]
fn config_rejects_non_loopback_agent_or_unallowlisted_handoff_host() {
    assert!(
        GatewayConfig::parse(test_env("https://remote-agent.invalid", "allowed.example")).is_err()
    );
    assert!(
        GatewayConfig::parse(test_env("http://127.0.0.1:9090", "unregistered.example")).is_err()
    );
}

#[test]
fn config_rejects_unpinned_https_room_gateway_and_ip_handoff_hosts() {
    let mut remote_room_gateway = test_env("http://127.0.0.1:9090", "allowed.example");
    remote_room_gateway.insert(
        "THOUGHT_KHORAL_ROOM_GATEWAY_ORIGIN".into(),
        "https://room-gateway.attacker.invalid".into(),
    );
    assert!(GatewayConfig::parse(remote_room_gateway).is_err());

    let mut ip_handoff = test_env("http://127.0.0.1:9090", "127.0.0.1");
    ip_handoff.insert(
        "THOUGHT_KHORAL_ALLOWED_HANDOFF_HOSTS".into(),
        "127.0.0.1".into(),
    );
    assert!(GatewayConfig::parse(ip_handoff).is_err());

    let mut noncanonical_ip_handoff = test_env("http://127.0.0.1:9090", "127.000.0.1");
    noncanonical_ip_handoff.insert(
        "THOUGHT_KHORAL_ALLOWED_HANDOFF_HOSTS".into(),
        "127.000.0.1".into(),
    );
    assert!(GatewayConfig::parse(noncanonical_ip_handoff).is_err());
}

#[test]
fn config_requires_service_credentials_and_short_operational_limits() {
    let config = GatewayConfig::parse(test_env("http://127.0.0.1:9090", "allowed.example"))
        .expect("the pinned local reference-agent configuration is valid");

    assert_eq!(
        config.room_gateway_origin().as_str(),
        "http://127.0.0.1:8080/"
    );
    assert_eq!(config.lease_seconds(), 120);
    assert_eq!(config.poll_millis(), 1_000);
    assert_eq!(config.update_rate_per_minute(), 30);
    assert!(config.allowed_handoff_hosts().contains("allowed.example"));
}

// Compose and Kubernetes use these reviewed service identities. This fails if
// an otherwise arbitrary cleartext token endpoint becomes trusted.
#[test]
fn config_accepts_only_reviewed_internal_keycloak_http_origins() {
    for token_url in [
        "http://thought-khoral-keycloak:8080/realms/thought-khoral/protocol/openid-connect/token",
        "http://thought-khoral-keycloak.thought-khoral-dev.svc.cluster.local:8080/realms/thought-khoral/protocol/openid-connect/token",
    ] {
        let mut environment = test_env("http://127.0.0.1:9090", "allowed.example");
        environment.insert("THOUGHT_KHORAL_KEYCLOAK_TOKEN_URL".into(), token_url.into());
        assert!(GatewayConfig::parse(environment).is_ok(), "{token_url}");
    }

    let mut unreviewed = test_env("http://127.0.0.1:9090", "allowed.example");
    unreviewed.insert(
        "THOUGHT_KHORAL_KEYCLOAK_TOKEN_URL".into(),
        "http://keycloak.attacker.invalid/realms/thought-khoral/protocol/openid-connect/token"
            .into(),
    );
    assert!(GatewayConfig::parse(unreviewed).is_err());
}

fn test_env(card_url: &str, selected_handoff_host: &str) -> BTreeMap<String, String> {
    [
        (
            "THOUGHT_KHORAL_ROOM_GATEWAY_ORIGIN",
            "http://127.0.0.1:8080",
        ),
        (
            "THOUGHT_KHORAL_KEYCLOAK_TOKEN_URL",
            "http://127.0.0.1:8081/realms/thought-khoral/protocol/openid-connect/token",
        ),
        (
            "THOUGHT_KHORAL_AGENT_GATEWAY_CLIENT_ID",
            "thought-khoral-agent-gateway",
        ),
        ("THOUGHT_KHORAL_AGENT_GATEWAY_CLIENT_SECRET", "test-secret"),
        ("THOUGHT_KHORAL_REFERENCE_AGENT_CARD_URL", card_url),
        ("THOUGHT_KHORAL_ALLOWED_HANDOFF_HOSTS", "allowed.example"),
        (
            "THOUGHT_KHORAL_REFERENCE_AGENT_HANDOFF_HOST",
            selected_handoff_host,
        ),
        ("THOUGHT_KHORAL_AGENT_LEASE_SECONDS", "120"),
        ("THOUGHT_KHORAL_AGENT_POLL_MILLIS", "1000"),
        ("THOUGHT_KHORAL_AGENT_UPDATE_RATE_PER_MINUTE", "30"),
    ]
    .into_iter()
    .map(|(key, value)| (key.to_owned(), value.to_owned()))
    .collect()
}
