use std::sync::Arc;

use thought_khoral_agent_gateway::{ClaimRequest, RoomGatewayClient, StaticServiceTokenProvider};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    sync::Mutex,
};
use url::Url;
use uuid::Uuid;

#[tokio::test]
async fn room_client_never_forwards_a_user_access_token() {
    let server = recording_mock_room_gateway().await;
    RoomGatewayClient::for_test(server.url(), service_token_provider())
        .claim(test_claim())
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

fn test_claim() -> ClaimRequest {
    ClaimRequest {
        lease_owner: Uuid::new_v4(),
    }
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
            .write_all(b"HTTP/1.1 204 No Content\r\ncontent-length: 0\r\nconnection: close\r\n\r\n")
            .await
            .unwrap();
    });

    RecordingMockRoomGateway {
        url: Url::parse(&format!("http://{address}")).unwrap(),
        authorization,
    }
}
