use std::{error::Error, time::Duration};

use thought_khoral_agent_gateway::{
    GatewayConfig, RegisteredAgent, RoomGatewayClient, a2a_adapter::A2aAdapter,
    dispatcher::Dispatcher,
};
use uuid::Uuid;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let config = GatewayConfig::from_process_env()?;

    let bearer_secret = std::env::var("THOUGHT_KHORAL_REFERENCE_AGENT_INBOUND_SECRET")?;
    // Discovery and invocation use only the pinned loopback URL and the
    // dedicated inbound secret. The server validates the same secret. This is
    // deliberately distinct from the Keycloak client credential in `config`.
    let adapter = A2aAdapter::new(bearer_secret)?;
    let registered_agent: RegisteredAgent = adapter
        .resolve_pinned_card()
        .await?
        .with_allowed_handoff_hosts(config.allowed_handoff_hosts().clone());
    let room_gateway = RoomGatewayClient::from_config(&config)?;

    println!(
        "dispatching pinned local Agent Card for {} ({})",
        registered_agent.display_name, registered_agent.id
    );
    let dispatcher = Dispatcher::new(
        room_gateway,
        adapter,
        Uuid::new_v4(),
        Duration::from_secs(1),
    );
    loop {
        dispatcher.run_once().await?;
        tokio::time::sleep(Duration::from_millis(config.poll_millis())).await;
    }
}
