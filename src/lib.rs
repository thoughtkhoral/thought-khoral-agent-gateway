//! Zero-trust adapters between the governed room gateway and the one pinned
//! local A2A reference agent.
//!
//! This crate deliberately owns neither task persistence nor room
//! authorization. It has no database dependency and does not accept browser
//! credentials.

pub mod a2a_adapter;
pub mod config;
pub mod dispatcher;
pub mod domain;
pub mod reference_agent;
pub mod registry;
pub mod room_client;
pub mod update_validation;

pub use config::{ConfigError, GatewayConfig, RoomGatewayOrigin};
pub use domain::{
    ActiveDecision, AgentTaskLease, ClaimRequest, ContextResponse, NormalizedAgentTaskUpdate,
    RoomContextPacket, TaskUpdateRequest, TaskUpdateResponse,
};
pub use registry::{
    PinnedAgentRegistration, RegisteredAgent, RegistryError, SkillId, reference_registration,
};
pub use room_client::{RoomClientError, RoomGatewayClient};
