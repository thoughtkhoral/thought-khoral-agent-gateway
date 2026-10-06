// SPDX-License-Identifier: Apache-2.0
//! Separate Codex mediation. The broker owns room authorization and persistence.
use crate::{
    RoomClientError, RoomGatewayClient,
    conversation_validation::{self as v, Admission, CODEX_ID, PROFILE, ValidationError},
};
use reqwest::{Client, Method, StatusCode, redirect::Policy};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{collections::BTreeSet, path::PathBuf, time::Duration};
use tokio::sync::Mutex;
use uuid::Uuid;
const WORKER: &str = "http://thought-khoral-codex-agent:9091";
#[derive(thiserror::Error, Debug)]
pub enum MediationError {
    #[error("conversation validation failed")]
    Validation(#[from] ValidationError),
    #[error("worker transport failed")]
    Http(#[from] reqwest::Error),
    #[error("broker transport failed")]
    Broker(#[from] RoomClientError),
    #[error("private mediator state unavailable")]
    State(#[from] std::io::Error),
    #[error("worker execution or authority is uncertain")]
    Interrupted,
    #[error("broker task authority has expired or been revoked")]
    AuthorityLost,
}
fn definitive_authority_loss(error: &MediationError) -> bool {
    matches!(error, MediationError::AuthorityLost)
}
type Result<T> = std::result::Result<T, MediationError>;
/// Credentials deliberately do not implement Debug or Serialize.
pub struct WorkerClient {
    client: Client,
    secret: String,
    pub admission: Admission,
}
impl WorkerClient {
    pub fn new(secret: String, admission: Admission) -> Result<Self> {
        Self::build(secret, admission, None)
    }
    pub fn with_loopback_dns(
        secret: String,
        admission: Admission,
        ip: std::net::IpAddr,
    ) -> Result<Self> {
        if !ip.is_loopback() {
            return Err(ValidationError.into());
        }
        Self::build(secret, admission, Some(ip))
    }
    fn build(secret: String, admission: Admission, ip: Option<std::net::IpAddr>) -> Result<Self> {
        if secret.len() < 16 || secret.len() > 4096 || !secret.bytes().all(|b| b.is_ascii_graphic())
        {
            return Err(ValidationError.into());
        }
        let mut b = Client::builder()
            .no_proxy()
            .redirect(Policy::none())
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(185));
        if let Some(ip) = ip {
            b = b.resolve("thought-khoral-codex-agent", (ip, 9091).into())
        }
        Ok(Self {
            client: b.build()?,
            secret,
            admission,
        })
    }
    async fn request(
        &self,
        method: Method,
        path: &str,
        body: Option<&Value>,
        timeout: Duration,
    ) -> Result<Option<Value>> {
        let mut q = self
            .client
            .request(method, format!("{WORKER}{path}"))
            .bearer_auth(&self.secret)
            .timeout(timeout);
        if let Some(b) = body {
            q = q.json(b)
        }
        let mut response = q.send().await?;
        let status = response.status();
        let mut bytes = vec![];
        while let Some(chunk) = response.chunk().await? {
            if bytes.len() + chunk.len() > 4 * 1_048_576 {
                return Err(ValidationError.into());
            }
            bytes.extend_from_slice(&chunk)
        }
        if status == StatusCode::NO_CONTENT {
            return Ok(None);
        }
        let value = v::parse_json(&bytes)?;
        if status == StatusCode::BAD_REQUEST
            && path.starts_with("/control/v1/receipts/")
            && value == json!({"code":"invalid_task_input"})
        {
            return Ok(None);
        }
        if !status.is_success() {
            return Err(MediationError::Interrupted);
        }
        Ok(Some(value))
    }
    pub async fn admit(&self) -> Result<()> {
        v::validate_card(
            &self
                .request(
                    Method::GET,
                    "/.well-known/agent-card.json",
                    None,
                    Duration::from_secs(5),
                )
                .await?
                .ok_or(ValidationError)?,
        )?;
        Ok(())
    }
    /// Authenticated, one bounded normalized page. The later broker CatalogQuery bridge can call this API.
    pub async fn catalog(&self) -> Result<Value> {
        let page = self
            .request(
                Method::GET,
                "/control/v1/models",
                None,
                Duration::from_secs(5),
            )
            .await?
            .ok_or(ValidationError)?;
        v::schema("catalog", &page)?;
        if v::canonical_bytes(&page)?.len() > 1_048_576 {
            return Err(ValidationError.into());
        }
        if page["catalogRevision"] != self.admission.catalog_revision
            || !page["nextCursor"].is_null()
        {
            return Err(ValidationError.into());
        }
        let mut ids = BTreeSet::new();
        for model in page["data"].as_array().ok_or(ValidationError)? {
            let id = model["id"].as_str().ok_or(ValidationError)?;
            let allowed = self.admission.models.get(id).ok_or(ValidationError)?;
            if !ids.insert(id) {
                return Err(ValidationError.into());
            }
            let mut efforts = BTreeSet::new();
            for e in model["supportedReasoningEfforts"]
                .as_array()
                .ok_or(ValidationError)?
            {
                let id = e["id"].as_str().ok_or(ValidationError)?;
                if !allowed.iter().any(|s| s == id) || !efforts.insert(id) {
                    return Err(ValidationError.into());
                }
            }
            if !model["defaultReasoningEffort"].is_null()
                && !efforts.contains(
                    model["defaultReasoningEffort"]
                        .as_str()
                        .ok_or(ValidationError)?,
                )
            {
                return Err(ValidationError.into());
            }
        }
        Ok(page)
    }
    async fn receipt(&self, id: &str) -> Result<Option<Value>> {
        self.request(
            Method::GET,
            &format!("/control/v1/receipts/{id}"),
            None,
            Duration::from_secs(5),
        )
        .await
    }
    async fn rpc(&self, method: &str, params: Value, timeout: Duration) -> Result<Value> {
        let id = Uuid::new_v4().to_string();
        let response = self
            .request(
                Method::POST,
                "/",
                Some(&json!({"jsonrpc":"2.0","id":id,"method":method,"params":params})),
                timeout,
            )
            .await?
            .ok_or(ValidationError)?;
        if response["jsonrpc"] != "2.0" || response["id"] != id || response.get("error").is_some() {
            return Err(MediationError::Interrupted);
        }
        response
            .get("result")
            .cloned()
            .ok_or_else(|| ValidationError.into())
    }
    async fn cancel(&self, id: &str) {
        let _ = self
            .rpc("CancelTask", json!({"id":id}), Duration::from_secs(5))
            .await;
    }
    async fn ack(&self, id: &str, ack: &Value) -> Result<()> {
        self.request(
            Method::POST,
            &format!("/control/v1/receipts/{id}/ack"),
            Some(ack),
            Duration::from_secs(5),
        )
        .await?;
        Ok(())
    }
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Record {
    lease: String,
    binding: Value,
    sources: BTreeSet<String>,
    phase: String,
    result: Option<Value>,
    runtime_binding: Option<Value>,
    update: Option<Value>,
    ack: Option<Value>,
}
fn binding(p: &Value) -> Value {
    let mut b = p.clone();
    for k in ["entries", "activeDecisions", "nativeReplyBindings"] {
        b["context"]
            .as_object_mut()
            .expect("validated context")
            .remove(k);
    }
    for k in ["leaseOwner", "leaseExpiresAt"] {
        b.as_object_mut().expect("validated input").remove(k);
    }
    b
}
fn closed(v: &Value, names: &[&str]) -> bool {
    v.as_object()
        .is_some_and(|m| m.len() == names.len() && m.keys().all(|k| names.contains(&k.as_str())))
}
fn validate_receipt(r: &Value, p: &Value) -> Result<()> {
    if !closed(
        r,
        &[
            "taskId",
            "conversationId",
            "generation",
            "phase",
            "result",
            "acknowledgement",
            "error",
            "runtimeBinding",
        ],
    ) || r["taskId"] != p["taskId"]
        || r["conversationId"] != p["conversation"]["id"]
        || r["generation"] != p["conversation"]["generation"]
        || !matches!(
            r["phase"].as_str(),
            Some("reserved" | "running" | "completed" | "interrupted" | "failed")
        )
    {
        return Err(ValidationError.into());
    }
    Ok(())
}
fn validate_runtime(r: &Value) -> Result<()> {
    if !closed(r, &["threadId", "turnId"])
        || ["threadId", "turnId"]
            .iter()
            .any(|k| r[*k].as_str().is_none_or(|s| s.is_empty() || s.len() > 256))
    {
        return Err(ValidationError.into());
    }
    Ok(())
}
pub struct ConversationDispatcher {
    room: RoomGatewayClient,
    worker: WorkerClient,
    dir: PathBuf,
    owner: String,
    gate: Mutex<()>,
}
impl ConversationDispatcher {
    pub fn new(
        room: RoomGatewayClient,
        worker: WorkerClient,
        dir: PathBuf,
        owner: String,
    ) -> Result<Self> {
        if owner.is_empty() || owner.len() > 128 || !dir.is_absolute() {
            return Err(ValidationError.into());
        }
        std::fs::create_dir_all(&dir)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))?;
        }
        if std::fs::symlink_metadata(&dir)?.file_type().is_symlink() {
            return Err(ValidationError.into());
        }
        Ok(Self {
            room,
            worker,
            dir,
            owner,
            gate: Mutex::new(()),
        })
    }
    fn path(&self, id: &str) -> Result<PathBuf> {
        if Uuid::parse_str(id).is_err() {
            return Err(ValidationError.into());
        }
        Ok(self.dir.join(format!("{id}.json")))
    }
    fn load(&self, id: &str) -> Result<Option<Record>> {
        let path = self.path(id)?;
        match std::fs::read(path) {
            Ok(b) => {
                let value = v::parse_json(&b)?;
                serde_json::from_value(value)
                    .map(Some)
                    .map_err(|_| ValidationError.into())
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.into()),
        }
    }
    fn save(&self, id: &str, r: &Record) -> Result<()> {
        use std::io::Write;
        let path = self.path(id)?;
        let temp = self.dir.join(format!("{}.tmp", Uuid::new_v4()));
        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        let mut f = opts.open(&temp)?;
        f.write_all(&serde_json::to_vec(r).map_err(|_| ValidationError)?)?;
        f.sync_all()?;
        std::fs::rename(temp, path)?;
        std::fs::File::open(&self.dir)?.sync_all()?;
        Ok(())
    }
    fn prior_sources(&self, p: &Value) -> Result<BTreeSet<String>> {
        let mut sources = BTreeSet::new();
        for file in std::fs::read_dir(&self.dir)? {
            let path = file?.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let id = path
                .file_stem()
                .and_then(|s| s.to_str())
                .ok_or(ValidationError)?;
            let Some(r) = self.load(id)? else { continue };
            let b = &r.binding;
            if r.phase == "acknowledged"
                && r.ack.is_some()
                && ["roomId", "agentId"].iter().all(|k| b[*k] == p[*k])
                && b["conversation"]["id"] == p["conversation"]["id"]
                && b["conversation"]["generation"] == p["conversation"]["generation"]
                && b["context"]["policyRevision"] == p["context"]["policyRevision"]
            {
                sources.extend(r.sources);
            }
        }
        Ok(sources)
    }
    async fn broker(
        &self,
        id: &str,
        suffix: &str,
        lease: Option<&str>,
        body: Option<&Value>,
    ) -> Result<Value> {
        self.room
            .conversation_request(
                if body.is_some() {
                    Method::POST
                } else {
                    Method::GET
                },
                &format!("tasks/{id}/{suffix}"),
                lease,
                body,
            )
            .await?
            .ok_or_else(|| ValidationError.into())
    }
    async fn authority(&self, p: &Value, lease: &str) -> Result<()> {
        let id = p["taskId"].as_str().ok_or(ValidationError)?;
        let a = match self.broker(id, "authority", Some(lease), None).await {
            Err(MediationError::Broker(RoomClientError::Http(e)))
                if e.status()
                    .is_some_and(|s| matches!(s.as_u16(), 403 | 409 | 410)) =>
            {
                return Err(MediationError::AuthorityLost);
            }
            result => result?,
        };
        if !closed(
            &a,
            &[
                "profileVersion",
                "taskId",
                "generation",
                "contextDigest",
                "expiresAt",
                "authorizationExpiresAt",
            ],
        ) || a["profileVersion"] != PROFILE
            || a["taskId"] != p["taskId"]
            || a["generation"] != p["conversation"]["generation"]
            || a["contextDigest"] != p["context"]["digest"]
            || a["expiresAt"] != p["expiresAt"]
            || a["authorizationExpiresAt"] != p["authorizationExpiresAt"]
        {
            return Err(ValidationError.into());
        }
        if chrono::Utc::now() >= v::timestamp(&a["expiresAt"])?
            || chrono::Utc::now() >= v::timestamp(&a["authorizationExpiresAt"])?
        {
            return Err(MediationError::AuthorityLost);
        }
        Ok(())
    }
    fn update(p: &Value, kind: &str, data: Value) -> Value {
        json!({"profileVersion":PROFILE,"updateId":Uuid::new_v4(),"ordinal":1,"taskId":p["taskId"],"generation":p["conversation"]["generation"],"contextDigest":p["context"]["digest"],"kind":kind,"data":data})
    }
    async fn accept_ack(&self, id: &str, r: &mut Record, ack: Value) -> Result<()> {
        v::schema("ack", &ack)?;
        let p = &r.binding;
        let result = r.result.as_ref().ok_or(ValidationError)?;
        if ack["taskId"] != p["taskId"]
            || ack["conversationId"] != p["conversation"]["id"]
            || ack["generation"] != p["conversation"]["generation"]
            || ack["consumedRevision"] != p["context"]["revision"]
            || ack["contextDigest"] != p["context"]["digest"]
            || ack["textDigest"]
                != format!(
                    "{:x}",
                    <sha2::Sha256 as sha2::Digest>::digest(
                        result["assistantText"]
                            .as_str()
                            .ok_or(ValidationError)?
                            .as_bytes()
                    )
                )
            || r.ack.as_ref().is_some_and(|a| a != &ack)
        {
            return Err(ValidationError.into());
        }
        self.worker.ack(id, &ack).await?;
        r.ack = Some(ack);
        r.phase = "acknowledged".into();
        self.save(id, r)
    }
    async fn reconcile_broker(&self, id: &str, r: &mut Record) -> Result<bool> {
        let receipt = self.broker(id, "receipt", None, None).await?;
        if !closed(
            &receipt,
            &[
                "profileVersion",
                "taskId",
                "conversationId",
                "generation",
                "state",
                "acknowledgement",
                "result",
            ],
        ) || receipt["profileVersion"] != PROFILE
            || receipt["taskId"] != r.binding["taskId"]
            || receipt["conversationId"] != r.binding["conversation"]["id"]
            || receipt["generation"] != r.binding["conversation"]["generation"]
        {
            return Err(ValidationError.into());
        }
        if receipt["state"] == "failed" {
            if !receipt["acknowledgement"].is_null() || !receipt["result"].is_null() {
                return Err(ValidationError.into());
            }
            self.quarantine(id, r).await?;
            return Ok(true);
        }
        if receipt["state"] == "completed" {
            if r.result.as_ref() != Some(&receipt["result"]) {
                return Err(ValidationError.into());
            }
            self.accept_ack(id, r, receipt["acknowledgement"].clone())
                .await?;
            return Ok(true);
        }
        Ok(false)
    }
    async fn quarantine(&self, id: &str, r: &mut Record) -> Result<()> {
        self.worker.cancel(id).await;
        r.phase = "quarantined".into();
        self.save(id, r)
    }
    async fn recover_pending(&self, id: &str, r: &mut Record) -> Result<bool> {
        if self.reconcile_broker(id, r).await? {
            return Ok(r.phase != "quarantined");
        }
        if r.phase != "completed" {
            return Ok(false);
        }
        // Claims are not reissued while the original lease is active. Replay only
        // the stored normalized output, never a worker/provider invocation.
        self.authority(&r.binding, &r.lease).await?;
        v::validate_result_with_admission(
            r.result.as_ref().ok_or(ValidationError)?,
            &r.binding,
            &r.sources,
            &self.worker.admission,
        )?;
        let update = r.update.as_ref().ok_or(ValidationError)?;
        v::schema("update", update)?;
        let ack = self
            .broker(id, "updates", Some(&r.lease), Some(update))
            .await?;
        self.accept_ack(id, r, ack).await?;
        Ok(true)
    }
    pub async fn run_once(&self) -> Result<bool> {
        let _guard = self.gate.lock().await;
        for file in std::fs::read_dir(&self.dir)? {
            let path = file?.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let id = path
                .file_stem()
                .and_then(|s| s.to_str())
                .ok_or(ValidationError)?;
            if let Some(mut r) = self.load(id)? {
                if r.result.is_some() && r.ack.is_none() && r.phase != "quarantined" {
                    match self.recover_pending(id, &mut r).await {
                        Ok(true) => return Ok(true),
                        Ok(false) => (),
                        Err(MediationError::State(e)) => return Err(e.into()),
                        Err(e) => {
                            if definitive_authority_loss(&e) {
                                self.quarantine(id, &mut r).await?;
                            }
                            // Transport/malformed-receipt uncertainty remains durable and
                            // fail closed for this task, but does not starve other rooms.
                        }
                    }
                }
            }
        }
        let Some(claim) = self
            .room
            .conversation_request(
                Method::POST,
                "claim",
                None,
                Some(&json!({"agentId":CODEX_ID,"leaseOwner":self.owner})),
            )
            .await?
        else {
            return Ok(false);
        };
        if !closed(&claim, &["packet", "leaseToken"]) {
            return Err(ValidationError.into());
        }
        let p = claim["packet"].clone();
        let lease = claim["leaseToken"].as_str().ok_or(ValidationError)?;
        let id = p["taskId"].as_str().ok_or(ValidationError)?;
        let prior = self.prior_sources(&p)?;
        v::validate_packet_with_sources(&p, &self.worker.admission, true, &prior)?;
        if p["leaseOwner"] != self.owner {
            return Err(ValidationError.into());
        }
        let mut r = match self.load(id)? {
            Some(r) => {
                if r.binding != binding(&p) {
                    return Err(ValidationError.into());
                }
                r
            }
            None => Record {
                lease: lease.to_owned(),
                binding: binding(&p),
                sources: v::sources(&p).union(&prior).cloned().collect(),
                phase: "reserved".into(),
                result: None,
                runtime_binding: None,
                update: None,
                ack: None,
            },
        };
        r.lease = lease.to_owned();
        // Broker-first recovery always precedes worker invocation, including a lost terminal acknowledgement.
        if self.reconcile_broker(id, &mut r).await? {
            return Ok(true);
        }
        if let Err(e) = self.authority(&p, lease).await {
            self.worker.cancel(id).await;
            return Err(e);
        }
        let worker_receipt = self.worker.receipt(id).await;
        match worker_receipt {
            Ok(Some(receipt)) => {
                validate_receipt(&receipt, &p)?;
                if receipt["phase"] == "completed" {
                    validate_runtime(&receipt["runtimeBinding"])?;
                    v::validate_result_with_admission(
                        &receipt["result"],
                        &p,
                        &r.sources,
                        &self.worker.admission,
                    )?;
                    if r.result.as_ref().is_some_and(|v| v != &receipt["result"])
                        || r.runtime_binding
                            .as_ref()
                            .is_some_and(|v| v != &receipt["runtimeBinding"])
                    {
                        return Err(ValidationError.into());
                    }
                    r.result = Some(receipt["result"].clone());
                    r.runtime_binding = Some(receipt["runtimeBinding"].clone());
                } else {
                    self.worker.cancel(id).await;
                    return self.fail(id, &p, lease, &mut r).await;
                }
            }
            Err(_) => {
                self.worker.cancel(id).await;
                return self.fail(id, &p, lease, &mut r).await;
            }
            Ok(None) => {
                if r.phase != "reserved" || r.result.is_some() {
                    self.worker.cancel(id).await;
                    return self.fail(id, &p, lease, &mut r).await;
                }
                self.worker.admit().await?;
                let catalog = self.worker.catalog().await?;
                if !catalog["data"]
                    .as_array()
                    .ok_or(ValidationError)?
                    .iter()
                    .any(|m| {
                        m["id"] == p["model"]
                            && m["supportedReasoningEfforts"]
                                .as_array()
                                .is_some_and(|e| e.iter().any(|e| e["id"] == p["reasoningEffort"]))
                    })
                {
                    return Err(ValidationError.into());
                }
                self.save(id, &r)?;
                r.phase = "submission-intent".into();
                self.save(id, &r)?;
                let params = json!({"message":{"messageId":format!("thought-khoral-{id}"),"taskId":p["taskId"],"contextId":p["conversation"]["id"],"role":"ROLE_USER","parts":[{"data":{"profileVersion":PROFILE,"packet":p}}]},"configuration":{"acceptedOutputModes":["application/json"],"returnImmediately":false,"historyLength":0}});
                let send = self
                    .worker
                    .rpc("SendMessage", params, Duration::from_secs(185));
                tokio::pin!(send);
                let mut interval = tokio::time::interval(Duration::from_secs(1));
                interval.tick().await;
                let output = loop {
                    tokio::select! {out=&mut send=>break out, _=interval.tick()=>{if let Err(e)=self.authority(&p,lease).await{self.worker.cancel(id).await;r.phase="interrupted".into();self.save(id,&r)?;return Err(e)}}}
                };
                let output = match output {
                    Ok(v) => v,
                    Err(e) => {
                        self.worker.cancel(id).await;
                        r.phase = "interrupted".into();
                        self.save(id, &r)?;
                        let _ = self.fail(id, &p, lease, &mut r).await;
                        return Err(e);
                    }
                };
                if !closed(&output, &["task"]) {
                    return Err(ValidationError.into());
                }
                let task = &output["task"];
                if !closed(task, &["id", "contextId", "status", "artifacts"])
                    || !closed(&task["status"], &["state"])
                {
                    return Err(ValidationError.into());
                }
                let artifacts = task["artifacts"].as_array().ok_or(ValidationError)?;
                if task["id"] != p["taskId"]
                    || task["contextId"] != p["conversation"]["id"]
                    || task["status"]["state"] != "TASK_STATE_COMPLETED"
                    || artifacts.len() != 1
                    || !closed(&artifacts[0], &["artifactId", "parts"])
                    || artifacts[0]["artifactId"] != "reply"
                {
                    return Err(ValidationError.into());
                }
                let parts = artifacts[0]["parts"].as_array().ok_or(ValidationError)?;
                if parts.len() != 1
                    || !closed(&parts[0], &["data"])
                    || !closed(&parts[0]["data"], &["reply", "runtimeBinding"])
                {
                    return Err(ValidationError.into());
                }
                let data = &parts[0]["data"];
                let receipt = self.worker.receipt(id).await?.ok_or(ValidationError)?;
                validate_receipt(&receipt, &p)?;
                validate_runtime(&data["runtimeBinding"])?;
                if receipt["phase"] != "completed"
                    || receipt["runtimeBinding"] != data["runtimeBinding"]
                    || receipt["result"] != data["reply"]
                {
                    return Err(ValidationError.into());
                }
                v::validate_result_with_admission(
                    &data["reply"],
                    &p,
                    &r.sources,
                    &self.worker.admission,
                )?;
                r.result = Some(data["reply"].clone());
                r.runtime_binding = Some(data["runtimeBinding"].clone());
            }
        }
        if let Err(e) = self.authority(&p, lease).await {
            self.worker.cancel(id).await;
            return Err(e);
        }
        r.phase = "completed".into();
        if r.update.is_none() {
            r.update = Some(Self::update(
                &p,
                "completed",
                r.result.clone().ok_or(ValidationError)?,
            ));
        }
        self.save(id, &r)?;
        let update = r.update.as_ref().ok_or(ValidationError)?;
        v::schema("update", update)?;
        let ack = self
            .broker(id, "updates", Some(lease), Some(update))
            .await?;
        self.accept_ack(id, &mut r, ack).await?;
        Ok(true)
    }
    async fn fail(&self, id: &str, p: &Value, lease: &str, r: &mut Record) -> Result<bool> {
        r.phase = "interrupted".into();
        r.update = Some(Self::update(
            p,
            "failed",
            json!({"code":"conversation_interrupted"}),
        ));
        self.save(id, r)?;
        let update = r.update.as_ref().ok_or(ValidationError)?;
        v::schema("update", update)?;
        self.broker(id, "updates", Some(lease), Some(update))
            .await?;
        Ok(true)
    }
    pub async fn run_forever(&self, poll: Duration) -> Result<()> {
        loop {
            match self.run_once().await {
                Ok(_) => (),
                Err(MediationError::State(e)) => return Err(e.into()),
                Err(_) => (),
            };
            tokio::time::sleep(poll).await;
        }
    }
}
