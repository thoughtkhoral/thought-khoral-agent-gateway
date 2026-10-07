// SPDX-License-Identifier: Apache-2.0
//! Workload-only catalog bridge. No task, receipt or arbitrary discovery API.
use crate::conversation_dispatcher::{MediationError, WorkerClient};
use axum::{
    Json, Router,
    extract::{Request, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::get,
};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::sync::Arc;
const PATH: &str = "/internal/agent-conversations/v1/models";
#[derive(Clone)]
struct Bridge {
    worker: Arc<WorkerClient>,
    credential_hash: [u8; 32],
}
pub fn catalog_service(worker: WorkerClient, secret: String) -> Result<Router, MediationError> {
    if secret.len() < 32
        || secret.len() > 4096
        || !secret
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-._~+/=".contains(&b))
    {
        return Err(crate::conversation_validation::ValidationError.into());
    }
    let state = Bridge {
        worker: Arc::new(worker),
        credential_hash: Sha256::digest(secret.as_bytes()).into(),
    };
    Ok(Router::new().route(PATH, get(catalog)).with_state(state))
}
fn failure(status: StatusCode, code: &str) -> Response {
    (status, Json(json!({"code":code}))).into_response()
}
async fn catalog(State(state): State<Bridge>, request: Request) -> Response {
    let mut headers = request.headers().get_all("authorization").iter();
    let valid = headers
        .next()
        .and_then(|h| h.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
        .is_some_and(|token| {
            if token.len() > 4096 {
                return false;
            }
            let hash = Sha256::digest(token.as_bytes());
            hash.iter()
                .zip(state.credential_hash)
                .fold(0u8, |n, (a, b)| n | (a ^ b))
                == 0
        })
        && headers.next().is_none();
    if !valid {
        return failure(StatusCode::UNAUTHORIZED, "authentication_required");
    }
    if request.uri().query().is_some() {
        return failure(StatusCode::BAD_REQUEST, "invalid_task_input");
    }
    match tokio::time::timeout(
        std::time::Duration::from_secs(1),
        axum::body::to_bytes(request.into_body(), 1),
    )
    .await
    {
        Ok(Ok(bytes)) if bytes.is_empty() => (),
        _ => return failure(StatusCode::BAD_REQUEST, "invalid_task_input"),
    }
    if state.worker.admit().await.is_err() {
        return failure(StatusCode::SERVICE_UNAVAILABLE, "runtime_unavailable");
    }
    match state.worker.catalog().await {
        Ok(page) => Json(page).into_response(),
        Err(_) => failure(StatusCode::SERVICE_UNAVAILABLE, "runtime_unavailable"),
    }
}
