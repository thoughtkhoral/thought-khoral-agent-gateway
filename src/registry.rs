use std::collections::BTreeSet;

use a2a::AgentCard;
use a2a_client::agent_card::AgentCardResolver;
use reqwest::redirect::Policy;
use thiserror::Error;
use url::{Host, Url};
use uuid::Uuid;

pub type SkillId = String;

const REFERENCE_AGENT_ID: Uuid = Uuid::from_u128(0x74686f756768746b_686f72616c000003);
const REFERENCE_AGENT_NAME: &str = "Reference Agent";
const REFERENCE_CARD_URL: &str = "http://127.0.0.1:9090/.well-known/agent-card.json";
const REFERENCE_ENDPOINT: &str = "http://127.0.0.1:9090/jsonrpc";
const REFERENCE_BINDING: &str = "JSONRPC";
const REFERENCE_PROTOCOL_VERSION: &str = "1.0";
const REFERENCE_SKILLS: [&str; 2] = ["extract-action-items", "summarize-context"];

#[derive(Clone, Debug)]
pub struct RegisteredAgent {
    pub id: Uuid,
    pub display_name: String,
    pub card_url: Url,
    pub allowed_skills: BTreeSet<SkillId>,
    pub allowed_handoff_hosts: BTreeSet<String>,
}

#[derive(Clone, Debug)]
pub struct PinnedAgentRegistration {
    id: Uuid,
    display_name: String,
    card_url: Url,
    endpoint: Url,
    binding: String,
    protocol_version: String,
    allowed_skills: BTreeSet<SkillId>,
}

pub fn reference_registration() -> PinnedAgentRegistration {
    PinnedAgentRegistration {
        id: REFERENCE_AGENT_ID,
        display_name: REFERENCE_AGENT_NAME.to_owned(),
        card_url: Url::parse(REFERENCE_CARD_URL).expect("valid fixed Agent Card URL"),
        endpoint: Url::parse(REFERENCE_ENDPOINT).expect("valid fixed Agent endpoint"),
        binding: REFERENCE_BINDING.to_owned(),
        protocol_version: REFERENCE_PROTOCOL_VERSION.to_owned(),
        allowed_skills: REFERENCE_SKILLS.into_iter().map(str::to_owned).collect(),
    }
}

impl RegisteredAgent {
    /// Admits a card only if every pinned identity and transport property
    /// matches. Discovery supplies metadata, never authority.
    pub fn from_pinned_card(
        registration: PinnedAgentRegistration,
        card: AgentCard,
    ) -> Result<Self, RegistryError> {
        if card.name != registration.display_name {
            return Err(RegistryError::IdentityDrift);
        }

        let card_skills = card
            .skills
            .iter()
            .map(|skill| skill.id.clone())
            .collect::<BTreeSet<_>>();
        if card_skills != registration.allowed_skills || card.skills.len() != card_skills.len() {
            return Err(RegistryError::SkillDrift);
        }

        if card.supported_interfaces.len() != 1 {
            return Err(RegistryError::EndpointDrift);
        }
        let interface = &card.supported_interfaces[0];
        let endpoint = Url::parse(&interface.url).map_err(|_| RegistryError::EndpointDrift)?;
        if endpoint != registration.endpoint
            || interface.protocol_binding != registration.binding
            || interface.protocol_version != registration.protocol_version
        {
            return Err(RegistryError::EndpointDrift);
        }

        Ok(Self {
            id: registration.id,
            display_name: registration.display_name,
            card_url: registration.card_url,
            allowed_skills: registration.allowed_skills,
            allowed_handoff_hosts: BTreeSet::new(),
        })
    }

    pub fn with_allowed_handoff_hosts(mut self, allowed_handoff_hosts: BTreeSet<String>) -> Self {
        self.allowed_handoff_hosts = allowed_handoff_hosts;
        self
    }
}

impl PinnedAgentRegistration {
    /// Resolves only the registration's loopback Agent Card. The registration
    /// has no public constructor or mutable fields, so callers cannot turn
    /// this discovery path into arbitrary egress.
    pub async fn resolve_pinned_card(&self) -> Result<AgentCard, RegistryError> {
        if !is_loopback_card_url(&self.card_url) {
            return Err(RegistryError::NonLoopbackCard);
        }
        let client = reqwest::Client::builder()
            .no_proxy()
            .redirect(Policy::none())
            .build()
            .map_err(|error| RegistryError::CardDiscovery(error.to_string()))?;
        let resolver = AgentCardResolver::new(Some(client));
        resolver
            .resolve(card_base_url(&self.card_url).as_str())
            .await
            .map_err(|error| RegistryError::CardDiscovery(error.to_string()))
    }
}

fn is_loopback_card_url(url: &Url) -> bool {
    url.scheme() == "http"
        && url.username().is_empty()
        && url.password().is_none()
        && url.query().is_none()
        && url.fragment().is_none()
        && url.path() == "/.well-known/agent-card.json"
        && match url.host() {
            Some(Host::Ipv4(address)) => address.is_loopback(),
            Some(Host::Ipv6(address)) => address.is_loopback(),
            Some(Host::Domain(host)) => host.eq_ignore_ascii_case("localhost"),
            None => false,
        }
}

fn card_base_url(card_url: &Url) -> Url {
    let mut base = card_url.clone();
    base.set_path("");
    base.set_query(None);
    base.set_fragment(None);
    base
}

#[derive(Debug, Error)]
pub enum RegistryError {
    #[error("Agent Card name differs from the pinned Reference Agent identity")]
    IdentityDrift,
    #[error("Agent Card skills differ from the two pinned Reference Agent skills")]
    SkillDrift,
    #[error("Agent Card endpoint, binding, or protocol version differs from the pinned value")]
    EndpointDrift,
    #[error("Agent Card discovery is restricted to loopback")]
    NonLoopbackCard,
    #[error("pinned Agent Card discovery failed: {0}")]
    CardDiscovery(String),
}

#[cfg(test)]
mod tests {
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
    };

    use super::*;

    #[tokio::test]
    async fn pinned_card_discovery_rejects_nonloopback_before_fetch() {
        let mut registration = reference_registration();
        registration.card_url =
            Url::parse("https://remote-agent.invalid/.well-known/agent-card.json").unwrap();

        assert!(registration.resolve_pinned_card().await.is_err());
    }

    #[tokio::test]
    async fn pinned_card_discovery_rejects_redirects() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0_u8; 4_096];
            let _ = socket.read(&mut request).await.unwrap();
            socket
                .write_all(
                    b"HTTP/1.1 302 Found\r\nlocation: http://remote-agent.invalid/card\r\ncontent-length: 0\r\nconnection: close\r\n\r\n",
                )
                .await
                .unwrap();
        });

        let mut registration = reference_registration();
        registration.card_url =
            Url::parse(&format!("http://{address}/.well-known/agent-card.json")).unwrap();

        assert!(registration.resolve_pinned_card().await.is_err());
    }
}
