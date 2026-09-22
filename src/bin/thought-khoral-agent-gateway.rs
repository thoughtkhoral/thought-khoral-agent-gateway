use std::error::Error;

use a2a_client::agent_card::AgentCardResolver;
use thought_khoral_agent_gateway::{
    GatewayConfig, RegisteredAgent, RoomGatewayClient, reference_registration,
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let config = GatewayConfig::from_process_env()?;

    // Agent Card discovery is strictly local and is admitted only after every
    // fixed identity, skill, endpoint, binding, and protocol value is pinned.
    let resolver = AgentCardResolver::new(None);
    let card = resolver
        .resolve(config.reference_agent_base_url().as_str())
        .await?;
    let registered_agent = RegisteredAgent::from_pinned_card(reference_registration(), card)
        .map(|agent| agent.with_allowed_handoff_hosts(config.allowed_handoff_hosts.clone()))?;
    let _room_gateway = RoomGatewayClient::from_config(&config)?;

    println!(
        "validated pinned local Agent Card for {} ({})",
        registered_agent.display_name, registered_agent.id
    );
    // Dispatcher, reference-agent server, and A2A task invocation deliberately
    // start in their later slices.
    Ok(())
}
