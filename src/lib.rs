//! Zero-trust adapters between the governed room gateway and the one pinned
//! local A2A reference agent.
//!
//! This crate deliberately owns neither task persistence nor room
//! authorization. It has no database dependency and does not accept browser
//! credentials.

pub mod config;
pub mod domain;
pub mod registry;
pub mod room_client;

pub use config::{ClientCredentialsConfig, ConfigError, GatewayConfig};
pub use domain::{
    ActiveDecision, AgentTaskLease, ClaimRequest, ContextResponse, NormalizedAgentTaskUpdate,
    RoomContextPacket, TaskUpdateRequest, TaskUpdateResponse,
};
pub use registry::{
    PinnedAgentRegistration, RegisteredAgent, RegistryError, SkillId, reference_registration,
};
pub use room_client::{
    ClientCredentialsTokenProvider, RoomClientError, RoomGatewayClient, ServiceTokenProvider,
    StaticServiceTokenProvider,
};
