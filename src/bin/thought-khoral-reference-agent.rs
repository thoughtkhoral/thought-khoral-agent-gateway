use std::error::Error;

use thought_khoral_agent_gateway::reference_agent::local_router;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let bearer_secret = std::env::var("THOUGHT_KHORAL_REFERENCE_AGENT_INBOUND_SECRET")?;
    if bearer_secret.is_empty() {
        return Err("THOUGHT_KHORAL_REFERENCE_AGENT_INBOUND_SECRET must not be empty".into());
    }
    // The Reference Agent intentionally has no configurable host, port, or
    // egress target. It serves only the pinned loopback card and JSON-RPC path.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:9090").await?;
    axum::serve(listener, local_router(bearer_secret)).await?;
    Ok(())
}
