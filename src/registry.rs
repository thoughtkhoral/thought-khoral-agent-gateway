use std::collections::BTreeSet;

use a2a::AgentCard;
use thiserror::Error;
use url::Url;
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

#[derive(Debug, Error)]
pub enum RegistryError {
    #[error("Agent Card name differs from the pinned Reference Agent identity")]
    IdentityDrift,
    #[error("Agent Card skills differ from the two pinned Reference Agent skills")]
    SkillDrift,
    #[error("Agent Card endpoint, binding, or protocol version differs from the pinned value")]
    EndpointDrift,
}
