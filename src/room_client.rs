use std::sync::Arc;

use async_trait::async_trait;
use reqwest::{Client, StatusCode, redirect::Policy};
use serde::Deserialize;
use thiserror::Error;
use url::Url;
use uuid::Uuid;

use crate::{
    GatewayConfig,
    config::ClientCredentialsConfig,
    domain::{ClaimRequest, ContextResponse, TaskUpdateRequest, TaskUpdateResponse},
};

const LEASE_TOKEN_HEADER: &str = "x-thought-khoral-lease-token";

#[async_trait]
trait ServiceTokenProvider: Send + Sync {
    /// Obtains only a workload/client-credentials token. This API has no
    /// browser-token input by design.
    async fn service_access_token(&self) -> Result<String, RoomClientError>;
}

struct ClientCredentialsTokenProvider {
    client: Client,
    credentials: ClientCredentialsConfig,
}

impl ClientCredentialsTokenProvider {
    fn new(credentials: ClientCredentialsConfig) -> Result<Self, RoomClientError> {
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
            .post(self.credentials.token_url().clone())
            .basic_auth(
                self.credentials.client_id(),
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

#[cfg(test)]
struct StaticServiceTokenProvider {
    token: String,
}

#[cfg(test)]
impl StaticServiceTokenProvider {
    fn new(token: impl Into<String>) -> Self {
        Self {
            token: token.into(),
        }
    }
}

#[cfg(test)]
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
            config.client_credentials().clone(),
        )?);
        Ok(Self {
            origin: config.room_gateway_origin().as_url().clone(),
            client: secure_http_client()?,
            token_provider,
        })
    }

    #[cfg(test)]
    fn for_test(origin: Url, token_provider: Arc<dyn ServiceTokenProvider>) -> Self {
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
        // Workload secrets must not be sent to a host configured through the
        // ambient HTTP(S)_PROXY environment.
        .no_proxy()
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

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
        sync::Mutex,
    };

    use super::*;
    use crate::ClaimRequest;

    #[tokio::test]
    async fn room_client_never_forwards_a_user_access_token() {
        let server = recording_mock_room_gateway().await;
        RoomGatewayClient::for_test(server.url(), service_token_provider())
            .claim(ClaimRequest {
                lease_owner: Uuid::new_v4(),
            })
            .await
            .unwrap();

        assert_eq!(
            server.last_authorization().await,
            Some("Bearer service-token".into())
        );
    }

    fn service_token_provider() -> Arc<StaticServiceTokenProvider> {
        Arc::new(StaticServiceTokenProvider::new("service-token"))
    }

    struct RecordingMockRoomGateway {
        url: Url,
        authorization: Arc<Mutex<Option<String>>>,
    }

    impl RecordingMockRoomGateway {
        fn url(&self) -> Url {
            self.url.clone()
        }

        async fn last_authorization(&self) -> Option<String> {
            self.authorization.lock().await.clone()
        }
    }

    async fn recording_mock_room_gateway() -> RecordingMockRoomGateway {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let authorization = Arc::new(Mutex::new(None));
        let recorded_authorization = Arc::clone(&authorization);

        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = vec![0_u8; 4_096];
            let read = socket.read(&mut request).await.unwrap();
            let request = String::from_utf8_lossy(&request[..read]);
            let value = request
                .lines()
                .find_map(|line| line.strip_prefix("authorization: "))
                .map(str::to_owned);
            *recorded_authorization.lock().await = value;
            socket
                .write_all(
                    b"HTTP/1.1 204 No Content\r\ncontent-length: 0\r\nconnection: close\r\n\r\n",
                )
                .await
                .unwrap();
        });

        RecordingMockRoomGateway {
            url: Url::parse(&format!("http://{address}")).unwrap(),
            authorization,
        }
    }
}
