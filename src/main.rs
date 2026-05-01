use axum::{
    body::Body,
    extract::{Path, Query, State},
    http::{header, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use base64::{engine::general_purpose::STANDARD as B64, Engine};
use chrono::Utc;
use clap::{Args, Parser, Subcommand};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    env, fs,
    io::Write,
    net::SocketAddr,
    path::{Path as FsPath, PathBuf},
    process::{Child, Stdio},
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::sync::oneshot;

const PROTOCOL: &str = "v0";
const DEFAULT_PORT: u16 = 7777;
const ATLAS_HTML: &str = include_str!("../assets/atlas.html");

#[derive(Parser, Debug)]
#[command(name = "pageville", version)]
struct Cli {
    #[arg(long, global = true)]
    target: Option<String>,
    #[arg(long, global = true)]
    json: bool,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    Publish(PublishArgs),
    Pages {
        #[command(subcommand)]
        command: PagesCommand,
    },
    Versions {
        #[command(subcommand)]
        command: VersionsCommand,
    },
    Events {
        #[command(subcommand)]
        command: EventsCommand,
    },
    Daemon {
        #[command(subcommand)]
        command: DaemonCommand,
    },
}

#[derive(Args, Debug)]
struct PublishArgs {
    dir: PathBuf,
    #[arg(long)]
    page: String,
    #[arg(long)]
    spa: bool,
}
#[derive(Subcommand, Debug)]
enum PagesCommand {
    List,
}
#[derive(Subcommand, Debug)]
enum VersionsCommand {
    List {
        #[arg(long)]
        page: String,
    },
}
#[derive(Subcommand, Debug)]
enum EventsCommand {
    Pull {
        #[arg(long)]
        page: String,
        #[arg(long)]
        session: Option<String>,
        #[arg(long)]
        since: Option<String>,
        #[arg(long)]
        until: Option<String>,
        #[arg(long)]
        version: Option<String>,
    },
}
#[derive(Subcommand, Debug)]
enum DaemonCommand {
    Run,
    Status,
    Start,
    Stop,
}

#[derive(Clone)]
struct AppState {
    data: PathBuf,
    shutdown: Arc<Mutex<Option<oneshot::Sender<()>>>>,
}

#[derive(Serialize, Deserialize, Clone)]
struct SnapshotManifest {
    files: BTreeMap<String, String>,
    spa: bool,
}
#[derive(Serialize, Deserialize)]
struct PublishRequest {
    files: BTreeMap<String, String>,
    spa: bool,
}
#[derive(Serialize, Deserialize)]
struct PublishResponse {
    page: String,
    snapshot_id: String,
    url: String,
    spa: bool,
}
#[derive(Serialize)]
struct PageInfo {
    page: String,
    latest: String,
}
#[derive(Serialize)]
struct SnapshotInfo {
    snapshot_id: String,
    page: String,
    created_at: String,
    spa: bool,
}
#[derive(Deserialize)]
struct EventRequest {
    version: String,
    session: String,
    payload: Value,
}
#[derive(Serialize, Deserialize, Clone)]
struct EventEnvelope {
    event_id: String,
    page: String,
    version: String,
    session: String,
    ts: String,
    identity: Option<Value>,
    payload: Value,
}
#[derive(Deserialize, Default)]
struct EventQuery {
    session: Option<String>,
    since: Option<String>,
    until: Option<String>,
    version: Option<String>,
}
#[derive(Deserialize)]
struct ContextQuery {
    path: Option<String>,
}

fn data_dir() -> PathBuf {
    env::var_os("PAGEVILLE_DATA_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            env::var_os("HOME")
                .map(|h| PathBuf::from(h).join(".pageville"))
                .unwrap_or_else(|| PathBuf::from(".pageville"))
        })
}
fn port() -> u16 {
    env::var("PAGEVILLE_PORT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(DEFAULT_PORT)
}
fn base_url(target: Option<&str>) -> String {
    target
        .map(|s| s.trim_end_matches('/').to_owned())
        .unwrap_or_else(|| format!("http://127.0.0.1:{}", port()))
}
fn validate_target(target: Option<&str>) -> Result<(), String> {
    if let Some(t) = target {
        if !(t.starts_with("http://127.0.0.1:") || t.starts_with("http://localhost:")) {
            return Err(
                "v0 --target only supports loopback http://127.0.0.1:<port> or localhost".into(),
            );
        }
    }
    Ok(())
}
fn db_path(data: &FsPath) -> PathBuf {
    data.join("pageville.db")
}
fn db(data: &FsPath) -> rusqlite::Result<Connection> {
    fs::create_dir_all(data).map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
    let conn = Connection::open(db_path(data))?;
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.execute_batch("CREATE TABLE IF NOT EXISTS pages(page TEXT PRIMARY KEY, latest TEXT NOT NULL); CREATE TABLE IF NOT EXISTS snapshots(id TEXT PRIMARY KEY, page TEXT NOT NULL, created_at TEXT NOT NULL, spa INTEGER NOT NULL, manifest TEXT NOT NULL); CREATE TABLE IF NOT EXISTS events(event_id TEXT PRIMARY KEY, page TEXT NOT NULL, version TEXT NOT NULL, session TEXT NOT NULL, ts TEXT NOT NULL, identity TEXT, payload TEXT NOT NULL);")?;
    Ok(conn)
}
fn valid_slug(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 128
        && s.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        && s != "api"
        && s != "latest"
}
fn snapshot_hash(page: &str, manifest: &SnapshotManifest) -> String {
    // Keep the storage key globally unique even when two pages publish the
    // same files. Object hashes remain page-independent; only the snapshot
    // namespace is page-qualified.
    let bytes = serde_json::to_vec(&(page, manifest)).expect("manifest serializable");
    blake3::hash(&bytes).to_hex().to_string()[..12].to_string()
}
fn ensure_object(data: &FsPath, hash: &str, bytes: &[u8]) -> std::io::Result<()> {
    let dir = data.join("objects");
    fs::create_dir_all(&dir)?;
    let path = dir.join(hash);
    if !path.exists() {
        fs::write(path, bytes)?;
    }
    Ok(())
}
fn snapshot_for_page(
    data: &FsPath,
    page: &str,
    version: Option<&str>,
) -> Option<(String, SnapshotManifest)> {
    let conn = db(data).ok()?;
    let id = match version {
        Some(v) if v != "latest" => v.to_owned(),
        _ => conn
            .query_row("SELECT latest FROM pages WHERE page=?1", [page], |r| {
                r.get(0)
            })
            .ok()?,
    };
    let raw: String = conn
        .query_row(
            "SELECT manifest FROM snapshots WHERE id=?1 AND page=?2",
            params![id, page],
            |r| r.get(0),
        )
        .ok()?;
    Some((id, serde_json::from_str(&raw).ok()?))
}

async fn health_at(target: Option<&str>) -> bool {
    reqwest::get(format!("{}/api/v0/health", base_url(target)))
        .await
        .map(|r| r.status().is_success())
        .unwrap_or(false)
}

struct StartLock {
    _file: fs::File,
}

fn try_start_lock(data: &FsPath) -> Result<Option<StartLock>, String> {
    let path = data.join("daemon.lock");
    let mut file = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)
        .map_err(|e| format!("cannot open daemon lock: {e}"))?;
    match file.try_lock() {
        Ok(()) => {
            file.set_len(0)
                .and_then(|_| writeln!(file, "{}", std::process::id()))
                .map_err(|e| format!("cannot update daemon lock: {e}"))?;
            Ok(Some(StartLock { _file: file }))
        }
        Err(std::fs::TryLockError::WouldBlock) => Ok(None),
        Err(std::fs::TryLockError::Error(e)) => Err(format!("cannot acquire daemon lock: {e}")),
    }
}

struct PidFileGuard {
    path: PathBuf,
    value: String,
}

impl Drop for PidFileGuard {
    fn drop(&mut self) {
        if fs::read_to_string(&self.path)
            .ok()
            .is_some_and(|value| value.trim() == self.value)
        {
            let _ = fs::remove_file(&self.path);
        }
    }
}

async fn ensure_daemon(target: Option<&str>) -> Result<(), String> {
    if health_at(target).await {
        return Ok(());
    }
    let data = data_dir();
    fs::create_dir_all(&data).map_err(|e| e.to_string())?;
    let mut child: Option<Child> = None;
    let mut start_lock = try_start_lock(&data)?;
    for _ in 0..50 {
        if health_at(target).await {
            return Ok(());
        }
        if start_lock.is_none() {
            start_lock = try_start_lock(&data)?;
            if start_lock.is_some() && health_at(target).await {
                return Ok(());
            }
        }
        if start_lock.is_some() && child.is_none() {
            let exe = env::current_exe().map_err(|e| e.to_string())?;
            let spawned = std::process::Command::new(exe)
                .arg("daemon")
                .arg("run")
                .env("PAGEVILLE_DATA_DIR", &data)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .stdin(Stdio::null())
                .spawn();
            child = Some(spawned.map_err(|e| e.to_string())?);
        }
        if let Some(c) = child.as_mut() {
            if let Ok(Some(_)) = c.try_wait() {
                return Err("daemon exited before becoming healthy".into());
            }
        }
        if health_at(target).await {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    Err("daemon did not become healthy".into())
}

async fn cmd_publish(
    args: PublishArgs,
    target: Option<String>,
    json_out: bool,
) -> Result<(), String> {
    if !valid_slug(&args.page) {
        return Err("invalid page slug".into());
    }
    ensure_daemon(target.as_deref()).await?;
    let mut files = BTreeMap::new();
    collect_files(&args.dir, &args.dir, &mut files).map_err(|e| e.to_string())?;
    let client = reqwest::Client::new();
    let req = PublishRequest {
        files,
        spa: args.spa,
    };
    let url = format!(
        "{}/api/v0/pages/{}/snapshots",
        base_url(target.as_deref()),
        args.page
    );
    let out: PublishResponse = client
        .post(url)
        .json(&req)
        .send()
        .await
        .map_err(|e| e.to_string())?
        .error_for_status()
        .map_err(|e| e.to_string())?
        .json()
        .await
        .map_err(|e| e.to_string())?;
    if json_out {
        println!("{}", serde_json::to_string(&out).unwrap());
    } else {
        println!("snapshot_id={} url={}", out.snapshot_id, out.url);
    }
    Ok(())
}
fn collect_files(
    root: &FsPath,
    dir: &FsPath,
    out: &mut BTreeMap<String, String>,
) -> std::io::Result<()> {
    for ent in fs::read_dir(dir)? {
        let ent = ent?;
        let p = ent.path();
        if p.is_dir() {
            collect_files(root, &p, out)?;
        } else if p.is_file() {
            let rel = p
                .strip_prefix(root)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            out.insert(rel, B64.encode(fs::read(p)?));
        }
    }
    Ok(())
}

async fn cmd_pages(target: Option<String>, json_out: bool) -> Result<(), String> {
    ensure_daemon(target.as_deref()).await?;
    futuresless_get(
        format!("{}/api/v0/pages", base_url(target.as_deref())),
        json_out,
    )
    .await
}
async fn cmd_versions(page: String, target: Option<String>, json_out: bool) -> Result<(), String> {
    ensure_daemon(target.as_deref()).await?;
    futuresless_get(
        format!(
            "{}/api/v0/pages/{}/snapshots",
            base_url(target.as_deref()),
            page
        ),
        json_out,
    )
    .await
}
async fn cmd_events(
    page: String,
    session: Option<String>,
    since: Option<String>,
    until: Option<String>,
    version: Option<String>,
    target: Option<String>,
) -> Result<(), String> {
    if !valid_slug(&page) {
        return Err("invalid page slug".into());
    }
    ensure_daemon(target.as_deref()).await?;
    let mut u = reqwest::Url::parse(&format!(
        "{}/api/v0/pages/{}/events",
        base_url(target.as_deref()),
        page
    ))
    .map_err(|e| e.to_string())?;
    {
        let mut query = u.query_pairs_mut();
        if let Some(v) = session.as_deref() {
            query.append_pair("session", v);
        }
        if let Some(v) = since.as_deref() {
            query.append_pair("since", v);
        }
        if let Some(v) = until.as_deref() {
            query.append_pair("until", v);
        }
        if let Some(v) = version.as_deref() {
            query.append_pair("version", v);
        }
    }
    let txt = reqwest::get(u)
        .await
        .map_err(|e| e.to_string())?
        .text()
        .await
        .map_err(|e| e.to_string())?;
    print!("{}", txt);
    Ok(())
}

async fn api_publish(
    State(st): State<AppState>,
    Path(page): Path<String>,
    Json(req): Json<PublishRequest>,
) -> impl IntoResponse {
    if !valid_slug(&page) {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error":"invalid page slug"})),
        )
            .into_response();
    }
    let mut manifest = SnapshotManifest {
        files: BTreeMap::new(),
        spa: req.spa,
    };
    for (path, encoded) in req.files {
        if path.is_empty() || path.starts_with('/') || path.contains("..") {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({"error":"invalid path"})),
            )
                .into_response();
        }
        let bytes = match B64.decode(encoded) {
            Ok(b) => b,
            Err(_) => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(json!({"error":"invalid base64"})),
                )
                    .into_response()
            }
        };
        let hash = blake3::hash(&bytes).to_hex().to_string();
        if ensure_object(&st.data, &hash, &bytes).is_err() {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error":"object write failed"})),
            )
                .into_response();
        }
        manifest.files.insert(path, hash);
    }
    let id = snapshot_hash(&page, &manifest);
    let created = Utc::now().to_rfc3339();
    let raw = serde_json::to_string(&manifest).unwrap();
    if fs::create_dir_all(st.data.join("manifests")).is_err()
        || fs::write(st.data.join("manifests").join(format!("{}.json", id)), &raw).is_err()
    {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error":"manifest write failed"})),
        )
            .into_response();
    }
    let conn = match db(&st.data) {
        Ok(c) => c,
        Err(_) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error":"db unavailable"})),
            )
                .into_response()
        }
    };
    let tx = match conn.unchecked_transaction() {
        Ok(t) => t,
        Err(_) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error":"transaction failed"})),
            )
                .into_response()
        }
    };
    if tx
        .execute(
            "INSERT OR IGNORE INTO snapshots(id,page,created_at,spa,manifest) VALUES(?1,?2,?3,?4,?5)",
            params![&id, &page, &created, manifest.spa as i32, &raw],
        )
        .is_err()
    {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error":"snapshot write failed"})),
        )
            .into_response();
    }
    let existing = match tx.query_row(
        "SELECT page,manifest FROM snapshots WHERE id=?1",
        [&id],
        |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
    ) {
        Ok(value) => value,
        Err(_) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error":"snapshot write failed"})),
            )
                .into_response()
        }
    };
    if existing.0 != page || existing.1 != raw {
        return (
            StatusCode::CONFLICT,
            Json(json!({"error":"snapshot id collision"})),
        )
            .into_response();
    }
    if tx
        .execute(
            "INSERT INTO pages(page,latest) VALUES(?1,?2) ON CONFLICT(page) DO UPDATE SET latest=excluded.latest",
            params![&page, &id],
        )
        .is_err()
    {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error":"latest update failed"})),
        )
            .into_response();
    }
    if tx.commit().is_err() {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error":"commit failed"})),
        )
            .into_response();
    }
    let out = PublishResponse {
        page: page.clone(),
        snapshot_id: id.clone(),
        url: format!("http://127.0.0.1:{}/{}/", port(), page),
        spa: manifest.spa,
    };
    (StatusCode::OK, Json(out)).into_response()
}

async fn api_pages(State(st): State<AppState>) -> impl IntoResponse {
    let Ok(conn) = db(&st.data) else {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error":"db unavailable"})),
        )
            .into_response();
    };
    let mut stmt = conn
        .prepare("SELECT page,latest FROM pages ORDER BY page")
        .unwrap();
    let rows = stmt
        .query_map([], |r| {
            Ok(PageInfo {
                page: r.get(0)?,
                latest: r.get(1)?,
            })
        })
        .unwrap();
    let out: Vec<_> = rows.filter_map(Result::ok).collect();
    Json(out).into_response()
}
async fn api_snapshots(State(st): State<AppState>, Path(page): Path<String>) -> impl IntoResponse {
    let Ok(conn) = db(&st.data) else {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error":"db unavailable"})),
        )
            .into_response();
    };
    let mut stmt = conn
        .prepare(
            "SELECT id,page,created_at,spa FROM snapshots WHERE page=?1 ORDER BY created_at DESC",
        )
        .unwrap();
    let rows = stmt
        .query_map([page], |r| {
            Ok(SnapshotInfo {
                snapshot_id: r.get(0)?,
                page: r.get(1)?,
                created_at: r.get(2)?,
                spa: r.get::<_, i32>(3)? != 0,
            })
        })
        .unwrap();
    let out: Vec<_> = rows.filter_map(Result::ok).collect();
    Json(out).into_response()
}
async fn api_event_post(
    State(st): State<AppState>,
    Path(page): Path<String>,
    Json(req): Json<EventRequest>,
) -> impl IntoResponse {
    if req.version == "latest" || snapshot_for_page(&st.data, &page, Some(&req.version)).is_none() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error":"version must be an existing snapshot id"})),
        )
            .into_response();
    }
    let ev = EventEnvelope {
        event_id: format!("evt_{}", ulid::Ulid::new()),
        page: page.clone(),
        version: req.version,
        session: req.session,
        ts: Utc::now().to_rfc3339(),
        identity: None,
        payload: req.payload,
    };
    let Ok(conn) = db(&st.data) else {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error":"db unavailable"})),
        )
            .into_response();
    };
    if conn.execute("INSERT INTO events(event_id,page,version,session,ts,identity,payload) VALUES(?1,?2,?3,?4,?5,NULL,?6)", params![ev.event_id, ev.page, ev.version, ev.session, ev.ts, serde_json::to_string(&ev.payload).unwrap()]).is_err() { return (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error":"event write failed"}))).into_response(); }
    Json(ev).into_response()
}
async fn api_event_get(
    State(st): State<AppState>,
    Path(page): Path<String>,
    Query(q): Query<EventQuery>,
) -> impl IntoResponse {
    let Ok(conn) = db(&st.data) else {
        return (StatusCode::INTERNAL_SERVER_ERROR, "").into_response();
    };
    let mut stmt = match conn.prepare(
        "SELECT event_id,version,session,ts,payload FROM events WHERE page=?1 ORDER BY ts,event_id",
    ) {
        Ok(s) => s,
        Err(_) => return (StatusCode::INTERNAL_SERVER_ERROR, "").into_response(),
    };
    let rows = stmt
        .query_map([page.clone()], |r| {
            let payload: String = r.get(4)?;
            Ok(EventEnvelope {
                event_id: r.get(0)?,
                page: page.clone(),
                version: r.get(1)?,
                session: r.get(2)?,
                ts: r.get(3)?,
                identity: None,
                payload: serde_json::from_str(&payload).unwrap_or(Value::Null),
            })
        })
        .unwrap();
    let mut body = String::new();
    for row in rows.flatten() {
        if q.session.as_ref().is_some_and(|v| v != &row.session)
            || q.version.as_ref().is_some_and(|v| v != &row.version)
            || q.since.as_ref().is_some_and(|v| &row.ts < v)
            || q.until.as_ref().is_some_and(|v| &row.ts > v)
        {
            continue;
        }
        body.push_str(&serde_json::to_string(&row).unwrap());
        body.push('\n');
    }
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/x-ndjson")
        .body(Body::from(body))
        .unwrap()
}
async fn api_health() -> impl IntoResponse {
    Json(json!({"status":"ok", "protocol":"v0"}))
}

async fn atlas_root() -> Response {
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
        .header("cache-control", "no-store")
        .header("x-content-type-options", "nosniff")
        .body(Body::from(ATLAS_HTML))
        .expect("static atlas response is valid")
}
async fn api_context(
    State(st): State<AppState>,
    Query(q): Query<ContextQuery>,
) -> impl IntoResponse {
    let path = q.path.unwrap_or_default();
    let parts: Vec<_> = path.trim_matches('/').split('/').collect();
    if parts.is_empty() || parts[0].is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error":"path required"})),
        )
            .into_response();
    }
    let page = parts[0];
    let version = match parts.get(1).copied() {
        Some("latest") | None => None,
        Some(v) if v.len() == 12 && v.chars().all(|c| c.is_ascii_hexdigit()) => Some(v),
        _ => None,
    };
    let Some((id, _)) = snapshot_for_page(&st.data, page, version) else {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({"error":"page not found"})),
        )
            .into_response();
    };
    Json(json!({"page":page,"version":id})).into_response()
}

async fn serve_page(State(st): State<AppState>, Path(path): Path<String>) -> Response {
    let parts: Vec<_> = path.split('/').collect();
    if parts.is_empty() {
        return StatusCode::NOT_FOUND.into_response();
    }
    let page = parts[0];
    if !valid_slug(page) {
        return StatusCode::NOT_FOUND.into_response();
    }
    let (version, manifest, rel_start) = if parts.get(1).is_some_and(|v| *v == "latest") {
        match snapshot_for_page(&st.data, page, None) {
            Some((i, m)) => (i, m, 2),
            None => return StatusCode::NOT_FOUND.into_response(),
        }
    } else if parts
        .get(1)
        .is_some_and(|v| v.len() == 12 && v.chars().all(|c| c.is_ascii_hexdigit()))
    {
        match snapshot_for_page(&st.data, page, parts.get(1).copied()) {
            Some((i, m)) => (i, m, 2),
            None => return StatusCode::NOT_FOUND.into_response(),
        }
    } else {
        match snapshot_for_page(&st.data, page, None) {
            Some((i, m)) => (i, m, 1),
            None => return StatusCode::NOT_FOUND.into_response(),
        }
    };
    let mut rel = parts.get(rel_start..).unwrap_or(&[]).join("/");
    if rel.is_empty() {
        rel = "index.html".into();
    }
    if rel.starts_with('/') || rel.contains("..") {
        return StatusCode::BAD_REQUEST.into_response();
    }
    let (served_rel, hash) = if let Some(hash) = manifest.files.get(&rel) {
        (rel.clone(), hash.clone())
    } else if manifest.spa {
        let Some(hash) = manifest.files.get("index.html") else {
            return StatusCode::NOT_FOUND.into_response();
        };
        ("index.html".to_owned(), hash.clone())
    } else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let Ok(bytes) = fs::read(st.data.join("objects").join(hash)) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let mime = mime_guess::from_path(&served_rel).first_or_octet_stream();
    let mut response = Response::new(Body::from(bytes));
    *response.status_mut() = StatusCode::OK;
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_str(mime.as_ref()).unwrap(),
    );
    response
        .headers_mut()
        .insert("x-pageville-page", HeaderValue::from_str(page).unwrap());
    response.headers_mut().insert(
        "x-pageville-version",
        HeaderValue::from_str(&version).unwrap(),
    );
    response
}
async fn api_shutdown(State(st): State<AppState>) -> impl IntoResponse {
    if let Some(tx) = st.shutdown.lock().unwrap().take() {
        let _ = tx.send(());
    }
    Json(json!({"status":"stopping"}))
}

async fn run_daemon() -> Result<(), String> {
    let data = data_dir();
    fs::create_dir_all(data.join("objects")).map_err(|e| e.to_string())?;
    if health_at(None).await {
        return Err("daemon already running".into());
    }
    let _ = db(&data).map_err(|e| e.to_string())?;
    let (tx, rx) = oneshot::channel();
    let pid_path = data.join("daemon.pid");
    let pid_value = std::process::id().to_string();
    fs::write(&pid_path, &pid_value).map_err(|e| e.to_string())?;
    let _pid_guard = PidFileGuard {
        path: pid_path,
        value: pid_value,
    };
    let state = AppState {
        data: data.clone(),
        shutdown: Arc::new(Mutex::new(Some(tx))),
    };
    let app = Router::new()
        .route("/", get(atlas_root))
        .route("/api/v0/health", get(api_health))
        .route("/api/v0/pages", get(api_pages))
        .route(
            "/api/v0/pages/:page/snapshots",
            post(api_publish).get(api_snapshots),
        )
        .route(
            "/api/v0/pages/:page/events",
            post(api_event_post).get(api_event_get),
        )
        .route("/api/v0/context", get(api_context))
        .route("/api/v0/shutdown", post(api_shutdown))
        .route("/*path", get(serve_page))
        .with_state(state);
    let addr = SocketAddr::from(([127, 0, 0, 1], port()));
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .map_err(|e| e.to_string())?;
    let result = axum::serve(listener, app)
        .with_graceful_shutdown(async {
            let _ = rx.await;
        })
        .await;
    result.map_err(|e| e.to_string())
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    if let Err(e) = validate_target(cli.target.as_deref()) {
        eprintln!("error: {}", e);
        std::process::exit(1);
    }
    let result = match cli.command {
        Command::Publish(a) => cmd_publish(a, cli.target, cli.json).await,
        Command::Pages {
            command: PagesCommand::List,
        } => cmd_pages(cli.target.clone(), cli.json).await,
        Command::Versions {
            command: VersionsCommand::List { page },
        } => cmd_versions(page, cli.target.clone(), cli.json).await,
        Command::Events {
            command:
                EventsCommand::Pull {
                    page,
                    session,
                    since,
                    until,
                    version,
                },
        } => cmd_events(page, session, since, until, version, cli.target.clone()).await,
        Command::Daemon {
            command: DaemonCommand::Run,
        } => run_daemon().await,
        Command::Daemon {
            command: DaemonCommand::Status,
        } => {
            let ok = health_at(cli.target.as_deref()).await;
            if cli.json {
                println!("{}", json!({"running":ok,"protocol":PROTOCOL}));
            } else {
                println!("{}", if ok { "running" } else { "stopped" });
            }
            Ok(())
        }
        Command::Daemon {
            command: DaemonCommand::Start,
        } => ensure_daemon(cli.target.as_deref()).await,
        Command::Daemon {
            command: DaemonCommand::Stop,
        } => {
            if !health_at(cli.target.as_deref()).await {
                Ok(())
            } else {
                reqwest::Client::new()
                    .post(format!(
                        "{}/api/v0/shutdown",
                        base_url(cli.target.as_deref())
                    ))
                    .send()
                    .await
                    .map_err(|e| e.to_string())
                    .map(|_| ())
            }
        }
    };
    if let Err(e) = result {
        eprintln!("error: {}", e);
        std::process::exit(1);
    }
}

async fn futuresless_get(url: String, json_out: bool) -> Result<(), String> {
    let text = reqwest::get(url)
        .await
        .map_err(|e| e.to_string())?
        .error_for_status()
        .map_err(|e| e.to_string())?
        .text()
        .await
        .map_err(|e| e.to_string())?;
    if json_out {
        println!("{}", text);
    } else if let Ok(v) = serde_json::from_str::<Value>(&text) {
        if let Some(arr) = v.as_array() {
            for x in arr {
                println!("{}", serde_json::to_string(x).unwrap());
            }
        } else {
            println!("{}", text);
        }
    } else {
        print!("{}", text);
    }
    Ok(())
}
