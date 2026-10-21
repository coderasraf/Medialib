//! The app's backend: an axum server on 127.0.0.1 serving the single-page
//! UI and its JSON API. Used both by the desktop window and by
//! `medialib serve`.
//!
//! Why local and plain HTTP: the media server is usually on a LAN address.
//! A page served from any public origin has no route to it, an HTTPS page is
//! blocked from loading HTTP media (mixed content), and listing servers send
//! no CORS headers. A page on http://127.0.0.1 has none of those problems.
//! `<video src>` and `<img src>` need no CORS at all, so the page can load
//! posters and stream video straight from the media server.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, RwLock, RwLockReadGuard};

use anyhow::{anyhow, bail, Context, Result};
use axum::body::Body;
use axum::extract::{Path as UrlPath, Query, Request, State};
use axum::http::{header, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::{get, post, put};
use axum::{Json, Router};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::net::TcpListener;
use tower::ServiceExt;
use tower_http::services::ServeFile;
use url::Url;

use crate::assistant;
use crate::config::{Config, Root, SourceKind};
use crate::crawler::{self, percent_decode};
use crate::index::Index;
use crate::jobs::{self, Job, Jobs};
use crate::library::{self, Query as TitleQuery};
use crate::player;
use crate::tmdb::{self, Tmdb};

// `rusqlite::Connection` is `Send` but not `Sync`; the Mutex is what lets
// handlers on different threads share it. Handlers take a guard, use it and
// drop it within one statement or block, never across an `.await` (a std
// guard isn't `Send`, so the compiler would refuse anyway).
#[derive(Clone)]
struct AppState {
    index: Arc<Mutex<Index>>,
    config: Arc<RwLock<Config>>,
    config_path: Arc<PathBuf>,
    jobs: Jobs,
}

impl AppState {
    // A panic elsewhere shouldn't brick the UI, so poisoned locks are
    // recovered rather than propagated. SQLite rolls back unfinished
    // transactions itself, so no invariant spans a panic.
    fn index(&self) -> MutexGuard<'_, Index> {
        self.index.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn config(&self) -> RwLockReadGuard<'_, Config> {
        self.config.read().unwrap_or_else(PoisonError::into_inner)
    }

    /// Runs `f` against the shared connection.
    fn db<T>(&self, f: impl FnOnce(&Connection) -> Result<T>) -> Result<T, ApiError> {
        f(self.index().conn()).map_err(ApiError::internal)
    }

    /// Applies an edit to a copy of the config, validates it, writes it to
    /// disk, and only then makes it current, so a rejected edit changes
    /// nothing and the file is always loadable.
    fn update_config(&self, edit: impl FnOnce(&mut Config) -> Result<()>) -> Result<(), ApiError> {
        let mut current = self.config.write().unwrap_or_else(PoisonError::into_inner);
        let mut next = current.clone();
        edit(&mut next).map_err(ApiError::bad_request)?;
        next.normalise().map_err(ApiError::bad_request)?;
        next.save(&self.config_path).map_err(ApiError::internal)?;
        *current = next;
        Ok(())
    }

    /// A second connection for heavy work run on the blocking pool.
    fn open_index(&self) -> Result<Index> {
        Index::open(self.config().db_file())
    }
}

/// A bound-but-not-yet-running server, so the desktop app can learn the
/// port (it asks the OS for a free one) before opening its window.
pub struct Server {
    listener: TcpListener,
    app: Router,
}

impl Server {
    pub async fn bind(config: Config, config_path: PathBuf, port: u16) -> Result<Server> {
        let index = Index::open(config.db_file())?;
        let config = Arc::new(RwLock::new(config));
        let state = AppState {
            index: Arc::new(Mutex::new(index)),
            jobs: Jobs::start(Arc::clone(&config)),
            config,
            config_path: Arc::new(config_path),
        };
        // Loopback only: /api/play starts a process on this machine, so it
        // must not be reachable from the network.
        let addr = SocketAddr::from(([127, 0, 0, 1], port));
        let listener = TcpListener::bind(addr)
            .await
            .with_context(|| format!("binding {addr}"))?;
        Ok(Server {
            listener,
            app: router(state),
        })
    }

    pub fn addr(&self) -> Result<SocketAddr> {
        Ok(self.listener.local_addr()?)
    }

    pub async fn run(self) -> Result<()> {
        axum::serve(self.listener, self.app).await?;
        Ok(())
    }
}

fn router(state: AppState) -> Router {
    Router::new()
        .route("/", get(|| async { Html(INDEX_HTML) }))
        .route("/app.css", get(|| async { ([(header::CONTENT_TYPE, "text/css")], APP_CSS) }))
        .route("/app.js", get(|| async { ([(header::CONTENT_TYPE, "text/javascript")], APP_JS) }))
        .route("/api/state", get(get_state))
        .route("/api/home", get(home))
        .route("/api/titles", get(titles))
        .route("/api/facets", get(facets))
        .route("/api/titles/{id}", get(title_detail))
        .route("/api/titles/{id}/match", post(rematch))
        .route("/api/recent", get(recent))
        .route("/api/watchlist", post(watchlist))
        .route("/api/watched", post(watched))
        .route("/api/play", post(play))
        .route("/api/playlist", post(playlist))
        .route("/api/open-folder", post(open_folder))
        .route("/api/open-url", post(open_url))
        .route("/api/pick", post(pick))
        .route("/api/sources", post(add_source))
        .route("/api/sources/check", post(check_source))
        .route("/api/sources/{name}", put(edit_source).delete(delete_source))
        .route("/api/sources/{name}/sync", post(sync_source))
        .route("/api/sources/{name}/stop", post(stop_source))
        .route("/api/sync", post(sync_all))
        .route("/api/jobs/stop", post(stop_jobs))
        .route("/api/settings", get(get_settings))
        .route("/api/settings/player", put(save_player))
        .route("/api/settings/tmdb", put(save_tmdb))
        .route("/api/settings/assistant", put(save_assistant))
        .route("/api/settings/library", put(save_library))
        .route("/api/tmdb/test", post(tmdb_test))
        .route("/api/tmdb/lookup", post(tmdb_lookup))
        .route("/api/assistant", get(chat_history).post(chat).delete(chat_clear))
        .route("/api/assistant/models", get(models))
        .route("/api/storage", get(storage))
        .route("/api/storage/{action}", post(storage_action))
        .route("/media/{id}", get(media))
        .route("/poster/{id}", get(poster))
        .layer(middleware::from_fn(require_local_host))
        .with_state(state)
}

/// Rejects requests whose Host header isn't loopback. Binding to 127.0.0.1
/// stops other machines; this stops DNS rebinding, where a hostile web page
/// points its own hostname at 127.0.0.1 to talk to us same-origin.
async fn require_local_host(req: Request, next: Next) -> Response {
    let host = req
        .headers()
        .get(header::HOST)
        .and_then(|h| h.to_str().ok())
        .unwrap_or("");
    let hostname = match host.rsplit_once(':') {
        Some((h, port)) if port.chars().all(|c| c.is_ascii_digit()) => h,
        _ => host,
    };
    if matches!(hostname, "127.0.0.1" | "localhost" | "[::1]") {
        next.run(req).await
    } else {
        ApiError::new(StatusCode::FORBIDDEN, anyhow!("unexpected Host header")).into_response()
    }
}

type ApiResult<T = Json<Value>> = Result<T, ApiError>;

fn ok() -> ApiResult {
    Ok(Json(json!({ "ok": true })))
}

// ---------------------------------------------------------------- state ---

#[derive(Serialize)]
struct SourceView {
    name: String,
    kind: SourceKind,
    location: String,
    web: bool,
    count: i64,
    last_sync: Option<i64>,
    status: Option<String>,
    error: Option<String>,
    syncing: bool,
    queued: bool,
}

async fn get_state(State(st): State<AppState>) -> ApiResult {
    let counts = st.db(library::counts)?;
    let states: Vec<(String, Option<i64>, Option<String>, Option<String>)> = st.db(|c| {
        let mut stmt = c.prepare("SELECT root, last_sync, status, error FROM sources_state")?;
        let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    })?;
    let job = st.jobs.status();
    let cfg = st.config().clone();
    let sources: Vec<SourceView> = cfg
        .roots
        .iter()
        .map(|r| {
            let s = states.iter().find(|s| s.0 == r.name);
            SourceView {
                name: r.name.clone(),
                kind: r.kind,
                location: r.location().to_string(),
                web: r.url.is_some(),
                count: counts.per_root.get(&r.name).copied().unwrap_or(0),
                last_sync: s.and_then(|s| s.1),
                status: s.and_then(|s| s.2.clone()),
                error: s.and_then(|s| s.3.clone()),
                syncing: job.current_root.as_deref() == Some(&r.name),
                queued: job.queued_roots.contains(&r.name),
            }
        })
        .collect();
    let mut job = job;
    let log_len = job.log.len();
    job.log.drain(..log_len.saturating_sub(120));
    Ok(Json(json!({
        "counts": counts,
        "sources": sources,
        "job": job,
        "flags": {
            "tmdb": !cfg.tmdb.api_key.is_empty(),
            "assistant": !cfg.assistant.api_key.is_empty(),
            "model": cfg.assistant.model,
            "prefer_builtin": cfg.player.prefer_builtin,
            "player": cfg.player.command,
        },
    })))
}

async fn home(State(st): State<AppState>) -> ApiResult {
    let rows = st.db(library::home)?;
    Ok(Json(json!({ "rows": rows })))
}

async fn titles(State(st): State<AppState>, Query(q): Query<TitleQuery>) -> ApiResult {
    let page = st.db(|c| library::titles(c, &q))?;
    Ok(Json(serde_json::to_value(page).map_err(|e| ApiError::internal(e.into()))?))
}

async fn facets(State(st): State<AppState>, Query(q): Query<TitleQuery>) -> ApiResult {
    let f = st.db(|c| library::facets(c, &q))?;
    Ok(Json(serde_json::to_value(f).map_err(|e| ApiError::internal(e.into()))?))
}

async fn recent(State(st): State<AppState>) -> ApiResult {
    let items = st.db(|c| library::recently_played(c, 200))?;
    Ok(Json(json!({ "total": items.len(), "items": items })))
}

async fn title_detail(State(st): State<AppState>, UrlPath(id): UrlPath<i64>) -> ApiResult {
    // Episode titles and stills come from TMDB a season at a time, fetched
    // the first time someone opens the show rather than for every show
    // during sync.
    let (tmdb_cfg, kind_id) = {
        let cfg = st.config().tmdb.clone();
        let info = st.db(|c| {
            Ok(c.query_row(
                "SELECT kind, tmdb_id FROM titles WHERE id = ?1",
                [id],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, Option<i64>>(1)?)),
            )
            .ok())
        })?;
        (cfg, info)
    };
    let Some((kind, tmdb_id)) = kind_id else {
        return Err(ApiError::new(StatusCode::NOT_FOUND, anyhow!("no such title")));
    };
    if let (Some(tv), true) = (tmdb_id, kind == "show" && !tmdb_cfg.api_key.is_empty()) {
        let missing = st.db(|c| tmdb::missing_seasons(c, id))?;
        if !missing.is_empty() {
            if let Ok(client) = Tmdb::new(&tmdb_cfg) {
                for season in missing {
                    // A failed fetch just leaves generic episode names.
                    if let Ok(eps) = client.season(tv, season).await {
                        let _ = tmdb::store_season(st.index().conn_mut(), id, season, &eps);
                    }
                }
            }
        }
    }
    let detail = st
        .db(|c| library::detail(c, id))?
        .ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, anyhow!("no such title")))?;
    Ok(Json(serde_json::to_value(detail).map_err(|e| ApiError::internal(e.into()))?))
}

#[derive(Deserialize)]
struct MatchRequest {
    #[serde(default)]
    query: String,
    year: Option<u32>,
    tmdb_id: Option<i64>,
}

/// "Wrong info?": search TMDB again with the user's own query, or take an
/// id / themoviedb.org URL directly.
async fn rematch(
    State(st): State<AppState>,
    UrlPath(id): UrlPath<i64>,
    Json(req): Json<MatchRequest>,
) -> ApiResult {
    let cfg = st.config().tmdb.clone();
    let client = Tmdb::new(&cfg).map_err(ApiError::bad_request)?;
    let kind: String = st.db(|c| Ok(c.query_row("SELECT kind FROM titles WHERE id = ?1", [id], |r| r.get(0))?))?;
    let tmdb_id = req.tmdb_id.or_else(|| tmdb_id_from_text(&req.query));
    let meta = match tmdb_id {
        Some(t) => Some(client.details(&kind, t).await.map_err(ApiError::bad_request)?),
        None => {
            if req.query.trim().is_empty() {
                return Err(ApiError::bad_request(anyhow!("type a title to search for")));
            }
            client
                .lookup(&kind, req.query.trim(), req.year)
                .await
                .map_err(ApiError::bad_request)?
        }
    };
    let Some(meta) = meta else {
        return Err(ApiError::bad_request(anyhow!("nothing found on TMDB for that")));
    };
    st.db(|c| {
        tmdb::store(c, id, Some(&meta))?;
        c.execute("DELETE FROM episode_meta WHERE title_id = ?1", [id])?;
        c.execute("DELETE FROM season_fetched WHERE title_id = ?1", [id])?;
        Ok(())
    })?;
    Ok(Json(json!({ "ok": true, "title": meta.title })))
}

/// Accepts "1396", "tv/1396" or a full themoviedb.org URL.
fn tmdb_id_from_text(text: &str) -> Option<i64> {
    let t = text.trim();
    if let Ok(n) = t.parse() {
        return Some(n);
    }
    let after = t
        .split(['/', '?'])
        .skip_while(|s| !matches!(*s, "movie" | "tv"))
        .nth(1)?;
    after.split('-').next()?.parse().ok()
}

#[derive(Deserialize)]
struct Toggle {
    #[serde(default)]
    title_id: i64,
    #[serde(default)]
    file_id: i64,
    on: bool,
}

async fn watchlist(State(st): State<AppState>, Json(t): Json<Toggle>) -> ApiResult {
    st.db(|c| library::set_watchlist(c, t.title_id, t.on))?;
    ok()
}

async fn watched(State(st): State<AppState>, Json(t): Json<Toggle>) -> ApiResult {
    st.db(|c| library::set_watched(c, t.file_id, t.on))?;
    ok()
}

// ----------------------------------------------------------------- play ---

#[derive(Deserialize)]
struct PlayRequest {
    file_id: Option<i64>,
    url: Option<String>,
    /// The UI played it in its own player; just record the play.
    #[serde(default)]
    record_only: bool,
}

async fn play(State(st): State<AppState>, Json(req): Json<PlayRequest>) -> ApiResult {
    // Only play what was indexed. Even with the scheme check, an open
    // "launch the player on any URL" endpoint is more power than the UI needs.
    let file = st.db(|c| match (req.file_id, &req.url) {
        (Some(id), _) => Ok(library::file_info(c, id)?.map(|f| (id, f.0))),
        (None, Some(url)) => Ok(c
            .query_row("SELECT id, url FROM files WHERE url = ?1", [url], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .ok()),
        _ => Ok(None),
    })?;
    let Some((file_id, url)) = file else {
        if let Some(url) = &req.url {
            player::validate_url(url).map_err(ApiError::bad_request)?;
        }
        return Err(ApiError::bad_request(anyhow!("that video is not in the library")));
    };
    if !req.record_only {
        let player_cfg = st.config().player.clone();
        player::play_indexed(&player_cfg, &url).map_err(ApiError::bad_request)?;
    }
    st.db(|c| library::record_play(c, file_id))?;
    ok()
}

#[derive(Deserialize)]
struct PlaylistRequest {
    title_id: i64,
    from_file_id: Option<i64>,
    season: Option<u32>,
}

/// Writes an M3U playlist of a title's files and opens it in the player,
/// so a whole season plays back to back.
async fn playlist(State(st): State<AppState>, Json(req): Json<PlaylistRequest>) -> ApiResult {
    let (files, name) = st.db(|c| {
        let mut files = library::title_files(c, req.title_id)?;
        if let Some(season) = req.season {
            let in_season: Vec<i64> = {
                let mut stmt = c.prepare("SELECT id FROM files WHERE title_id = ?1 AND season = ?2")?;
                let rows = stmt.query_map(rusqlite::params![req.title_id, season], |r| r.get(0))?;
                rows.collect::<rusqlite::Result<_>>()?
            };
            files.retain(|f| in_season.contains(&f.0));
        }
        if let Some(from) = req.from_file_id {
            if let Some(pos) = files.iter().position(|f| f.0 == from) {
                files.drain(..pos);
            }
        }
        let name: String = c.query_row("SELECT name FROM titles WHERE id = ?1", [req.title_id], |r| r.get(0))?;
        Ok((files, name))
    })?;
    if files.is_empty() {
        return Err(ApiError::bad_request(anyhow!("nothing to play")));
    }
    let mut m3u = String::from("#EXTM3U\n");
    for (_, url, file_name) in &files {
        let target = match Url::parse(url) {
            Ok(u) if u.scheme() == "file" => u
                .to_file_path()
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_else(|_| url.clone()),
            _ => url.clone(),
        };
        m3u.push_str(&format!("#EXTINF:-1,{name} - {file_name}\n{target}\n"));
    }
    let (path, player_cfg) = {
        let cfg = st.config();
        (cfg.base_dir.join("playlist.m3u8"), cfg.player.clone())
    };
    std::fs::write(&path, m3u).map_err(|e| ApiError::internal(e.into()))?;
    player::play_file(&player_cfg, &path).map_err(ApiError::bad_request)?;
    let first = files[0].0;
    st.db(|c| library::record_play(c, first))?;
    Ok(Json(json!({ "ok": true, "count": files.len() })))
}

#[derive(Deserialize)]
struct FolderRequest {
    title_id: Option<i64>,
    path: Option<String>,
}

/// Opens a title's folder: Explorer for local folders, the browser for
/// folders on a media server.
async fn open_folder(State(st): State<AppState>, Json(req): Json<FolderRequest>) -> ApiResult {
    let target = match (req.title_id, req.path) {
        (Some(id), _) => st
            .db(|c| library::title_dir(c, id))?
            .ok_or_else(|| ApiError::bad_request(anyhow!("no such title")))?,
        // Only the app's own data folder may be opened by path.
        (None, Some(p)) if Path::new(&p) == st.config().base_dir => p,
        _ => return Err(ApiError::bad_request(anyhow!("nothing to open"))),
    };
    let target = match Url::parse(&target) {
        Ok(u) if u.scheme() == "file" => u
            .to_file_path()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or(target),
        _ => target,
    };
    open_external(&target).map_err(ApiError::internal)?;
    ok()
}

#[derive(Deserialize)]
struct UrlRequest {
    url: String,
}

/// Opens help links in the real browser. Allow-listed, so the endpoint
/// can't be used to launch arbitrary URLs.
async fn open_url(Json(req): Json<UrlRequest>) -> ApiResult {
    let url = Url::parse(&req.url).map_err(|e| ApiError::bad_request(e.into()))?;
    let allowed = ["www.themoviedb.org", "aistudio.google.com", "www.videolan.org"];
    if url.scheme() != "https" || !allowed.contains(&url.host_str().unwrap_or("")) {
        return Err(ApiError::bad_request(anyhow!("that link can't be opened from here")));
    }
    open_external(url.as_str()).map_err(ApiError::internal)?;
    ok()
}

fn open_external(target: &str) -> Result<()> {
    let mut cmd = if cfg!(windows) {
        let mut c = std::process::Command::new("explorer");
        c.arg(target);
        c
    } else if cfg!(target_os = "macos") {
        let mut c = std::process::Command::new("open");
        c.arg(target);
        c
    } else {
        let mut c = std::process::Command::new("xdg-open");
        c.arg(target);
        c
    };
    cmd.spawn().context("opening")?;
    Ok(())
}

#[derive(Deserialize)]
struct PickRequest {
    kind: String,
}

/// Native folder / program picker. The dialog blocks, so it runs on the
/// blocking pool rather than an async worker.
async fn pick(Json(req): Json<PickRequest>) -> ApiResult {
    let path = tokio::task::spawn_blocking(move || match req.kind.as_str() {
        "program" => rfd::FileDialog::new()
            .set_title("Choose your video player")
            .add_filter("Programs", &["exe"])
            .pick_file(),
        _ => rfd::FileDialog::new()
            .set_title("Choose a folder with videos")
            .pick_folder(),
    })
    .await
    .map_err(|e| ApiError::internal(anyhow!(e)))?;
    Ok(Json(json!({ "path": path.map(|p| p.display().to_string()) })))
}

// -------------------------------------------------------------- sources ---

#[derive(Deserialize)]
struct SourceRequest {
    #[serde(default)]
    name: String,
    #[serde(default)]
    location: String,
    #[serde(default)]
    kind: SourceKind,
    #[serde(default = "yes")]
    sync: bool,
}

fn yes() -> bool {
    true
}

/// "http://..." is a web source; anything else is a local folder.
fn split_location(location: &str) -> Result<(Option<String>, Option<String>)> {
    let loc = location.trim();
    if loc.is_empty() {
        bail!("paste a folder link or choose a folder");
    }
    if loc.starts_with("http://") || loc.starts_with("https://") {
        Ok((Some(loc.to_string()), None))
    } else if Path::new(loc).is_dir() {
        Ok((None, Some(loc.to_string())))
    } else {
        bail!("that is neither a web link (http://...) nor a folder on this PC")
    }
}

async fn add_source(State(st): State<AppState>, Json(req): Json<SourceRequest>) -> ApiResult {
    let (url, path) = split_location(&req.location).map_err(ApiError::bad_request)?;
    let location = url.clone().or(path.clone()).unwrap_or_default();
    let mut name = req.name.trim().to_string();
    {
        let cfg = st.config();
        if let Some(existing) = cfg.roots.iter().find(|r| same_location(r.location(), &location)) {
            return Err(ApiError::bad_request(anyhow!(
                "already added as \"{}\"",
                existing.name
            )));
        }
        if name.is_empty() {
            name = default_name(&location);
        }
        name = unique_name(&cfg, &name);
    }
    st.update_config(|cfg| {
        cfg.roots.push(Root {
            name: name.clone(),
            url,
            path,
            kind: req.kind,
        });
        Ok(())
    })?;
    if req.sync {
        st.jobs.enqueue(Job::Sync {
            root: name.clone(),
            full: false,
        });
    }
    Ok(Json(json!({ "ok": true, "name": name })))
}

#[derive(Deserialize)]
struct CheckRequest {
    location: String,
}

/// "Check link": fetches the listing once and reports what's there, with a
/// guess at whether it holds movies or shows.
async fn check_source(State(st): State<AppState>, Json(req): Json<CheckRequest>) -> ApiResult {
    let (url, path) = split_location(&req.location).map_err(ApiError::bad_request)?;
    let cfg = st.config().clone();
    let (dirs, files): (Vec<String>, Vec<String>) = if let Some(u) = url {
        let mut base = Url::parse(&u).map_err(|e| ApiError::bad_request(e.into()))?;
        if !base.path().ends_with('/') {
            base.set_path(&format!("{}/", base.path()));
        }
        let client = crawler::client(&cfg).map_err(ApiError::internal)?;
        let resp = client
            .get(base.clone())
            .send()
            .await
            .and_then(|r| r.error_for_status())
            .map_err(|e| ApiError::bad_request(anyhow!("couldn't open the link: {e}")))?;
        let html = resp.text().await.map_err(|e| ApiError::bad_request(e.into()))?;
        let listing = crawler::parse_listing(&base, &html);
        (
            listing.dirs.into_iter().map(|d| d.name).collect(),
            listing.files.into_iter().map(|f| f.name).collect(),
        )
    } else {
        let dir = PathBuf::from(path.unwrap_or_default());
        let mut dirs = Vec::new();
        let mut files = Vec::new();
        for e in std::fs::read_dir(&dir).map_err(|e| ApiError::bad_request(e.into()))?.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            match e.file_type() {
                Ok(t) if t.is_dir() => dirs.push(name),
                Ok(t) if t.is_file() => files.push(name),
                _ => {}
            }
        }
        (dirs, files)
    };
    let videos = files.iter().filter(|f| cfg.is_media(f)).count();
    if dirs.is_empty() && videos == 0 {
        return Err(ApiError::bad_request(anyhow!(
            "the link works, but there are no folders or videos in it"
        )));
    }
    let showish = dirs
        .iter()
        .filter(|d| {
            let l = d.to_lowercase();
            l.contains("tv series") || l.contains("tv mini") || l.contains("season") || l.contains("web series")
        })
        .count();
    let kind = if showish * 3 >= dirs.len().max(1) && showish > 0 { "shows" } else { "movies" };
    let suggested = default_name(&req.location);
    Ok(Json(json!({
        "ok": true,
        "folders": dirs.len(),
        "videos": videos,
        "kind": kind,
        "name": suggested,
        "sample": dirs.iter().take(6).collect::<Vec<_>>(),
    })))
}

#[derive(Deserialize)]
struct EditRequest {
    name: String,
    location: String,
    kind: SourceKind,
}

async fn edit_source(
    State(st): State<AppState>,
    UrlPath(old): UrlPath<String>,
    Json(req): Json<EditRequest>,
) -> ApiResult {
    if st.jobs.involves(&old) {
        return Err(ApiError::conflict(anyhow!("stop this source's sync before editing it")));
    }
    let (url, path) = split_location(&req.location).map_err(ApiError::bad_request)?;
    let new_name = req.name.trim().to_string();
    let before = st
        .config()
        .root(&old)
        .cloned()
        .ok_or_else(|| ApiError::bad_request(anyhow!("no source named {old:?}")))?;
    st.update_config(|cfg| {
        if new_name != old && cfg.roots.iter().any(|r| r.name.eq_ignore_ascii_case(&new_name)) {
            bail!("another source is already called {new_name:?}");
        }
        let r = cfg.roots.iter_mut().find(|r| r.name == old).ok_or_else(|| anyhow!("no source named {old:?}"))?;
        r.name = new_name.clone();
        r.url = url.clone();
        r.path = path.clone();
        r.kind = req.kind;
        Ok(())
    })?;
    let after = st.config().root(&new_name).cloned().expect("just saved");
    let moved = before.location() != after.location();
    let regroup = before.kind != after.kind;
    let state = st.clone();
    let old_name = old.clone();
    tokio::task::spawn_blocking(move || -> Result<()> {
        let mut index = state.open_index()?;
        if old_name != after.name {
            index.rename_root(&old_name, &after.name)?;
        }
        if moved {
            // A different folder: drop what came from the old one.
            index.remove_root(&after.name)?;
        } else if regroup {
            library::rebuild_titles(&mut index, &after)?;
        }
        Ok(())
    })
    .await
    .map_err(|e| ApiError::internal(anyhow!(e)))?
    .map_err(ApiError::internal)?;
    if moved {
        st.jobs.enqueue(Job::Sync {
            root: new_name,
            full: true,
        });
    }
    ok()
}

async fn delete_source(State(st): State<AppState>, UrlPath(name): UrlPath<String>) -> ApiResult {
    st.jobs.stop_root(&name);
    if st.jobs.status().current_root.as_deref() == Some(name.as_str()) {
        return Err(ApiError::conflict(anyhow!(
            "stopping this source's sync; try again in a moment"
        )));
    }
    st.update_config(|cfg| {
        let before = cfg.roots.len();
        cfg.roots.retain(|r| r.name != name);
        if cfg.roots.len() == before {
            bail!("no source named {name:?}");
        }
        Ok(())
    })?;
    let state = st.clone();
    let removed = tokio::task::spawn_blocking(move || state.open_index()?.remove_root(&name))
        .await
        .map_err(|e| ApiError::internal(anyhow!(e)))?
        .map_err(ApiError::internal)?;
    Ok(Json(json!({ "ok": true, "removed_files": removed })))
}

async fn sync_source(State(st): State<AppState>, UrlPath(name): UrlPath<String>) -> ApiResult {
    if st.config().root(&name).is_none() {
        return Err(ApiError::bad_request(anyhow!("no source named {name:?}")));
    }
    st.jobs.enqueue(Job::Sync { root: name, full: false });
    ok()
}

async fn stop_source(State(st): State<AppState>, UrlPath(name): UrlPath<String>) -> ApiResult {
    st.jobs.stop_root(&name);
    ok()
}

#[derive(Deserialize, Default)]
struct SyncAllRequest {
    #[serde(default)]
    full: bool,
}

async fn sync_all(State(st): State<AppState>, body: Option<Json<SyncAllRequest>>) -> ApiResult {
    let full = body.map(|Json(b)| b.full).unwrap_or(false);
    let roots = st.config().roots.clone();
    if roots.is_empty() {
        return Err(ApiError::bad_request(anyhow!("add a source first")));
    }
    jobs::sync_all(&st.jobs, &roots, full);
    ok()
}

async fn stop_jobs(State(st): State<AppState>) -> ApiResult {
    st.jobs.stop_all();
    ok()
}

fn same_location(a: &str, b: &str) -> bool {
    let norm = |s: &str| s.trim().trim_end_matches(['/', '\\']).to_lowercase();
    norm(a) == norm(b)
}

/// "D:\Videos\Movies" -> "Movies"; "http://nas/media/" -> "media".
fn default_name(location: &str) -> String {
    if let Ok(url) = Url::parse(location.trim()) {
        if matches!(url.scheme(), "http" | "https") {
            let last = url
                .path_segments()
                .and_then(|mut s| s.rfind(|seg| !seg.is_empty()))
                .map(percent_decode);
            return last.unwrap_or_else(|| url.host_str().unwrap_or("server").to_string());
        }
    }
    Path::new(location.trim())
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| location.trim().trim_end_matches(['\\', '/']).to_string())
}

fn unique_name(cfg: &Config, base: &str) -> String {
    let taken = |n: &str| cfg.roots.iter().any(|r| r.name.eq_ignore_ascii_case(n));
    if !taken(base) {
        return base.to_string();
    }
    (2..)
        .map(|i| format!("{base} {i}"))
        .find(|n| !taken(n))
        .expect("unbounded range")
}

// ------------------------------------------------------------- settings ---

async fn get_settings(State(st): State<AppState>) -> ApiResult {
    let cfg = st.config().clone();
    Ok(Json(json!({
        "player": cfg.player,
        "tmdb": cfg.tmdb,
        "assistant": cfg.assistant,
        "library": {
            "skip_dirs": cfg.skip_dirs,
            "media_extensions": cfg.media_extensions,
            "concurrency": cfg.concurrency,
            "delay_ms": cfg.delay_ms,
            "timeout_secs": cfg.timeout_secs,
            "max_depth": cfg.max_depth,
        },
        "paths": {
            "config": st.config_path.display().to_string(),
            "data": cfg.base_dir.display().to_string(),
            "db": cfg.db_file().display().to_string(),
        },
    })))
}

async fn save_player(State(st): State<AppState>, Json(p): Json<crate::config::Player>) -> ApiResult {
    st.update_config(|cfg| {
        if p.command.trim().is_empty() {
            bail!("choose a video player program");
        }
        cfg.player = crate::config::Player {
            command: p.command.trim().to_string(),
            args: p.args.into_iter().filter(|a| !a.trim().is_empty()).collect(),
            prefer_builtin: p.prefer_builtin,
        };
        Ok(())
    })?;
    ok()
}

async fn save_tmdb(State(st): State<AppState>, Json(t): Json<crate::config::Tmdb>) -> ApiResult {
    let had_key = !st.config().tmdb.api_key.is_empty();
    st.update_config(|cfg| {
        cfg.tmdb = t;
        Ok(())
    })?;
    let cfg = st.config().tmdb.clone();
    if !had_key && !cfg.api_key.is_empty() && cfg.auto_lookup {
        st.jobs.enqueue(Job::Lookup { retry_misses: false });
    }
    ok()
}

async fn save_assistant(State(st): State<AppState>, Json(a): Json<crate::config::Assistant>) -> ApiResult {
    st.update_config(|cfg| {
        cfg.assistant = a;
        Ok(())
    })?;
    ok()
}

#[derive(Deserialize)]
struct LibrarySettings {
    skip_dirs: Vec<String>,
    media_extensions: Vec<String>,
    concurrency: usize,
    delay_ms: u64,
    timeout_secs: u64,
    max_depth: usize,
}

async fn save_library(State(st): State<AppState>, Json(s): Json<LibrarySettings>) -> ApiResult {
    st.update_config(|cfg| {
        if s.media_extensions.iter().all(|e| e.trim().is_empty()) {
            bail!("list at least one video file type");
        }
        cfg.skip_dirs = s.skip_dirs;
        cfg.media_extensions = s.media_extensions;
        cfg.concurrency = s.concurrency;
        cfg.delay_ms = s.delay_ms;
        cfg.timeout_secs = s.timeout_secs;
        cfg.max_depth = s.max_depth;
        Ok(())
    })?;
    ok()
}

#[derive(Deserialize)]
struct KeyTest {
    api_key: String,
}

async fn tmdb_test(Json(k): Json<KeyTest>) -> ApiResult {
    let cfg = crate::config::Tmdb {
        api_key: k.api_key.trim().to_string(),
        ..Default::default()
    };
    let client = Tmdb::new(&cfg).map_err(ApiError::bad_request)?;
    client.check().await.map_err(ApiError::bad_request)?;
    ok()
}

#[derive(Deserialize, Default)]
struct LookupRequest {
    #[serde(default)]
    retry: bool,
}

async fn tmdb_lookup(State(st): State<AppState>, body: Option<Json<LookupRequest>>) -> ApiResult {
    if st.config().tmdb.api_key.is_empty() {
        return Err(ApiError::bad_request(anyhow!("add a TMDB API key first")));
    }
    let retry = body.map(|Json(b)| b.retry).unwrap_or(false);
    st.jobs.enqueue(Job::Lookup { retry_misses: retry });
    ok()
}

// ------------------------------------------------------------ assistant ---

async fn chat_history(State(st): State<AppState>) -> ApiResult {
    let messages = st.db(|c| assistant::history(c, 100))?;
    Ok(Json(json!({ "messages": messages })))
}

#[derive(Deserialize)]
struct ChatRequest {
    message: String,
}

async fn chat(State(st): State<AppState>, Json(req): Json<ChatRequest>) -> ApiResult {
    let message = req.message.trim();
    if message.is_empty() {
        return Err(ApiError::bad_request(anyhow!("type a message")));
    }
    let cfg = st.config().assistant.clone();
    let state = st.clone();
    let with_db = move |f: &mut dyn FnMut(&Connection) -> Result<()>| -> Result<()> {
        f(state.index().conn())
    };
    let answer = assistant::ask(&cfg, message, with_db)
        .await
        .map_err(ApiError::bad_request)?;
    Ok(Json(serde_json::to_value(answer).map_err(|e| ApiError::internal(e.into()))?))
}

async fn chat_clear(State(st): State<AppState>) -> ApiResult {
    st.db(assistant::clear)?;
    ok()
}

#[derive(Deserialize)]
struct ModelsQuery {
    key: Option<String>,
}

async fn models(State(st): State<AppState>, Query(q): Query<ModelsQuery>) -> ApiResult {
    let key = q
        .key
        .filter(|k| !k.trim().is_empty())
        .unwrap_or_else(|| st.config().assistant.api_key.clone());
    let models = assistant::list_models(key.trim())
        .await
        .map_err(ApiError::bad_request)?;
    Ok(Json(json!({ "models": models })))
}

// -------------------------------------------------------------- storage ---

async fn storage(State(st): State<AppState>) -> ApiResult {
    let db = st.config().db_file();
    let size = |p: &Path| std::fs::metadata(p).map(|m| m.len()).unwrap_or(0);
    let db_bytes = size(&db) + size(&PathBuf::from(format!("{}-wal", db.display())));
    let stats = st.db(|c| {
        let one = |sql: &str| -> Result<i64> { Ok(c.query_row(sql, [], |r| r.get(0))?) };
        Ok(json!({
            "files": one("SELECT COUNT(*) FROM files")?,
            "titles": one("SELECT COUNT(*) FROM titles")?,
            "movies": one("SELECT COUNT(*) FROM titles WHERE kind = 'movie'")?,
            "shows": one("SELECT COUNT(*) FROM titles WHERE kind = 'show'")?,
            "posters": one("SELECT COUNT(*) FROM titles WHERE poster IS NOT NULL OR tmdb_poster IS NOT NULL")?,
            "matched": one("SELECT COUNT(*) FROM titles WHERE tmdb_status = 'ok'")?,
            "unmatched": one("SELECT COUNT(*) FROM titles WHERE tmdb_status = 'none'")?,
            "pending": one("SELECT COUNT(*) FROM titles WHERE tmdb_status IS NULL")?,
            "watchlist": one("SELECT COUNT(*) FROM watchlist")?,
            "plays": one("SELECT COUNT(*) FROM plays")?,
            "chat": one("SELECT COUNT(*) FROM chat")?,
            "bytes": one("SELECT COALESCE(SUM(size), 0) FROM files")?,
        }))
    })?;
    let cfg = st.config().clone();
    Ok(Json(json!({
        "stats": stats,
        "db_bytes": db_bytes,
        "db_path": db.display().to_string(),
        "data_dir": cfg.base_dir.display().to_string(),
        "config_path": st.config_path.display().to_string(),
    })))
}

async fn storage_action(State(st): State<AppState>, UrlPath(action): UrlPath<String>) -> ApiResult {
    match action.as_str() {
        "clear-history" => st.db(|c| {
            c.execute_batch("DELETE FROM plays; DELETE FROM watched;")?;
            Ok(())
        })?,
        "clear-info" => {
            st.db(|c| {
                c.execute_batch(
                    "UPDATE titles SET tmdb_status = NULL, tmdb_id = NULL, tmdb_title = NULL,
                         overview = NULL, tagline = NULL, rating = NULL, genres = NULL,
                         country = NULL, credits = NULL, tmdb_poster = NULL, backdrop = NULL,
                         runtime = NULL, language = NULL, network = NULL, tmdb_ongoing = NULL;
                     UPDATE titles_fts SET alt = '';
                     DELETE FROM episode_meta; DELETE FROM season_fetched;",
                )?;
                Ok(())
            })?;
            let cfg = st.config().tmdb.clone();
            if !cfg.api_key.is_empty() {
                st.jobs.enqueue(Job::Lookup { retry_misses: false });
            }
        }
        "rebuild" => {
            if st.jobs.status().running {
                return Err(ApiError::conflict(anyhow!("wait for the current sync to finish")));
            }
            let state = st.clone();
            tokio::task::spawn_blocking(move || -> Result<()> {
                let roots = state.config().roots.clone();
                let mut index = state.open_index()?;
                for r in &roots {
                    library::rebuild_titles(&mut index, r)?;
                }
                Ok(())
            })
            .await
            .map_err(|e| ApiError::internal(anyhow!(e)))?
            .map_err(ApiError::internal)?;
        }
        "reset" => {
            st.jobs.stop_all();
            if st.jobs.status().running {
                return Err(ApiError::conflict(anyhow!("stopping the current job; try again in a moment")));
            }
            st.db(|c| {
                c.execute_batch(
                    "DELETE FROM files; DELETE FROM images; DELETE FROM titles; DELETE FROM titles_fts;
                     DELETE FROM visited; DELETE FROM pending; DELETE FROM sources_state;
                     DELETE FROM episode_meta; DELETE FROM season_fetched; DELETE FROM watchlist;
                     DELETE FROM plays; DELETE FROM watched; DELETE FROM chat;",
                )?;
                Ok(())
            })?;
        }
        "open" => {
            let dir = st.config().base_dir.clone();
            open_external(&dir.display().to_string()).map_err(ApiError::internal)?;
        }
        _ => return Err(ApiError::bad_request(anyhow!("unknown action"))),
    }
    ok()
}

// ---------------------------------------------------------------- media ---

/// Streams an indexed local file to the `<video>` element. `ServeFile`
/// handles Range requests, which is what makes seeking work.
async fn media(State(st): State<AppState>, UrlPath(id): UrlPath<i64>, req: Request) -> Result<Response, ApiError> {
    let url = st.db(|c| Ok(library::file_info(c, id)?.map(|f| f.0)))?;
    serve_local(url, req).await
}

/// A poster image from a local source folder.
async fn poster(State(st): State<AppState>, UrlPath(id): UrlPath<i64>, req: Request) -> Result<Response, ApiError> {
    let url = st.db(|c| library::local_poster(c, id))?;
    serve_local(url, req).await
}

async fn serve_local(url: Option<String>, req: Request) -> Result<Response, ApiError> {
    let path = url
        .and_then(|u| Url::parse(&u).ok())
        .filter(|u| u.scheme() == "file")
        .and_then(|u| u.to_file_path().ok())
        .ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, anyhow!("no such local file")))?;
    match ServeFile::new(path).oneshot(req).await {
        Ok(res) => Ok(res.map(Body::new)),
        Err(never) => match never {},
    }
}

/// Can a browser `<video>` element likely play this file?
///
/// Keyed on the container only, which is a hint and nothing more: the
/// extension can't say which codec is inside. Chromium (and so the desktop
/// app's WebView2) has no official Matroska support, and HEVC/x265 is
/// unlicensed in most builds whatever the container, so a typical `.mkv`
/// x265 release won't play. An `.mp4` can still fail (HEVC video, or AC3/DTS
/// audio that plays silent); the UI's `onerror` fallback covers that.
pub fn playability(name: &str) -> &'static str {
    let ext = name
        .rsplit_once('.')
        .map(|(_, e)| e.to_ascii_lowercase())
        .unwrap_or_default();
    match ext.as_str() {
        "mp4" | "m4v" | "webm" => "likely",
        "mov" | "ogv" => "maybe",
        _ => "no",
    }
}

// --------------------------------------------------------------- errors ---

struct ApiError {
    status: StatusCode,
    message: String,
}

impl ApiError {
    fn new(status: StatusCode, err: anyhow::Error) -> Self {
        ApiError {
            status,
            message: format!("{err:#}"),
        }
    }
    fn bad_request(err: anyhow::Error) -> Self {
        Self::new(StatusCode::BAD_REQUEST, err)
    }
    fn conflict(err: anyhow::Error) -> Self {
        Self::new(StatusCode::CONFLICT, err)
    }
    fn internal(err: anyhow::Error) -> Self {
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, err)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.status, Json(json!({ "error": self.message }))).into_response()
    }
}

const INDEX_HTML: &str = include_str!("ui/index.html");
const APP_CSS: &str = include_str!("ui/app.css");
const APP_JS: &str = include_str!("ui/app.js");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn playability_by_container() {
        assert_eq!(playability("Arrival (2016) 1080p WEB-DL x264.mp4"), "likely");
        assert_eq!(playability("clip.WEBM"), "likely");
        assert_eq!(playability("clip.mov"), "maybe");
        assert_eq!(playability("Dune.Part.Two.2024.2160p.BluRay.x265.mkv"), "no");
    }

    #[test]
    fn source_names() {
        assert_eq!(default_name(r"D:\Videos\Movies"), "Movies");
        assert_eq!(default_name("http://nas.local/my%20media/"), "my media");
        assert_eq!(default_name("http://nas.local/"), "nas.local");
        let mut cfg = Config::default();
        cfg.roots.push(Root {
            name: "Movies".into(),
            url: None,
            path: Some("x".into()),
            kind: SourceKind::Movies,
        });
        assert_eq!(unique_name(&cfg, "movies"), "movies 2");
        assert!(same_location(r"D:\Videos\", r"d:\videos"));
    }

    #[test]
    fn tmdb_ids() {
        assert_eq!(tmdb_id_from_text("1396"), Some(1396));
        assert_eq!(tmdb_id_from_text("https://www.themoviedb.org/tv/1396-breaking-bad"), Some(1396));
        assert_eq!(tmdb_id_from_text("https://www.themoviedb.org/movie/329865-arrival?lang=en"), Some(329865));
        assert_eq!(tmdb_id_from_text("Arrival"), None);
    }
}
