// SPDX-License-Identifier: Apache-2.0
//! Strict validation of the separately pinned conversation profile.
use chrono::{DateTime, Utc};
use serde::{
    Deserialize, Deserializer,
    de::{self, MapAccess, SeqAccess, Visitor},
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use thiserror::Error;
pub const PROFILE: &str = "thought-khoral.agent-conversation.v1";
pub const CODEX_ID: &str = "74686f75-6768-746b-686f-72616c000004";
#[derive(Debug, Error)]
#[error("conversation profile validation failed")]
pub struct ValidationError;
pub type Checked<T> = Result<T, ValidationError>;
#[derive(Clone, Debug)]
pub struct Admission {
    pub catalog_revision: String,
    pub guidance_revision: String,
    pub models: BTreeMap<String, Vec<String>>,
}
impl Admission {
    pub fn new(
        catalog_revision: String,
        guidance_revision: String,
        models: BTreeMap<String, Vec<String>>,
    ) -> Checked<Self> {
        if catalog_revision.is_empty()
            || catalog_revision.len() > 128
            || guidance_revision.is_empty()
            || guidance_revision.len() > 128
            || models.is_empty()
            || models.len() > 100
            || models.iter().any(|(m, e)| {
                m.is_empty()
                    || m.len() > 128
                    || e.is_empty()
                    || e.iter().any(|s| s.is_empty() || s.len() > 128)
                    || e.iter().collect::<BTreeSet<_>>().len() != e.len()
            })
        {
            return Err(ValidationError);
        }
        Ok(Self {
            catalog_revision,
            guidance_revision,
            models,
        })
    }
}
struct Strict(Value);
impl<'de> Deserialize<'de> for Strict {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = Strict;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("unique-key safe-integer JSON")
            }
            fn visit_bool<E: de::Error>(self, v: bool) -> Result<Strict, E> {
                Ok(Strict(v.into()))
            }
            fn visit_unit<E: de::Error>(self) -> Result<Strict, E> {
                Ok(Strict(Value::Null))
            }
            fn visit_str<E: de::Error>(self, v: &str) -> Result<Strict, E> {
                Ok(Strict(v.into()))
            }
            fn visit_string<E: de::Error>(self, v: String) -> Result<Strict, E> {
                Ok(Strict(v.into()))
            }
            fn visit_u64<E: de::Error>(self, v: u64) -> Result<Strict, E> {
                if v > 9_007_199_254_740_991 {
                    return Err(E::custom("unsafe integer"));
                }
                Ok(Strict(v.into()))
            }
            fn visit_i64<E: de::Error>(self, v: i64) -> Result<Strict, E> {
                if v < 0 {
                    return Err(E::custom("negative integer"));
                }
                self.visit_u64(v as u64)
            }
            fn visit_f64<E: de::Error>(self, _: f64) -> Result<Strict, E> {
                Err(E::custom("floating number"))
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut a: A) -> Result<Strict, A::Error> {
                let mut v = vec![];
                while let Some(x) = a.next_element::<Strict>()? {
                    v.push(x.0)
                }
                Ok(Strict(v.into()))
            }
            fn visit_map<A: MapAccess<'de>>(self, mut a: A) -> Result<Strict, A::Error> {
                let mut v = serde_json::Map::new();
                while let Some(k) = a.next_key::<String>()? {
                    if v.contains_key(&k) {
                        return Err(de::Error::custom("duplicate key"));
                    }
                    v.insert(k, a.next_value::<Strict>()?.0);
                }
                Ok(Strict(v.into()))
            }
        }
        d.deserialize_any(V)
    }
}
pub fn parse_json(bytes: &[u8]) -> Checked<Value> {
    if bytes.len() > 4 * 1_048_576 {
        return Err(ValidationError);
    }
    serde_json::from_slice::<Strict>(bytes)
        .map(|v| v.0)
        .map_err(|_| ValidationError)
}
pub fn canonical_bytes(v: &Value) -> Checked<Vec<u8>> {
    fn sorted(v: &Value) -> Checked<Value> {
        Ok(match v {
            Value::Object(m) => {
                let mut out = serde_json::Map::new();
                let keys = m.keys().collect::<BTreeSet<_>>();
                for k in keys {
                    out.insert(k.clone(), sorted(&m[k])?);
                }
                Value::Object(out)
            }
            Value::Array(a) => Value::Array(a.iter().map(sorted).collect::<Checked<_>>()?),
            Value::Number(n) => {
                if n.as_u64().is_none_or(|n| n > 9_007_199_254_740_991) {
                    return Err(ValidationError);
                }
                v.clone()
            }
            _ => v.clone(),
        })
    }
    serde_json::to_vec(&sorted(v)?).map_err(|_| ValidationError)
}
pub fn context_digest(p: &Value) -> Checked<String> {
    let mut context = p["context"].clone();
    context
        .as_object_mut()
        .ok_or(ValidationError)?
        .remove("digest");
    let v = json!({"roomId":p["roomId"],"agentId":p["agentId"],"conversationId":p["conversation"]["id"],"generation":p["conversation"]["generation"],"triggerEventId":p["triggerEventId"],"guidanceRevision":p["guidanceRevision"],"context":context});
    let b = canonical_bytes(&v)?;
    if b.len() > 1_048_576 {
        return Err(ValidationError);
    }
    Ok(format!("{:x}", Sha256::digest(b)))
}
pub fn schema(name: &str, v: &Value) -> Checked<()> {
    let raw = match name {
        "input" => include_str!("../contracts/agent-conversation-v1/schemas/input.schema.json"),
        "result" => include_str!("../contracts/agent-conversation-v1/schemas/result.schema.json"),
        "update" => include_str!("../contracts/agent-conversation-v1/schemas/update.schema.json"),
        "catalog" => include_str!("../contracts/agent-conversation-v1/schemas/catalog.schema.json"),
        "ack" => include_str!("../contracts/agent-conversation-v1/schemas/ack.schema.json"),
        _ => return Err(ValidationError),
    };
    let turn: Value = serde_json::from_str(include_str!(
        "../contracts/agent-conversation-v1/schemas/turn.schema.json"
    ))
    .map_err(|_| ValidationError)?;
    fn expand(v: &mut Value, turn: &Value) -> Checked<()> {
        if let Some(r) = v.get("$ref").and_then(Value::as_str) {
            let pointer = r.strip_prefix("turn.schema.json#").ok_or(ValidationError)?;
            *v = turn.pointer(pointer).ok_or(ValidationError)?.clone();
        }
        match v {
            Value::Object(m) => {
                m.remove("$id");
                for child in m.values_mut() {
                    expand(child, turn)?
                }
            }
            Value::Array(a) => {
                for child in a {
                    expand(child, turn)?
                }
            }
            _ => (),
        }
        Ok(())
    }
    let mut s: Value = serde_json::from_str(raw).map_err(|_| ValidationError)?;
    expand(&mut s, &turn)?;
    canonical_bytes(v)?;
    let validator = jsonschema::options()
        .should_validate_formats(true)
        .build(&s)
        .map_err(|_| ValidationError)?;
    if validator.is_valid(v) {
        Ok(())
    } else {
        Err(ValidationError)
    }
}
pub fn validate_card(v: &Value) -> Checked<()> {
    if v == &crate::registry::codex_card() {
        Ok(())
    } else {
        Err(ValidationError)
    }
}
pub fn timestamp(v: &Value) -> Checked<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(v.as_str().ok_or(ValidationError)?)
        .map(|v| v.with_timezone(&Utc))
        .map_err(|_| ValidationError)
}
pub fn validate_packet(p: &Value, a: &Admission, current: bool) -> Checked<()> {
    validate_packet_with_sources(p, a, current, &BTreeSet::new())
}
pub fn validate_packet_with_sources(
    p: &Value,
    a: &Admission,
    current: bool,
    prior: &BTreeSet<String>,
) -> Checked<()> {
    schema("input", p)?;
    if p["agentId"] != CODEX_ID
        || p["guidanceRevision"] != a.guidance_revision
        || p["catalogRevision"] != a.catalog_revision
        || a.models
            .get(p["model"].as_str().ok_or(ValidationError)?)
            .is_none_or(|e| {
                !e.iter()
                    .any(|e| Some(e.as_str()) == p["reasoningEffort"].as_str())
            })
        || p["context"]["digest"] != context_digest(p)?
    {
        return Err(ValidationError);
    }
    let c = &p["context"];
    let base = c["baseRevision"].as_u64().ok_or(ValidationError)?;
    let rev = c["revision"].as_u64().ok_or(ValidationError)?;
    let entries = c["entries"].as_array().ok_or(ValidationError)?;
    let bindings = c["nativeReplyBindings"].as_array().ok_or(ValidationError)?;
    if entries.len() + bindings.len() > 2000
        || base >= rev
        || ((p["conversation"]["mode"] == "new") != (c["kind"] == "baseline"))
        || (c["kind"] == "baseline" && (base != 0 || !bindings.is_empty()))
    {
        return Err(ValidationError);
    }
    let mut ids = BTreeSet::new();
    let mut seqs = BTreeSet::new();
    for records in [entries, bindings] {
        let mut last = base;
        for e in records {
            let seq = e["sequence"].as_u64().ok_or(ValidationError)?;
            if seq <= last
                || seq > rev
                || !seqs.insert(seq)
                || !ids.insert(e["eventId"].as_str().ok_or(ValidationError)?.to_owned())
            {
                return Err(ValidationError);
            }
            if e.get("sourceTaskId").is_some() && e["generation"] != p["conversation"]["generation"]
            {
                return Err(ValidationError);
            }
            last = seq;
        }
    }
    let trigger = entries
        .iter()
        .filter(|e| e["eventId"] == p["triggerEventId"])
        .collect::<Vec<_>>();
    if trigger.len() != 1
        || trigger[0]["authorId"] != p["requesterId"]
        || trigger[0]["authorRole"] != "human"
        || trigger[0]["sequence"] != rev
        || trigger[0]["text"]
            .as_str()
            .ok_or(ValidationError)?
            .chars()
            .count()
            > 8000
    {
        return Err(ValidationError);
    }
    let mut decisions = BTreeSet::new();
    for d in c["activeDecisions"].as_array().ok_or(ValidationError)? {
        if !decisions.insert(d["decisionId"].clone().to_string())
            || d["sourceEventIds"]
                .as_array()
                .ok_or(ValidationError)?
                .iter()
                .any(|s| {
                    s.as_str()
                        .is_none_or(|s| !ids.contains(s) && !prior.contains(s))
                })
        {
            return Err(ValidationError);
        }
    }
    let issued = timestamp(&p["issuedAt"])?;
    let expires = timestamp(&p["expiresAt"])?;
    let auth = timestamp(&p["authorizationExpiresAt"])?;
    let lease = timestamp(&p["leaseExpiresAt"])?;
    if issued >= expires
        || expires > auth
        || expires - issued > chrono::Duration::seconds(180)
        || lease <= issued
        || (current && (Utc::now() >= expires || Utc::now() >= auth || Utc::now() >= lease))
    {
        return Err(ValidationError);
    }
    Ok(())
}
pub fn sources(p: &Value) -> BTreeSet<String> {
    p["context"]["entries"]
        .as_array()
        .into_iter()
        .flatten()
        .chain(
            p["context"]["nativeReplyBindings"]
                .as_array()
                .into_iter()
                .flatten(),
        )
        .filter_map(|e| e["eventId"].as_str().map(str::to_owned))
        .chain(
            p["context"]["activeDecisions"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|d| d["decisionId"].as_str().map(str::to_owned)),
        )
        .collect()
}
pub fn validate_result(r: &Value, p: &Value, prior: &BTreeSet<String>) -> Checked<()> {
    schema("result", r)?;
    if r["conversationId"] != p["conversation"]["id"]
        || r["generation"] != p["conversation"]["generation"]
        || r["consumedRevision"] != p["context"]["revision"]
        || r["contextDigest"] != p["context"]["digest"]
        || r["assistantText"].as_str().ok_or(ValidationError)?.len() > 65536
    {
        return Err(ValidationError);
    }
    let mut known = sources(p);
    known.extend(prior.iter().cloned());
    if r["citations"]
        .as_array()
        .ok_or(ValidationError)?
        .iter()
        .any(|c| c.as_str().is_none_or(|s| !known.contains(s)))
    {
        return Err(ValidationError);
    }
    let settings = &r["effectiveSettings"];
    if !settings.is_null() {
        let confirmed = settings["confirmation"] == "confirmed";
        for key in ["model", "reasoningEffort"] {
            if (confirmed || !settings[key].is_null()) && settings[key] != p[key] {
                return Err(ValidationError);
            }
        }
        if settings["reroutedModel"]
            .as_str()
            .is_some_and(|s| s.chars().count() > 128)
        {
            return Err(ValidationError);
        }
    }
    let usage = &r["usage"];
    if !usage.is_null()
        && (usage["model"] != p["model"]
            || (usage["freshness"] == "fresh" && usage["modelContextWindow"].is_null())
            || timestamp(&usage["reportedAt"])? < timestamp(&p["issuedAt"])?
            || timestamp(&usage["reportedAt"])? > timestamp(&p["expiresAt"])?)
    {
        return Err(ValidationError);
    }
    Ok(())
}
