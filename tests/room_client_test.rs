use std::collections::BTreeMap;

use thought_khoral_agent_gateway::{GatewayConfig, RoomGatewayClient};

#[test]
fn room_client_can_only_be_constructed_from_a_trusted_gateway_config() {
    let config = GatewayConfig::parse(test_env()).expect("trusted local development origin");
    assert!(RoomGatewayClient::from_config(&config).is_ok());
}

fn test_env() -> BTreeMap<String, String> {
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
        (
            "THOUGHT_KHORAL_REFERENCE_AGENT_INBOUND_SECRET",
            "test-reference-agent-inbound-secret",
        ),
        (
            "THOUGHT_KHORAL_REFERENCE_AGENT_CARD_URL",
            "http://127.0.0.1:9090",
        ),
        ("THOUGHT_KHORAL_ALLOWED_HANDOFF_HOSTS", "allowed.example"),
        (
            "THOUGHT_KHORAL_REFERENCE_AGENT_HANDOFF_HOST",
            "allowed.example",
        ),
        ("THOUGHT_KHORAL_AGENT_POLL_MILLIS", "1000"),
    ]
    .into_iter()
    .map(|(key, value)| (key.to_owned(), value.to_owned()))
    .collect()
}
