//! A minimal Blossom server for development, and a client (Core.md §9: artifacts are stored on
//! Blossom, sha256-addressed, and `REPORT` lists them in `x` tags).
//!
//! Server: BUD-01 `GET /<sha256>` and `HEAD /<sha256>` (an extension after the hash is
//! ignored); BUD-02 `PUT /upload`, authorized by a kind 24242 event in
//! `Authorization: Nostr <base64(event)>` with a `t` tag `upload`, an `x` tag equal to the
//! body's sha256 and an `expiration` in the future. Blobs live in memory, or in a directory.
//! Like `nmesh relay`, it is a dev server: no quotas, no deletion, no listing.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, put};
use axum::{Json, Router};
use base64::Engine;
use nostr::{Event, EventBuilder, JsonUtil, Keys, Kind, Tag, TagKind, Timestamp};
use serde_json::json;
use sha2::{Digest, Sha256};

/// Largest blob the dev server accepts.
pub const MAX_BLOB: usize = 16 * 1024 * 1024;
/// Upload authorization kind (BUD-02).
pub const AUTH_KIND: u16 = 24242;
/// User server list (BUD-03): where an identity's blobs are.
pub const SERVER_LIST_KIND: u16 = 10063;

pub fn sha256_hex(b: &[u8]) -> String {
    hex::encode(Sha256::digest(b))
}

/// sha256 → (bytes, media type).
type Blobs = HashMap<String, (Vec<u8>, String)>;

#[derive(Clone, Default)]
struct Store {
    mem: Arc<Mutex<Blobs>>,
    dir: Option<PathBuf>,
}

impl Store {
    fn put(&self, sha: &str, bytes: &[u8], mime: &str) -> std::io::Result<()> {
        match &self.dir {
            Some(d) => {
                std::fs::write(d.join(sha), bytes)?;
                std::fs::write(d.join(format!("{sha}.type")), mime)
            }
            None => {
                self.mem
                    .lock()
                    .unwrap()
                    .insert(sha.into(), (bytes.to_vec(), mime.into()));
                Ok(())
            }
        }
    }
    fn get(&self, sha: &str) -> Option<(Vec<u8>, String)> {
        match &self.dir {
            Some(d) => {
                let b = std::fs::read(d.join(sha)).ok()?;
                let t = std::fs::read_to_string(d.join(format!("{sha}.type")))
                    .unwrap_or_else(|_| "application/octet-stream".into());
                Some((b, t))
            }
            None => self.mem.lock().unwrap().get(sha).cloned(),
        }
    }
}

#[derive(Clone)]
struct AppState {
    store: Store,
    base: String,
}

/// A running server; dropping it doesn't stop it, `stop` does.
pub struct RunningServer {
    pub url: String,
    task: tokio::task::JoinHandle<()>,
}

impl RunningServer {
    pub fn stop(self) {
        self.task.abort();
    }
}

/// Serve on `listen` (port 0 picks one). `dir = None` keeps blobs in memory.
pub async fn serve(listen: SocketAddr, dir: Option<PathBuf>) -> anyhow::Result<RunningServer> {
    if let Some(d) = &dir {
        std::fs::create_dir_all(d)?;
    }
    let listener = tokio::net::TcpListener::bind(listen).await?;
    let url = format!("http://{}", listener.local_addr()?);
    let state = AppState {
        store: Store {
            dir,
            ..Default::default()
        },
        base: url.clone(),
    };
    let app = Router::new()
        .route("/upload", put(upload))
        .route("/{blob}", get(fetch).head(head))
        .with_state(state);
    let task = tokio::spawn(async move {
        if let Err(e) = axum::serve(listener, app).await {
            tracing::error!(error = %e, "blossom server stopped");
        }
    });
    Ok(RunningServer { url, task })
}

fn hash_of(blob: &str) -> Option<&str> {
    let h = blob.split('.').next()?;
    (h.len() == 64 && h.bytes().all(|b| b.is_ascii_hexdigit())).then_some(h)
}

async fn fetch(State(s): State<AppState>, Path(blob): Path<String>) -> Response {
    match hash_of(&blob).and_then(|h| s.store.get(&h.to_lowercase())) {
        Some((b, t)) => ([(header::CONTENT_TYPE, t)], b).into_response(),
        None => (StatusCode::NOT_FOUND, "blob not found").into_response(),
    }
}

async fn head(State(s): State<AppState>, Path(blob): Path<String>) -> Response {
    match hash_of(&blob).and_then(|h| s.store.get(&h.to_lowercase())) {
        Some((b, t)) => (
            [
                (header::CONTENT_TYPE, t),
                (header::CONTENT_LENGTH, b.len().to_string()),
            ],
            (),
        )
            .into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

/// BUD-02 authorization: a signed kind 24242 for `upload` of exactly this blob, not expired.
fn authorize(headers: &HeaderMap, sha: &str) -> Result<(), String> {
    let h = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .ok_or("missing Authorization")?;
    let b64 = h
        .strip_prefix("Nostr ")
        .ok_or("Authorization must be `Nostr <base64>`")?;
    let raw = base64::engine::general_purpose::STANDARD
        .decode(b64.trim())
        .map_err(|_| "authorization is not base64")?;
    let ev = Event::from_json(raw).map_err(|_| "authorization is not an event")?;
    ev.verify().map_err(|_| "bad authorization signature")?;
    if ev.kind != Kind::Custom(AUTH_KIND) {
        return Err("authorization must be kind 24242".into());
    }
    let tag = |k: &str| {
        ev.tags
            .iter()
            .filter(move |t| t.kind() == TagKind::custom(k))
            .filter_map(|t| t.content().map(str::to_owned))
            .collect::<Vec<_>>()
    };
    if !tag("t").iter().any(|t| t == "upload") {
        return Err("authorization is not for upload".into());
    }
    if !tag("x").iter().any(|x| x == sha) {
        return Err("authorization doesn't name this blob's sha256".into());
    }
    let exp = tag("expiration")
        .first()
        .and_then(|e| e.parse::<u64>().ok())
        .ok_or("authorization needs an expiration")?;
    if exp <= Timestamp::now().as_secs() {
        return Err("authorization expired".into());
    }
    Ok(())
}

async fn upload(State(s): State<AppState>, headers: HeaderMap, body: Bytes) -> Response {
    if body.len() > MAX_BLOB {
        return (StatusCode::PAYLOAD_TOO_LARGE, "blob too large").into_response();
    }
    let sha = sha256_hex(&body);
    if let Err(e) = authorize(&headers, &sha) {
        return (StatusCode::UNAUTHORIZED, [("X-Reason", e.clone())], e).into_response();
    }
    let mime = headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("application/octet-stream")
        .to_string();
    if let Err(e) = s.store.put(&sha, &body, &mime) {
        return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response();
    }
    Json(json!({
        "url": format!("{}/{sha}", s.base),
        "sha256": sha,
        "size": body.len(),
        "type": mime,
        "uploaded": Timestamp::now().as_secs(),
    }))
    .into_response()
}

/// Uploads to, and fetches from, a list of servers; every fetch is checked against its hash.
#[derive(Clone)]
pub struct BlossomClient {
    pub servers: Vec<String>,
    keys: Keys,
    http: reqwest::Client,
}

impl BlossomClient {
    pub fn new(servers: Vec<String>, keys: Keys) -> Self {
        Self {
            servers: servers
                .into_iter()
                .map(|s| s.trim_end_matches('/').to_string())
                .collect(),
            keys,
            http: reqwest::Client::new(),
        }
    }

    fn auth(&self, sha: &str) -> anyhow::Result<String> {
        let ev = EventBuilder::new(Kind::Custom(AUTH_KIND), "Upload blob")
            .tags([
                Tag::parse(["t", "upload"])?,
                Tag::parse(["x", sha])?,
                Tag::parse([
                    "expiration".to_string(),
                    (Timestamp::now().as_secs() + 300).to_string(),
                ])?,
            ])
            .sign_with_keys(&self.keys)?;
        Ok(format!(
            "Nostr {}",
            base64::engine::general_purpose::STANDARD.encode(ev.as_json())
        ))
    }

    /// Upload to every server (replication, Core §9); succeeds if at least one accepted it.
    /// Returns the sha256 and the servers that hold it.
    pub async fn upload(&self, bytes: &[u8], mime: &str) -> anyhow::Result<(String, Vec<String>)> {
        let sha = sha256_hex(bytes);
        let mut ok = Vec::new();
        let mut last = String::new();
        for s in &self.servers {
            let r = self
                .http
                .put(format!("{s}/upload"))
                .header(header::AUTHORIZATION, self.auth(&sha)?)
                .header(header::CONTENT_TYPE, mime)
                .body(bytes.to_vec())
                .send()
                .await;
            match r {
                Ok(r) if r.status().is_success() => ok.push(s.clone()),
                Ok(r) => last = format!("{s}: {}", r.status()),
                Err(e) => last = format!("{s}: {e}"),
            }
        }
        if ok.is_empty() {
            anyhow::bail!("no Blossom server accepted the blob ({last})");
        }
        Ok((sha, ok))
    }

    /// Fetch a blob from the first server that has it, verifying its hash.
    pub async fn fetch(&self, sha: &str) -> anyhow::Result<Vec<u8>> {
        fetch_from(&self.http, &self.servers, sha).await
    }
}

/// Fetch `sha` from any of `servers` (no key needed), verifying its hash.
pub async fn fetch_from(
    http: &reqwest::Client,
    servers: &[String],
    sha: &str,
) -> anyhow::Result<Vec<u8>> {
    for s in servers {
        let Ok(r) = http
            .get(format!("{}/{sha}", s.trim_end_matches('/')))
            .send()
            .await
        else {
            continue;
        };
        if !r.status().is_success() {
            continue;
        }
        let b = r.bytes().await?.to_vec();
        if sha256_hex(&b) == sha {
            return Ok(b);
        }
    }
    anyhow::bail!("blob {sha} is not retrievable from {servers:?}")
}

/// The BUD-03 server list event for `servers`.
pub fn server_list(servers: &[String]) -> EventBuilder {
    EventBuilder::new(Kind::Custom(SERVER_LIST_KIND), "").tags(
        servers
            .iter()
            .filter_map(|s| Tag::parse(["server", s.as_str()]).ok()),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn upload_requires_auth_and_fetch_checks_the_hash() {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let srv = serve("127.0.0.1:0".parse().unwrap(), None).await.unwrap();
        let c = BlossomClient::new(vec![srv.url.clone()], Keys::generate());
        let (sha, held) = c.upload(b"hello artifact", "text/plain").await.unwrap();
        assert_eq!(sha, sha256_hex(b"hello artifact"));
        assert_eq!(held, vec![srv.url.clone()]);
        assert_eq!(c.fetch(&sha).await.unwrap(), b"hello artifact");
        // No auth: refused.
        let r = reqwest::Client::new()
            .put(format!("{}/upload", srv.url))
            .body("x")
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), StatusCode::UNAUTHORIZED);
        // Auth for another blob: refused.
        let r = reqwest::Client::new()
            .put(format!("{}/upload", srv.url))
            .header(header::AUTHORIZATION, c.auth(&sha).unwrap())
            .body("something else")
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), StatusCode::UNAUTHORIZED);
        assert!(c.fetch(&"00".repeat(32)).await.is_err());
        srv.stop();
    }
}
