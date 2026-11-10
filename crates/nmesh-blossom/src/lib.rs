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

/// The workspace uses one TLS provider (ring); installing it twice is harmless.
fn install_tls() {
    let _ = rustls::crypto::ring::default_provider().install_default();
}

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
        install_tls();
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

    /// Fetch a blob from the first of this client's (configured, so trusted) servers that has
    /// it, verifying its hash.
    pub async fn fetch(&self, sha: &str) -> anyhow::Result<Vec<u8>> {
        fetch_from(&self.servers, sha, &FetchPolicy::trusting(&self.servers)).await
    }
}

/// How far a fetch may reach. Server URLs can come from other parties (a `terms` tag hint, an
/// agent's advertised servers), so by default a fetch only goes to public addresses, follows no
/// redirects, and is bounded in time and size. Servers the user configured are listed in
/// `trusted` and may be anywhere (e.g. a dev server on loopback).
#[derive(Clone, Debug)]
pub struct FetchPolicy {
    /// Server base URLs (normalised without a trailing `/`) exempt from the address check.
    pub trusted: Vec<String>,
    /// Allow loopback, private, link-local and other non-public addresses for every server.
    pub allow_private: bool,
    /// Largest body accepted; a longer one is abandoned before it is fully read.
    pub max_bytes: usize,
    /// Per-server deadline for connecting and reading the whole body.
    pub timeout: std::time::Duration,
}

impl Default for FetchPolicy {
    fn default() -> Self {
        Self {
            trusted: Vec::new(),
            allow_private: false,
            max_bytes: MAX_BLOB,
            timeout: std::time::Duration::from_secs(10),
        }
    }
}

impl FetchPolicy {
    /// Trust `servers` (the caller's own configuration); everything else must be public.
    pub fn trusting(servers: &[String]) -> Self {
        Self {
            trusted: servers
                .iter()
                .map(|s| s.trim_end_matches('/').to_string())
                .collect(),
            ..Self::default()
        }
    }

    /// Any destination (tests and fully local setups).
    pub fn local() -> Self {
        Self {
            allow_private: true,
            ..Self::default()
        }
    }
}

/// Whether `ip` is a globally routable unicast address.
pub fn is_public(ip: std::net::IpAddr) -> bool {
    use std::net::IpAddr;
    match ip {
        IpAddr::V4(v4) => {
            let o = v4.octets();
            !(v4.is_loopback()
                || v4.is_private()
                || v4.is_link_local()
                || v4.is_broadcast()
                || v4.is_documentation()
                || v4.is_unspecified()
                || v4.is_multicast()
                || o[0] == 0
                || (o[0] == 100 && (o[1] & 0xc0) == 64) // 100.64/10 shared address space
                || (o[0] == 192 && o[1] == 0 && o[2] == 0) // 192.0.0/24
                || (o[0] == 198 && (o[1] & 0xfe) == 18) // 198.18/15 benchmarking
                || o[0] >= 240)
        }
        IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                return is_public(IpAddr::V4(v4));
            }
            let seg = v6.segments();
            !(v6.is_loopback()
                || v6.is_unspecified()
                || v6.is_multicast()
                || (seg[0] & 0xfe00) == 0xfc00 // unique local
                || (seg[0] & 0xffc0) == 0xfe80 // link local
                || (seg[0] == 0x2001 && seg[1] == 0x0db8)) // documentation
        }
    }
}

/// Resolve `server` and check it against `policy`; returns the host and the address to pin the
/// connection to (so a second DNS answer can't redirect it), or why it is refused.
async fn destination(
    server: &str,
    policy: &FetchPolicy,
) -> Result<(reqwest::Url, Option<(String, SocketAddr)>), String> {
    let url = reqwest::Url::parse(server).map_err(|e| format!("{server}: {e}"))?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(format!("{server}: only http and https are fetched"));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(format!("{server}: credentials in the URL"));
    }
    let host = url
        .host_str()
        .ok_or_else(|| format!("{server}: no host"))?
        .trim_start_matches('[')
        .trim_end_matches(']')
        .to_string();
    let port = url
        .port_or_known_default()
        .ok_or_else(|| format!("{server}: no port"))?;
    if policy.allow_private
        || policy
            .trusted
            .iter()
            .any(|t| t == server.trim_end_matches('/'))
    {
        return Ok((url, None));
    }
    let addrs: Vec<SocketAddr> = tokio::time::timeout(
        policy.timeout,
        tokio::net::lookup_host((host.as_str(), port)),
    )
    .await
    .map_err(|_| format!("{server}: name resolution timed out"))?
    .map_err(|e| format!("{server}: {e}"))?
    .collect();
    if addrs.is_empty() {
        return Err(format!("{server}: no address"));
    }
    if let Some(a) = addrs.iter().find(|a| !is_public(a.ip())) {
        return Err(format!(
            "{server}: resolves to a non-public address ({})",
            a.ip()
        ));
    }
    Ok((url, Some((host, addrs[0]))))
}

/// Fetch one blob from one server under `policy`.
async fn fetch_one(server: &str, sha: &str, policy: &FetchPolicy) -> Result<Vec<u8>, String> {
    let url = format!("{}/{sha}", server.trim_end_matches('/'));
    fetch_checked(server, &url, sha, policy).await
}

/// Fetch `url` under `policy` and return its body if its sha256 is `sha` (e.g. a card image
/// whose payload pins its hash).
pub async fn fetch_url(url: &str, sha: &str, policy: &FetchPolicy) -> anyhow::Result<Vec<u8>> {
    let base = reqwest::Url::parse(url)?;
    let origin = base.origin().ascii_serialization();
    fetch_checked(&origin, url, &sha.to_ascii_lowercase(), policy)
        .await
        .map_err(|e| anyhow::anyhow!(e))
}

/// GET `url` (on `server`, whose destination is checked) under `policy`, hash-checked.
async fn fetch_checked(
    server: &str,
    url: &str,
    sha: &str,
    policy: &FetchPolicy,
) -> Result<Vec<u8>, String> {
    let (_, pin) = destination(server, policy).await?;
    install_tls();
    let mut b = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(policy.timeout);
    if let Some((host, addr)) = &pin {
        b = b.resolve(host, *addr);
    }
    let http = b.build().map_err(|e| e.to_string())?;
    let work = async {
        let mut r = http
            .get(url)
            .send()
            .await
            .map_err(|e| format!("{server}: {e}"))?;
        if !r.status().is_success() {
            return Err(format!("{server}: {}", r.status()));
        }
        if r.content_length()
            .is_some_and(|n| n > policy.max_bytes as u64)
        {
            return Err(format!("{server}: larger than {} bytes", policy.max_bytes));
        }
        let mut body = Vec::new();
        while let Some(chunk) = r.chunk().await.map_err(|e| format!("{server}: {e}"))? {
            if body.len() + chunk.len() > policy.max_bytes {
                return Err(format!("{server}: larger than {} bytes", policy.max_bytes));
            }
            body.extend_from_slice(&chunk);
        }
        if sha256_hex(&body) != sha {
            return Err(format!("{server}: hash mismatch"));
        }
        Ok(body)
    };
    tokio::time::timeout(policy.timeout, work)
        .await
        .map_err(|_| format!("{server}: timed out"))?
}

/// Fetch `sha` from the first of `servers` that has it under `policy` (destination, redirects,
/// size and time bounds), verifying its hash.
pub async fn fetch_from(
    servers: &[String],
    sha: &str,
    policy: &FetchPolicy,
) -> anyhow::Result<Vec<u8>> {
    let mut why = Vec::new();
    for s in servers {
        match fetch_one(s, sha, policy).await {
            Ok(b) => return Ok(b),
            Err(e) => why.push(e),
        }
    }
    anyhow::bail!("blob {sha} is not retrievable: {}", why.join("; "))
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

    #[tokio::test]
    async fn untrusted_fetches_are_bounded() {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let srv = serve("127.0.0.1:0".parse().unwrap(), None).await.unwrap();
        let c = BlossomClient::new(vec![srv.url.clone()], Keys::generate());
        let big = vec![7u8; 4096];
        let (sha, _) = c.upload(&big, "application/octet-stream").await.unwrap();
        let servers = vec![srv.url.clone()];
        // A hint pointing at loopback isn't fetched unless the server is trusted.
        let e = fetch_from(&servers, &sha, &FetchPolicy::default())
            .await
            .unwrap_err();
        assert!(e.to_string().contains("non-public"), "{e}");
        assert_eq!(
            fetch_from(&servers, &sha, &FetchPolicy::trusting(&servers))
                .await
                .unwrap(),
            big
        );
        // Size bound: abandoned, not buffered.
        let small = FetchPolicy {
            max_bytes: 1024,
            ..FetchPolicy::local()
        };
        let e = fetch_from(&servers, &sha, &small).await.unwrap_err();
        assert!(e.to_string().contains("larger than"), "{e}");
        // Schemes and credentials.
        for bad in ["file:///etc", "ftp://example.com", "http://u:p@example.com"] {
            assert!(fetch_from(&[bad.to_string()], &sha, &FetchPolicy::local())
                .await
                .is_err());
        }
        srv.stop();

        // Redirects aren't followed, even to a trusted server.
        let target = srv_with(&big).await;
        let app = Router::new().route(
            "/{sha}",
            get({
                let to = target.url.clone();
                move |Path(p): Path<String>| async move {
                    (StatusCode::FOUND, [(header::LOCATION, format!("{to}/{p}"))]).into_response()
                }
            }),
        );
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let redir = format!("http://{}", l.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(l, app).await });
        let e = fetch_from(&[redir], &sha, &FetchPolicy::local())
            .await
            .unwrap_err();
        assert!(e.to_string().contains("302"), "{e}");
        target.stop();

        assert!(!is_public("10.1.2.3".parse().unwrap()));
        assert!(!is_public("169.254.169.254".parse().unwrap()));
        assert!(!is_public("::1".parse().unwrap()));
        assert!(!is_public("::ffff:127.0.0.1".parse().unwrap()));
        assert!(!is_public("fd00::1".parse().unwrap()));
        assert!(is_public("93.184.216.34".parse().unwrap()));
    }

    async fn srv_with(b: &[u8]) -> RunningServer {
        let srv = serve("127.0.0.1:0".parse().unwrap(), None).await.unwrap();
        BlossomClient::new(vec![srv.url.clone()], Keys::generate())
            .upload(b, "application/octet-stream")
            .await
            .unwrap();
        srv
    }
}
