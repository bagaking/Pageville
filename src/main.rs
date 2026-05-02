use axum::{
    body::Body,
    extract::{Path, Query, Request, State},
    http::{header, HeaderValue, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use base64::{engine::general_purpose::STANDARD as B64, Engine};
use chrono::{DateTime, NaiveDateTime, Utc};
use clap::{Args, Parser, Subcommand};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    env, fs,
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
        // Port 0 parses fine but means "any free port" to the OS, so the child
        // binds something random while every caller keeps probing :0. Treat it
        // as unset: a reachable default beats an unreachable daemon.
        .filter(|p| *p != 0)
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
    // WAL still serializes writers. With the default 0ms timeout an overlapping
    // publish and event POST makes the loser fail instantly with SQLITE_BUSY,
    // dropping a legitimate write that a brief wait would have landed.
    conn.busy_timeout(Duration::from_secs(5))?;
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
/// True when `s` has the shape of a snapshot id (12 lowercase-or-upper hex).
/// Shape alone is not enough to route as a version — see `serve_page`, which
/// also requires the snapshot to exist so a real file of the same name wins.
fn looks_like_version(s: &str) -> bool {
    s.len() == 12 && s.chars().all(|c| c.is_ascii_hexdigit())
}
/// Compare event timestamps as instants, not bytes. The stored `ts` is always
/// `+00:00`, but a caller may legitimately pass `...Z` or a `+08:00` offset for
/// the very same moment; a lexicographic compare silently drops those events.
/// Bounds are validated at the request boundary, so an unparseable one never
/// reaches here; unparseable input excludes rather than falling back to byte
/// order, which is the bug this exists to prevent.
fn ts_before(ts: &str, bound: &str) -> bool {
    match (parse_ts(ts), parse_ts(bound)) {
        (Some(a), Some(b)) => a < b,
        _ => false,
    }
}
fn ts_after(ts: &str, bound: &str) -> bool {
    match (parse_ts(ts), parse_ts(bound)) {
        (Some(a), Some(b)) => a > b,
        _ => false,
    }
}
/// Reject traversal by path COMPONENT, not by substring. `contains("..")` also
/// rejects legal names like `app.2..1.js` or `config..bak`, failing the whole
/// publish with a misleading "invalid path". A component check still catches
/// real traversal (`a/../b`, a leading `../`) and additionally rejects a bare
/// `.` component and backslashes.
fn safe_rel_path(p: &str) -> bool {
    !p.is_empty()
        && !p.starts_with('/')
        && !p.contains('\\')
        && p.split('/')
            .all(|seg| !seg.is_empty() && seg != ".." && seg != ".")
}
fn parse_ts(s: &str) -> Option<DateTime<Utc>> {
    if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
        return Some(dt.with_timezone(&Utc));
    }
    // Accept a bare `YYYY-MM-DDTHH:MM:SS[.fff]` bound as UTC, which is what a
    // user naturally types after copying a ts and trimming the offset.
    NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S%.f")
        .ok()
        .map(|n| n.and_utc())
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
    // Trust an existing object only if its size matches. A crash mid-write
    // leaves a truncated file that `exists()` alone would accept forever,
    // silently serving half a page. Size is enough here because the name is
    // the content hash: same name + same length means same bytes.
    if path.metadata().is_ok_and(|m| m.len() == bytes.len() as u64) {
        return Ok(());
    }
    // Write to a temp file and rename so a crash can never publish a partial
    // object under its final name. The temp name needs a per-CALL suffix, not
    // just the pid: concurrent publishes of identical content run as threads of
    // ONE daemon process, so a pid-only name collides and the loser's rename
    // fails with ENOENT after the winner renamed the shared temp away.
    static TMP_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let seq = TMP_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let tmp = dir.join(format!(".{}.{}.{}.tmp", hash, std::process::id(), seq));
    fs::write(&tmp, bytes)?;
    fs::rename(&tmp, &path)?;
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
    // A short timeout matters: without it a foreign process that accepts the
    // connection but never answers hangs the CLI forever instead of failing.
    let Ok(client) = reqwest::Client::builder()
        .timeout(Duration::from_secs(2))
        .build()
    else {
        return false;
    };
    client
        .get(format!("{}/api/v0/health", base_url(target)))
        .send()
        .await
        .map(|r| r.status().is_success())
        .unwrap_or(false)
}

/// Client for CLI -> daemon calls. Every request is bounded so a wedged or
/// foreign listener surfaces as an error rather than an indefinite hang.
fn http_client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|e| e.to_string())
}

/// Port the daemon will listen on for `target`, so auto-start binds the same
/// port the caller is probing. `None` when the target is not a loopback URL.
fn target_port(target: Option<&str>) -> Option<u16> {
    let url = reqwest::Url::parse(target?).ok()?;
    url.port_or_known_default()
}

struct StartLock {
    _file: fs::File,
}

fn try_start_lock(data: &FsPath) -> Result<Option<StartLock>, String> {
    let path = data.join("daemon.lock");
    // The file's CONTENT is deliberately empty: the lock is the OS advisory
    // flock held by the open handle, and writing a pid here only ever recorded
    // the short-lived CLI that won the race, not the daemon. Nothing reads it,
    // so a stale pid was pure misinformation for anyone inspecting the dir.
    let file = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)
        .map_err(|e| format!("cannot open daemon lock: {e}"))?;
    match file.try_lock() {
        Ok(()) => Ok(Some(StartLock { _file: file })),
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
            let mut cmd = std::process::Command::new(exe);
            cmd.arg("daemon")
                .arg("run")
                .env("PAGEVILLE_DATA_DIR", &data);
            // The child must bind the very port we are probing. Without this a
            // custom --target port spawns a daemon on the default port, so the
            // probe never converges and the stray daemon is orphaned.
            if let Some(p) = target_port(target) {
                cmd.env("PAGEVILLE_PORT", p.to_string());
            }
            // Keep the child's stderr so a bind failure (EADDRINUSE) is
            // reportable instead of vanishing into /dev/null.
            let spawned = cmd
                .stdout(Stdio::null())
                .stderr(Stdio::piped())
                .stdin(Stdio::null())
                .spawn();
            child = Some(spawned.map_err(|e| e.to_string())?);
        }
        if let Some(c) = child.as_mut() {
            if let Ok(Some(_)) = c.try_wait() {
                return Err(format!(
                    "daemon exited before becoming healthy{}",
                    child_stderr_hint(child.as_mut(), target)
                ));
            }
        }
        if health_at(target).await {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    Err(format!(
        "daemon did not become healthy{}",
        child_stderr_hint(child.as_mut(), target)
    ))
}

/// Best-effort detail for a failed auto-start: the child's own error line, or
/// a port hint when something else already holds the address.
fn child_stderr_hint(child: Option<&mut Child>, target: Option<&str>) -> String {
    let mut detail = String::new();
    if let Some(c) = child {
        // Only read the pipe once the child is gone. A child that is alive but
        // unreachable never closes its stderr, and `read_to_string` would block
        // the CLI forever — an unbounded hang is worse than a missing hint.
        let exited = matches!(c.try_wait(), Ok(Some(_)));
        if !exited {
            let _ = c.kill();
            let _ = c.wait();
        }
        if let Some(mut err) = c.stderr.take() {
            let mut buf = String::new();
            if std::io::Read::read_to_string(&mut err, &mut buf).is_ok() {
                let line = buf.trim();
                if !line.is_empty() {
                    detail = format!(": {}", line.lines().last().unwrap_or(line));
                }
            }
        }
    }
    if detail.is_empty() {
        return String::new();
    }
    let p = target_port(target).unwrap_or_else(port);
    if detail.contains("Address already in use") || detail.contains("os error 48") {
        return format!(
            "{} (port {} is already in use; stop that process or set PAGEVILLE_PORT)",
            detail, p
        );
    }
    detail
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
    let client = http_client()?;
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
        // file_type() from the DirEntry does NOT follow symlinks (unlike
        // p.is_file()). A link inside the published dir pointing at, say,
        // ~/.ssh/id_rsa would otherwise be dereferenced and its contents
        // uploaded into the CAS and served over HTTP. Skip links entirely.
        let ft = ent.file_type()?;
        if ft.is_symlink() {
            eprintln!(
                "warning: skipping symlink {} (publishing follows no links)",
                p.display()
            );
            continue;
        }
        if ft.is_dir() {
            collect_files(root, &p, out)?;
        } else if ft.is_file() {
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
    print_list(
        format!("{}/api/v0/pages", base_url(target.as_deref())),
        json_out,
    )
    .await
}
async fn cmd_versions(page: String, target: Option<String>, json_out: bool) -> Result<(), String> {
    ensure_daemon(target.as_deref()).await?;
    print_list(
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
    print!("{}", get_text(u).await?);
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
    // Two passes: validate and hash everything BEFORE writing any object.
    // Interleaving them wrote objects for the files that passed, then bailed on
    // a later bad one — leaving unreferenced blobs (up to the ~1.5MiB body cap
    // per bad request) that nothing ever reclaims, since snapshots are the only
    // reachability root and no snapshot row was ever committed.
    let mut decoded = Vec::with_capacity(req.files.len());
    for (path, encoded) in req.files {
        if !safe_rel_path(&path) {
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
        manifest.files.insert(path, hash.clone());
        decoded.push((hash, bytes));
    }
    for (hash, bytes) in &decoded {
        if ensure_object(&st.data, hash, bytes).is_err() {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error":"object write failed"})),
            )
                .into_response();
        }
    }
    let id = snapshot_hash(&page, &manifest);
    let created = Utc::now().to_rfc3339();
    let raw = serde_json::to_string(&manifest).unwrap();
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
    // An unparseable bound must not silently filter. Falling back to string
    // order here would answer "no events" for a typo, and a caller cannot tell
    // that apart from a genuinely empty range.
    for (name, raw) in [("since", &q.since), ("until", &q.until)] {
        if let Some(v) = raw {
            if parse_ts(v).is_none() {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(json!({"error": format!("{name} must be an RFC3339 timestamp")})),
                )
                    .into_response();
            }
        }
    }
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
            || q.since.as_ref().is_some_and(|v| ts_before(&row.ts, v))
            || q.until.as_ref().is_some_and(|v| ts_after(&row.ts, v))
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
    Json(json!({"status":"ok", "protocol":"v0", "version": env!("CARGO_PKG_VERSION")}))
}

/// Reject requests whose Host is not our own loopback address.
///
/// The daemon binds 127.0.0.1, but that alone is not a trust boundary: any web
/// page the user visits can reach it, and with a rebound DNS name (attacker.com
/// -> 127.0.0.1) the browser treats the response as same-origin and hands the
/// attacker every page's HITL payloads. A literal-loopback Host cannot be
/// produced by a rebinding attack, so pinning it closes that door.
async fn guard_host(req: Request, next: Next) -> Response {
    let expected = port();
    let ok = req
        .headers()
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|h| host_is_loopback(h, expected));
    if !ok {
        return (
            StatusCode::FORBIDDEN,
            Json(json!({"error":"host not allowed; use 127.0.0.1 or localhost"})),
        )
            .into_response();
    }
    next.run(req).await
}

fn host_is_loopback(host: &str, expected_port: u16) -> bool {
    // Split off the port, tolerating the bracketed IPv6 form.
    let (name, port_part) = match host.rsplit_once(':') {
        Some((n, p)) if !n.ends_with(']') || p.chars().all(|c| c.is_ascii_digit()) => (n, Some(p)),
        _ => (host, None),
    };
    let name = name.trim_start_matches('[').trim_end_matches(']');
    let name_ok = name == "127.0.0.1" || name == "localhost" || name == "::1";
    let port_ok = match port_part {
        Some(p) => p.parse::<u16>().is_ok_and(|p| p == expected_port),
        None => expected_port == 80,
    };
    name_ok && port_ok
}

/// Block cross-site writes. A state-changing POST with no JSON body is a CORS
/// "simple request", so a hostile page can submit it with no preflight to stop
/// it; that is a drive-by kill switch for `/shutdown`. Same-origin callers
/// either send no Origin or send ours.
fn origin_is_same(req: &Request) -> bool {
    let Some(origin) = req.headers().get(header::ORIGIN) else {
        return true; // non-browser clients (the CLI) send none
    };
    origin
        .to_str()
        .ok()
        .and_then(|o| reqwest::Url::parse(o).ok())
        .is_some_and(|u| {
            let host_ok = matches!(u.host_str(), Some("127.0.0.1") | Some("localhost"));
            host_ok && u.port_or_known_default() == Some(port())
        })
}

async fn guard_write_origin(req: Request, next: Next) -> Response {
    if !origin_is_same(&req) {
        return (
            StatusCode::FORBIDDEN,
            Json(json!({"error":"cross-origin write rejected"})),
        )
            .into_response();
    }
    next.run(req).await
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
        // Same existence rule as serve_page: a path segment shaped like a
        // snapshot id only counts as one if that snapshot exists.
        Some(v) if looks_like_version(v) => Some(v),
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
    } else if let Some((i, m)) = parts
        .get(1)
        .filter(|v| looks_like_version(v))
        // Shape alone must not claim the segment: a published file or dir
        // named like a snapshot id (hashed asset names look exactly like
        // this) would otherwise be permanently unreachable. Only route as a
        // version when that snapshot really exists for this page.
        .and_then(|v| snapshot_for_page(&st.data, page, Some(v)))
    {
        (i, m, 2)
    } else {
        match snapshot_for_page(&st.data, page, None) {
            Some((i, m)) => (i, m, 1),
            None => return StatusCode::NOT_FOUND.into_response(),
        }
    };
    let mut rel = parts.get(rel_start..).unwrap_or(&[]).join("/");
    // A directory-style URL (`/page/sub/`) leaves a trailing empty segment.
    // That is ordinary, not traversal, so resolve it to the directory index
    // before the path check rejects the empty component.
    if rel.ends_with('/') {
        rel.push_str("index.html");
    }
    if rel.is_empty() {
        rel = "index.html".into();
    }
    if !safe_rel_path(&rel) {
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
    // Text types need an explicit charset or the browser guesses, which
    // mojibakes any non-ASCII page (CJK content is the common case here).
    let content_type = match (mime.type_(), mime.subtype()) {
        (mime_guess::mime::TEXT, _)
        | (_, mime_guess::mime::JAVASCRIPT)
        | (_, mime_guess::mime::JSON) => format!("{}; charset=utf-8", mime),
        _ => mime.to_string(),
    };
    let mut response = Response::new(Body::from(bytes));
    *response.status_mut() = StatusCode::OK;
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_str(&content_type).unwrap(),
    );
    // Published pages are user content sharing one origin with the API and
    // with each other. nosniff stops a mislabelled upload from being run as
    // script; the CSP keeps a page from reaching outside the host.
    response.headers_mut().insert(
        "x-content-type-options",
        HeaderValue::from_static("nosniff"),
    );
    response.headers_mut().insert(
        "content-security-policy",
        HeaderValue::from_static(
            "default-src 'self' 'unsafe-inline' 'unsafe-eval' data: blob:; frame-ancestors 'self'",
        ),
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
        .route(
            "/api/v0/shutdown",
            post(api_shutdown).layer(middleware::from_fn(guard_write_origin)),
        )
        .route("/*path", get(serve_page))
        .layer(middleware::from_fn(guard_host))
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
                match http_client() {
                    Err(e) => Err(e),
                    Ok(client) => client
                        .post(format!(
                            "{}/api/v0/shutdown",
                            base_url(cli.target.as_deref())
                        ))
                        .send()
                        .await
                        .map_err(|e| e.to_string())
                        .map(|_| ()),
                }
            }
        }
    };
    if let Err(e) = result {
        eprintln!("error: {}", e);
        std::process::exit(1);
    }
}

/// GET a v0 endpoint, surfacing the server's `error` field on failure.
/// `error_for_status` alone would discard it and report only the code.
async fn get_text(url: impl reqwest::IntoUrl) -> Result<String, String> {
    let resp = http_client()?
        .get(url)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    let status = resp.status();
    let text = resp.text().await.map_err(|e| e.to_string())?;
    if !status.is_success() {
        let detail = serde_json::from_str::<Value>(&text)
            .ok()
            .and_then(|v| v["error"].as_str().map(str::to_string))
            .unwrap_or_else(|| text.trim().to_string());
        return Err(format!("{status}: {detail}"));
    }
    Ok(text)
}

async fn print_list(url: String, json_out: bool) -> Result<(), String> {
    let text = get_text(url).await?;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn port_zero_falls_back() {
        // Port 0 means "any free port" to the OS: the daemon would bind a
        // random port while every caller probed :0, hanging unboundedly.
        std::env::set_var("PAGEVILLE_PORT", "0");
        assert_eq!(port(), DEFAULT_PORT);
        std::env::set_var("PAGEVILLE_PORT", "abc");
        assert_eq!(port(), DEFAULT_PORT);
        std::env::set_var("PAGEVILLE_PORT", "18123");
        assert_eq!(port(), 18123);
        std::env::remove_var("PAGEVILLE_PORT");
        assert_eq!(port(), DEFAULT_PORT);
    }

    #[test]
    fn traversal_is_rejected_by_component_not_substring() {
        // Real traversal stays rejected.
        assert!(!safe_rel_path("../etc/passwd"));
        assert!(!safe_rel_path("a/../../b"));
        assert!(!safe_rel_path("a/.."));
        assert!(!safe_rel_path("/abs/path"));
        assert!(!safe_rel_path(""));
        assert!(!safe_rel_path("a//b"), "empty component");
        assert!(!safe_rel_path("./a"), "bare dot component");
        assert!(!safe_rel_path("a\\..\\b"), "backslash separator");
        // Legal names that merely CONTAIN `..` must publish (the bug: these
        // failed the whole snapshot with a misleading "invalid path").
        assert!(safe_rel_path("data..old.json"));
        assert!(safe_rel_path("app.2..1.js"));
        assert!(safe_rel_path("..lead"));
        assert!(safe_rel_path("trail.."));
        assert!(safe_rel_path("dir/a..b/file.txt"));
        assert!(safe_rel_path("index.html"));
        assert!(safe_rel_path(".env"));
    }

    #[test]
    fn slug_rules() {
        assert!(valid_slug("demo"));
        assert!(valid_slug("my-page-2"));
        assert!(!valid_slug(""));
        assert!(!valid_slug("Bad_Name"));
        assert!(!valid_slug("UPPER"));
        assert!(!valid_slug("api"), "reserved");
        assert!(!valid_slug("latest"), "reserved");
        assert!(!valid_slug(&"a".repeat(129)), "too long");
        assert!(valid_slug(&"a".repeat(128)));
    }

    #[test]
    fn version_shape() {
        assert!(looks_like_version("a1b2c3d4e5f6"));
        assert!(looks_like_version("0123456789ab"));
        assert!(!looks_like_version("a1b2c3d4e5f"), "11 chars");
        assert!(!looks_like_version("a1b2c3d4e5f6a"), "13 chars");
        assert!(!looks_like_version("g1b2c3d4e5f6"), "non-hex");
        assert!(!looks_like_version("index.html"));
    }

    #[test]
    fn timestamps_compare_as_instants_not_strings() {
        let stored = "2026-09-13T11:00:00+00:00";
        // Same instant, three spellings: none may change the answer.
        for bound in [
            "2026-09-13T11:00:00+00:00",
            "2026-09-13T11:00:00Z",
            "2026-09-13T19:00:00+08:00",
        ] {
            assert!(!ts_before(stored, bound), "{bound} must not be after");
            assert!(!ts_after(stored, bound), "{bound} must not be before");
        }
        // A +08:00 bound that is EARLIER in absolute time sorts LATER as a
        // string; this is the regression that silently dropped events.
        let until = "2026-09-13T18:00:00+08:00"; // == 10:00Z, before stored
        assert!(ts_after(stored, until), "11:00Z is after 10:00Z");
        assert!(stored < until, "string compare disagrees (the old bug)");
        assert!(ts_before("2026-09-13T09:00:00Z", stored));
        // Unparseable bounds never reach the comparators (rejected at the
        // request boundary) and must not resurrect byte ordering here: a typo
        // that silently answers "no events" is worse than an error.
        assert!(!ts_before("2026-01-01T00:00:00+00:00", "zzz"));
        assert!(!ts_after("2026-01-01T00:00:00+00:00", "zzz"));
        assert!(parse_ts("zzz").is_none());
        assert!(parse_ts("").is_none());
        // Both spellings a caller may reasonably type do parse.
        assert!(parse_ts("2026-09-13T11:00:00Z").is_some());
        assert!(parse_ts("2026-09-13T11:00:00").is_some());
    }

    #[test]
    fn host_guard_pins_loopback() {
        assert!(host_is_loopback("127.0.0.1:7777", 7777));
        assert!(host_is_loopback("localhost:7777", 7777));
        assert!(host_is_loopback("[::1]:7777", 7777));
        // The rebinding case: attacker name resolving to 127.0.0.1.
        assert!(!host_is_loopback("evil.example:7777", 7777));
        assert!(!host_is_loopback("evil.example", 7777));
        // Right name, wrong port is still not us.
        assert!(!host_is_loopback("127.0.0.1:9999", 7777));
        assert!(!host_is_loopback("127.0.0.1", 7777), "missing port");
    }

    #[test]
    fn target_port_drives_child_bind() {
        assert_eq!(target_port(Some("http://127.0.0.1:8888")), Some(8888));
        assert_eq!(target_port(Some("http://localhost:8080/")), Some(8080));
        assert_eq!(target_port(Some("http://127.0.0.1")), Some(80));
        assert_eq!(target_port(None), None);
    }

    #[test]
    fn loopback_targets_only() {
        assert!(validate_target(None).is_ok());
        assert!(validate_target(Some("http://127.0.0.1:7777")).is_ok());
        assert!(validate_target(Some("http://localhost:7777")).is_ok());
        assert!(validate_target(Some("http://example.com:7777")).is_err());
        assert!(validate_target(Some("https://127.0.0.1:7777")).is_err());
    }

    #[test]
    fn snapshot_id_is_page_scoped_and_stable() {
        let m = SnapshotManifest {
            files: BTreeMap::from([("index.html".into(), "hash".into())]),
            spa: false,
        };
        let a = snapshot_hash("one", &m);
        assert_eq!(a, snapshot_hash("one", &m), "same input, same id");
        assert_ne!(a, snapshot_hash("two", &m), "page must qualify the id");
        assert_eq!(a.len(), 12);
        let spa = SnapshotManifest {
            spa: true,
            ..m.clone()
        };
        assert_ne!(
            a,
            snapshot_hash("one", &spa),
            "spa flag is part of identity"
        );
    }
}
