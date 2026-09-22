use std::sync::Arc;

use async_trait::async_trait;
use reqwest::{Client, StatusCode, redirect::Policy};
use serde::Deserialize;
use thiserror::Error;
use url::Url;
use uuid::Uuid;

use crate::{
    ClientCredentialsConfig, GatewayConfig,
    domain::{ClaimRequest, ContextResponse, TaskUpdateRequest, TaskUpdateResponse},
};

const LEASE_TOKEN_HEADER: &str = "x-thought-khoral-lease-token";

#[async_trait]
pub trait ServiceTokenProvider: Send + Sync {
    /// Obtains only a workload/client-credentials token. This API has no
    /// browser-token input by design.
    async fn service_access_token(&self) -> Result<String, RoomClientError>;
}

pub struct ClientCredentialsTokenProvider {
    client: Client,
    credentials: ClientCredentialsConfig,
}

impl ClientCredentialsTokenProvider {
    pub fn new(credentials: ClientCredentialsConfig) -> Result<Self, RoomClientError> {
        Ok(Self {
            client: secure_http_client()?,
            credentials,
        })
    }
}

#[async_trait]
impl ServiceTokenProvider for ClientCredentialsTokenProvider {
    async fn service_access_token(&self) -> Result<String, RoomClientError> {
        let response = self
            .client
            .post(self.credentials.token_url.clone())
            .basic_auth(
                &self.credentials.client_id,
                Some(self.credentials.client_secret()),
            )
            .form(&[("grant_type", "client_credentials")])
            .send()
            .await?
            .error_for_status()?;
        let token = response.json::<TokenResponse>().await?.access_token;
        if token.is_empty() {
            return Err(RoomClientError::EmptyServiceToken);
        }
        Ok(token)
    }
}

/// Test-only style provider for exercising the client boundary. Production
/// construction is `RoomGatewayClient::from_config`, which uses the
/// client-credentials provider above.
pub struct StaticServiceTokenProvider {
    token: String,
}

impl StaticServiceTokenProvider {
    pub fn new(token: impl Into<String>) -> Self {
        Self {
            token: token.into(),
        }
    }
}

#[async_trait]
impl ServiceTokenProvider for StaticServiceTokenProvider {
    async fn service_access_token(&self) -> Result<String, RoomClientError> {
        if self.token.is_empty() {
            return Err(RoomClientError::EmptyServiceToken);
        }
        Ok(self.token.clone())
    }
}

pub struct RoomGatewayClient {
    origin: Url,
    client: Client,
    token_provider: Arc<dyn ServiceTokenProvider>,
}

impl RoomGatewayClient {
    pub fn from_config(config: &GatewayConfig) -> Result<Self, RoomClientError> {
        let token_provider = Arc::new(ClientCredentialsTokenProvider::new(
            config.client_credentials.clone(),
        )?);
        Ok(Self {
            origin: config.room_gateway_origin.clone(),
            client: secure_http_client()?,
            token_provider,
        })
    }

    pub fn for_test(origin: Url, token_provider: Arc<dyn ServiceTokenProvider>) -> Self {
        Self {
            origin,
            client: secure_http_client().expect("safe HTTP client construction"),
            token_provider,
        }
    }

    pub async fn claim(
        &self,
        request: ClaimRequest,
    ) -> Result<Option<ContextResponse>, RoomClientError> {
        let response = self
            .authorized(
                self.client
                    .post(self.endpoint("internal/v1/agent-tasks/claim")?),
            )
            .await?
            .json(&request)
            .send()
            .await?;
        if response.status() == StatusCode::NO_CONTENT {
            return Ok(None);
        }
        Ok(Some(response.error_for_status()?.json().await?))
    }

    pub async fn fetch_context(
        &self,
        task_id: Uuid,
        lease_token: Uuid,
    ) -> Result<ContextResponse, RoomClientError> {
        let response = self
            .authorized(
                self.client
                    .get(self.endpoint(&format!("internal/v1/agent-tasks/{task_id}/context"))?)
                    .header(LEASE_TOKEN_HEADER, lease_token.to_string()),
            )
            .await?
            .send()
            .await?
            .error_for_status()?;
        Ok(response.json().await?)
    }

    pub async fn submit_update(
        &self,
        task_id: Uuid,
        lease_token: Uuid,
        request: TaskUpdateRequest,
    ) -> Result<TaskUpdateResponse, RoomClientError> {
        let response = self
            .authorized(
                self.client
                    .post(self.endpoint(&format!("internal/v1/agent-tasks/{task_id}/updates"))?)
                    .header(LEASE_TOKEN_HEADER, lease_token.to_string()),
            )
            .await?
            .json(&request)
            .send()
            .await?
            .error_for_status()?;
        Ok(response.json().await?)
    }

    async fn authorized(
        &self,
        request: reqwest::RequestBuilder,
    ) -> Result<reqwest::RequestBuilder, RoomClientError> {
        let token = self.token_provider.service_access_token().await?;
        Ok(request.bearer_auth(token))
    }

    fn endpoint(&self, relative_path: &str) -> Result<Url, RoomClientError> {
        self.origin
            .join(relative_path)
            .map_err(|_| RoomClientError::InvalidConfiguredOrigin)
    }
}

fn secure_http_client() -> Result<Client, RoomClientError> {
    Client::builder()
        // A redirect could move a workload bearer token to another origin.
        .redirect(Policy::none())
        .build()
        .map_err(RoomClientError::from)
}

#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: String,
}

#[derive(Debug, Error)]
pub enum RoomClientError {
    #[error("configured room-gateway origin cannot form an internal endpoint")]
    InvalidConfiguredOrigin,
    #[error("Keycloak returned an empty service access token")]
    EmptyServiceToken,
    #[error("HTTP client request failed")]
    Http(#[from] reqwest::Error),
}
