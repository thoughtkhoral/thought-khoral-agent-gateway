use std::collections::{BTreeMap, BTreeSet};

use thiserror::Error;
use url::{Host, Url};

const REQUIRED_CLIENT_ID: &str = "thought-khoral-agent-gateway";
const FIXED_REFERENCE_AGENT_BASE_URL: &str = "http://127.0.0.1:9090/";
const FIXED_REFERENCE_AGENT_CARD_URL: &str = "http://127.0.0.1:9090/.well-known/agent-card.json";
const TRUSTED_ROOM_GATEWAY_ORIGINS: [&str; 3] = [
    "http://127.0.0.1:8080/",
    "http://thought-khoral-room-gateway:8080/",
    "http://thought-khoral-room-gateway.thought-khoral-dev.svc.cluster.local:8080/",
];
const TRUSTED_KEYCLOAK_TOKEN_URLS: [&str; 2] = [
    "http://thought-khoral-keycloak:8080/realms/thought-khoral/protocol/openid-connect/token",
    "http://thought-khoral-keycloak.thought-khoral-dev.svc.cluster.local:8080/realms/thought-khoral/protocol/openid-connect/token",
];

/// A room-gateway authority reviewed into this crate. It cannot be constructed
/// from arbitrary configuration outside this module.
#[derive(Clone, Debug)]
pub struct RoomGatewayOrigin(Url);

impl RoomGatewayOrigin {
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }

    pub(crate) fn as_url(&self) -> &Url {
        &self.0
    }
}

#[derive(Clone)]
pub struct ClientCredentialsConfig {
    token_url: Url,
    client_id: String,
    client_secret: String,
}

impl ClientCredentialsConfig {
    pub(crate) fn token_url(&self) -> &Url {
        &self.token_url
    }

    pub(crate) fn client_id(&self) -> &str {
        &self.client_id
    }

    pub(crate) fn client_secret(&self) -> &str {
        &self.client_secret
    }
}

impl std::fmt::Debug for ClientCredentialsConfig {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ClientCredentialsConfig")
            .field("token_url", &self.token_url)
            .field("client_id", &self.client_id)
            .field("client_secret", &"[REDACTED]")
            .finish()
    }
}

#[derive(Clone, Debug)]
pub struct GatewayConfig {
    room_gateway_origin: RoomGatewayOrigin,
    client_credentials: ClientCredentialsConfig,
    allowed_handoff_hosts: BTreeSet<String>,
    lease_seconds: u64,
    poll_millis: u64,
    update_rate_per_minute: u32,
}

impl GatewayConfig {
    /// Parses the deployment environment without reading a browser or user
    /// credential. The fixed reference-agent card is local-only.
    pub fn parse(environment: BTreeMap<String, String>) -> Result<Self, ConfigError> {
        let room_gateway_origin = parse_trusted_room_gateway_origin(required(
            &environment,
            "THOUGHT_KHORAL_ROOM_GATEWAY_ORIGIN",
        )?)?;
        let token_url =
            parse_token_url(required(&environment, "THOUGHT_KHORAL_KEYCLOAK_TOKEN_URL")?)?;
        let client_id = required(&environment, "THOUGHT_KHORAL_AGENT_GATEWAY_CLIENT_ID")?;
        if client_id != REQUIRED_CLIENT_ID {
            return Err(ConfigError::InvalidClientId);
        }
        let client_secret = required(&environment, "THOUGHT_KHORAL_AGENT_GATEWAY_CLIENT_SECRET")?;
        if client_secret.is_empty() {
            return Err(ConfigError::EmptyClientSecret);
        }

        parse_fixed_card_url(required(
            &environment,
            "THOUGHT_KHORAL_REFERENCE_AGENT_CARD_URL",
        )?)?;
        let allowed_handoff_hosts = parse_handoff_hosts(required(
            &environment,
            "THOUGHT_KHORAL_ALLOWED_HANDOFF_HOSTS",
        )?)?;
        let selected_handoff_host =
            required(&environment, "THOUGHT_KHORAL_REFERENCE_AGENT_HANDOFF_HOST")?;
        if !allowed_handoff_hosts.contains(selected_handoff_host) {
            return Err(ConfigError::UnallowlistedHandoffHost(
                selected_handoff_host.to_owned(),
            ));
        }

        let lease_seconds =
            parse_bounded_u64(&environment, "THOUGHT_KHORAL_AGENT_LEASE_SECONDS", 30, 300)?;
        let poll_millis =
            parse_bounded_u64(&environment, "THOUGHT_KHORAL_AGENT_POLL_MILLIS", 100, 5_000)?;
        let update_rate_per_minute = parse_bounded_u64(
            &environment,
            "THOUGHT_KHORAL_AGENT_UPDATE_RATE_PER_MINUTE",
            1,
            60,
        )? as u32;

        Ok(Self {
            room_gateway_origin,
            client_credentials: ClientCredentialsConfig {
                token_url,
                client_id: client_id.to_owned(),
                client_secret: client_secret.to_owned(),
            },
            allowed_handoff_hosts,
            lease_seconds,
            poll_millis,
            update_rate_per_minute,
        })
    }

    pub fn from_process_env() -> Result<Self, ConfigError> {
        Self::parse(std::env::vars().collect())
    }

    pub fn room_gateway_origin(&self) -> &RoomGatewayOrigin {
        &self.room_gateway_origin
    }

    pub fn allowed_handoff_hosts(&self) -> &BTreeSet<String> {
        &self.allowed_handoff_hosts
    }

    pub fn lease_seconds(&self) -> u64 {
        self.lease_seconds
    }

    pub fn poll_millis(&self) -> u64 {
        self.poll_millis
    }

    pub fn update_rate_per_minute(&self) -> u32 {
        self.update_rate_per_minute
    }

    pub(crate) fn client_credentials(&self) -> &ClientCredentialsConfig {
        &self.client_credentials
    }
}

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("required configuration {0} is missing")]
    Missing(&'static str),
    #[error("room gateway origin is not one of the reviewed internal authorities")]
    InvalidRoomGatewayOrigin,
    #[error(
        "Keycloak token URL must be HTTP(S), credential-free, and use HTTPS outside loopback except for reviewed internal service origins"
    )]
    InvalidTokenUrl,
    #[error("only the thought-khoral-agent-gateway Keycloak client is accepted")]
    InvalidClientId,
    #[error("the Keycloak client secret must not be empty")]
    EmptyClientSecret,
    #[error(
        "reference agent card must be the pinned loopback card {FIXED_REFERENCE_AGENT_CARD_URL}"
    )]
    InvalidReferenceAgentCard,
    #[error("allowed handoff hosts must be non-empty, canonical DNS names")]
    InvalidHandoffHosts,
    #[error("reference-agent handoff host {0} is not in the configured allowlist")]
    UnallowlistedHandoffHost(String),
    #[error("{key} must be an integer from {minimum} through {maximum}")]
    InvalidBoundedInteger {
        key: &'static str,
        minimum: u64,
        maximum: u64,
    },
}

fn required<'a>(
    environment: &'a BTreeMap<String, String>,
    key: &'static str,
) -> Result<&'a str, ConfigError> {
    environment
        .get(key)
        .map(String::as_str)
        .filter(|value| !value.is_empty())
        .ok_or(ConfigError::Missing(key))
}

fn parse_trusted_room_gateway_origin(value: &str) -> Result<RoomGatewayOrigin, ConfigError> {
    let origin = Url::parse(value).map_err(|_| ConfigError::InvalidRoomGatewayOrigin)?;
    if !is_exact_origin(&origin)
        || !matches!(origin.scheme(), "http" | "https")
        || !TRUSTED_ROOM_GATEWAY_ORIGINS
            .iter()
            .any(|trusted| Url::parse(trusted).expect("valid trusted URL") == origin)
    {
        return Err(ConfigError::InvalidRoomGatewayOrigin);
    }
    Ok(RoomGatewayOrigin(origin))
}

fn parse_token_url(value: &str) -> Result<Url, ConfigError> {
    let url = Url::parse(value).map_err(|_| ConfigError::InvalidTokenUrl)?;
    let reviewed_internal_service = TRUSTED_KEYCLOAK_TOKEN_URLS
        .iter()
        .any(|trusted| Url::parse(trusted).expect("valid trusted URL") == url);
    if !matches!(url.scheme(), "http" | "https")
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || url.host().is_none()
        || (url.scheme() == "http" && !is_loopback(&url) && !reviewed_internal_service)
    {
        return Err(ConfigError::InvalidTokenUrl);
    }
    Ok(url)
}

fn parse_fixed_card_url(value: &str) -> Result<Url, ConfigError> {
    let value = Url::parse(value).map_err(|_| ConfigError::InvalidReferenceAgentCard)?;
    let fixed_base = Url::parse(FIXED_REFERENCE_AGENT_BASE_URL).expect("valid fixed URL");
    let fixed_card = Url::parse(FIXED_REFERENCE_AGENT_CARD_URL).expect("valid fixed URL");
    if value == fixed_base || value == fixed_card {
        Ok(fixed_card)
    } else {
        Err(ConfigError::InvalidReferenceAgentCard)
    }
}

fn parse_handoff_hosts(value: &str) -> Result<BTreeSet<String>, ConfigError> {
    let hosts = value
        .split(',')
        .map(str::trim)
        .filter(|host| !host.is_empty())
        .map(str::to_ascii_lowercase)
        .collect::<BTreeSet<_>>();
    if hosts.is_empty() || hosts.iter().any(|host| !is_canonical_host(host)) {
        return Err(ConfigError::InvalidHandoffHosts);
    }
    Ok(hosts)
}

fn is_canonical_host(host: &str) -> bool {
    host.len() <= 253
        && host.parse::<std::net::IpAddr>().is_err()
        && !is_dotted_numeric_literal(host)
        && host.contains('.')
        && host.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        })
}

fn is_dotted_numeric_literal(host: &str) -> bool {
    host.contains('.')
        && host
            .split('.')
            .all(|label| !label.is_empty() && label.bytes().all(|byte| byte.is_ascii_digit()))
}

fn parse_bounded_u64(
    environment: &BTreeMap<String, String>,
    key: &'static str,
    minimum: u64,
    maximum: u64,
) -> Result<u64, ConfigError> {
    let value = required(environment, key)?;
    let parsed = value.parse::<u64>().ok();
    match parsed.filter(|parsed| (*parsed >= minimum) && (*parsed <= maximum)) {
        Some(parsed) => Ok(parsed),
        None => Err(ConfigError::InvalidBoundedInteger {
            key,
            minimum,
            maximum,
        }),
    }
}

fn is_exact_origin(url: &Url) -> bool {
    url.username().is_empty()
        && url.password().is_none()
        && url.query().is_none()
        && url.fragment().is_none()
        && matches!(url.path(), "" | "/")
        && url.host().is_some()
}

fn is_loopback(url: &Url) -> bool {
    match url.host() {
        Some(Host::Ipv4(address)) => address.is_loopback(),
        Some(Host::Ipv6(address)) => address.is_loopback(),
        Some(Host::Domain(host)) => host.eq_ignore_ascii_case("localhost"),
        None => false,
    }
}
