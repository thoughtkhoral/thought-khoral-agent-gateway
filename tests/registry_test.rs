use a2a::{AgentCapabilities, AgentCard, AgentInterface, AgentSkill};
use thought_khoral_agent_gateway::{RegisteredAgent, reference_registration};

#[test]
fn registry_rejects_card_endpoint_or_skill_drift() {
    let mut card = fixed_reference_card();
    card.skills.clear();
    assert!(RegisteredAgent::from_pinned_card(reference_registration(), card).is_err());

    let mut card = fixed_reference_card();
    card.supported_interfaces[0].url = "http://127.0.0.1:9999/jsonrpc".into();
    assert!(RegisteredAgent::from_pinned_card(reference_registration(), card).is_err());
}

#[test]
fn registry_accepts_only_the_fixed_reference_agent_identity_and_two_skills() {
    let agent = RegisteredAgent::from_pinned_card(reference_registration(), fixed_reference_card())
        .expect("the fixed local Agent Card is registered");

    assert_eq!(agent.id.to_string(), "74686f75-6768-746b-686f-72616c000003");
    assert_eq!(agent.display_name, "Reference Agent");
    assert_eq!(agent.allowed_skills.len(), 2);
    assert!(agent.allowed_skills.contains("summarize-context"));
    assert!(agent.allowed_skills.contains("extract-action-items"));
}

fn fixed_reference_card() -> AgentCard {
    AgentCard {
        name: "Reference Agent".into(),
        description: "Deterministic ThoughtKhoral reference agent".into(),
        version: "1.0.0".into(),
        supported_interfaces: vec![AgentInterface::new(
            "http://127.0.0.1:9090/jsonrpc",
            "JSONRPC",
        )],
        capabilities: AgentCapabilities::default(),
        default_input_modes: vec!["text/plain".into()],
        default_output_modes: vec!["text/plain".into()],
        skills: vec![
            AgentSkill {
                id: "summarize-context".into(),
                name: "Summarize context".into(),
                description: "Produce a deterministic context summary".into(),
                tags: vec!["thought-khoral".into()],
                examples: None,
                input_modes: None,
                output_modes: None,
                security_requirements: None,
            },
            AgentSkill {
                id: "extract-action-items".into(),
                name: "Extract action items".into(),
                description: "Extract explicit action items".into(),
                tags: vec!["thought-khoral".into()],
                examples: None,
                input_modes: None,
                output_modes: None,
                security_requirements: None,
            },
        ],
        provider: None,
        documentation_url: None,
        icon_url: None,
        security_schemes: None,
        security_requirements: None,
        signatures: None,
    }
}
