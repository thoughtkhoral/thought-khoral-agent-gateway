use std::error::Error;

use thought_khoral_agent_gateway::{
    GatewayConfig, RegisteredAgent, RoomGatewayClient, reference_registration,
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let config = GatewayConfig::from_process_env()?;

    // Agent Card discovery is strictly local and is admitted only after every
    // fixed identity, skill, endpoint, binding, and protocol value is pinned.
    let registration = reference_registration();
    let card = registration.resolve_pinned_card().await?;
    let registered_agent = RegisteredAgent::from_pinned_card(registration, card)
        .map(|agent| agent.with_allowed_handoff_hosts(config.allowed_handoff_hosts().clone()))?;
    let _room_gateway = RoomGatewayClient::from_config(&config)?;

    println!(
        "validated pinned local Agent Card for {} ({})",
        registered_agent.display_name, registered_agent.id
    );
    // Dispatcher, reference-agent server, and A2A task invocation deliberately
    // start in their later slices.
    Ok(())
}
