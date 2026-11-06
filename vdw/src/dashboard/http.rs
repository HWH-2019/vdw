use super::{handoff, preferences, remotes, service};
use crate::{
    lifecycle::{io::*, Engine, Plan},
    store::manager,
};
use anyhow::{bail, Context, Result};
use axum::{
    body::Body,
    extract::{DefaultBodyLimit, Path, Request, State},
    http::{header, HeaderValue, StatusCode},
    middleware::{self, Next},
    response::{
        sse::{Event, KeepAlive},
        IntoResponse, Response, Sse,
    },
    routing::{get, post, put},
    Json, Router,
};
use rust_embed::RustEmbed;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, BTreeSet},
    convert::Infallible,
    fs,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::{Mutex, Semaphore},
};
use tokio_stream::{wrappers::IntervalStream, StreamExt};

#[derive(RustEmbed)]
#[folder = "dashboard-assets/"]
struct Assets;

#[derive(Clone)]
struct App {
    token: String,
    origin: String,
    build: String,
    cache: Arc<Mutex<Option<(Instant, Value)>>>,
    scan: Arc<Semaphore>,
    jobs: Arc<Mutex<BTreeMap<String, Value>>>,
    remote: Arc<Mutex<BTreeMap<String, (Instant, Value)>>>,
    network: Arc<Semaphore>,
    issued: Arc<Mutex<BTreeSet<String>>>,
}
type ApiResult = std::result::Result<Json<Value>, ApiError>;
struct ApiError(anyhow::Error);
impl<E: Into<anyhow::Error>> From<E> for ApiError {
    fn from(error: E) -> Self {
        Self(error.into())
    }
}
impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let message = format!("{:#}", self.0);
        let status = if message.contains("PLAN_STALE") || message.contains("operation is running") {
            StatusCode::CONFLICT
        } else if message.contains("not found") {
            StatusCode::NOT_FOUND
        } else {
            StatusCode::BAD_REQUEST
        };
        (status, Json(json!({"error":message}))).into_response()
    }
}
async fn blocking<T: Send + 'static>(f: impl FnOnce() -> Result<T> + Send + 'static) -> Result<T> {
    tokio::task::spawn_blocking(f)
        .await
        .context("Dashboard task interrupted")?
}

#[derive(Serialize, Deserialize)]
struct Instance {
    origin: String,
    token: String,
    version: String,
    build: String,
}
async fn probe(instance: &Instance) -> bool {
    let Some(address) = instance.origin.strip_prefix("http://") else {
        return false;
    };
    if !address.starts_with("127.0.0.1:") {
        return false;
    }
    tokio::time::timeout(Duration::from_secs(1), async {
        let mut stream = TcpStream::connect(address).await?;
        stream
            .write_all(
                format!("GET /api/health HTTP/1.1\r\nHost: {address}\r\nConnection: close\r\n\r\n")
                    .as_bytes(),
            )
            .await?;
        let mut response = Vec::new();
        stream.take(16384).read_to_end(&mut response).await?;
        let text = String::from_utf8_lossy(&response);
        Ok::<bool, std::io::Error>(text.contains(&hash(instance.token.as_bytes())))
    })
    .await
    .ok()
    .and_then(|r| r.ok())
    .unwrap_or(false)
}
fn launch_browser(url: &str) {
    match preferences::system_open(url).spawn() {
        Ok(mut child) => {
            std::thread::spawn(move || {
                let _ = child.wait();
            });
        }
        Err(error) => eprintln!("Could not open a browser ({error}); open the URL above manually."),
    }
}
pub async fn serve(port: u16, no_open: bool) -> Result<()> {
    let index = Assets::get("index.html")
        .context("Embedded dashboard missing; rebuild frontend before releasing VDW")?;
    let build = hash(&index.data);
    let home = manager::get_vdw_home()?;
    let file = home.join("dashboard/instance.json");
    if let Ok(existing) = read::<Instance>(&file) {
        if probe(&existing).await {
            if existing.version != env!("CARGO_PKG_VERSION") || existing.build != build {
                bail!("Another dashboard version is running at {}. Stop it with Ctrl+C before starting this version.",existing.origin);
            }
            if port != 0 && !existing.origin.ends_with(&format!(":{port}")) {
                bail!("Dashboard already running at {}", existing.origin);
            }
            let url = format!("{}/#session={}", existing.origin, existing.token);
            println!("VDW dashboard already running: {url}");
            if !no_open {
                launch_browser(&url);
            }
            return Ok(());
        }
    }
    let _singleton = Guard::job(&home, "dashboard-server")?;
    let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, port)).await?;
    let origin = format!("http://{}", listener.local_addr()?);
    let mut random = [0u8; 32];
    getrandom::fill(&mut random)
        .map_err(|e| anyhow::anyhow!("Cannot generate dashboard session: {e}"))?;
    let token = random
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>();
    save(
        &file,
        &Instance {
            origin: origin.clone(),
            token: token.clone(),
            version: env!("CARGO_PKG_VERSION").into(),
            build: build.clone(),
        },
    )?;
    let app = App {
        token: token.clone(),
        origin: origin.clone(),
        build,
        cache: Arc::new(Mutex::new(None)),
        scan: Arc::new(Semaphore::new(1)),
        jobs: Arc::new(Mutex::new(BTreeMap::new())),
        remote: Arc::new(Mutex::new(BTreeMap::new())),
        network: Arc::new(Semaphore::new(2)),
        issued: Arc::new(Mutex::new(BTreeSet::new())),
    };
    let routes = Router::new()
        .route("/api/health", get(health))
        .route("/api/workspaces", get(workspaces))
        .route("/api/refresh", post(refresh))
        .route("/api/config", get(configuration))
        .route("/api/config/preview", post(config_preview))
        .route("/api/preferences", put(save_preferences))
        .route("/api/workspaces/{id}/open", post(open_workspace))
        .route("/api/workspaces/{id}/remote", post(remote))
        .route("/api/plans/archive", post(archive))
        .route("/api/plans/restore", post(restore))
        .route("/api/workspaces/{id}/trash/preview", post(trash_preview))
        .route("/api/plans/{id}", get(read_plan))
        .route("/api/plans/{id}/execute", post(execute))
        .route("/api/operations", get(operations))
        .route("/api/operations/{id}/undo/preview", post(undo_preview))
        .route("/api/jobs/{id}", get(job))
        .route("/api/events", get(events))
        .fallback(static_file)
        .layer(DefaultBodyLimit::max(1024 * 1024))
        .layer(middleware::from_fn_with_state(app.clone(), security))
        .with_state(app);
    let url = format!("{origin}/#session={token}");
    println!("VDW dashboard v{}\n{url}\nLocal access only. Press Ctrl+C to stop. Active operations finish before shutdown.",env!("CARGO_PKG_VERSION"));
    if !no_open {
        launch_browser(&url);
    }
    let result = axum::serve(listener, routes)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await;
    // Only remove our descriptor; no workspace or lifecycle records are removed.
    if read::<Instance>(&file)
        .map(|i| i.token == token)
        .unwrap_or(false)
    {
        let _ = fs::remove_file(file);
    }
    result?;
    Ok(())
}

async fn security(State(app): State<App>, request: Request, next: Next) -> Response {
    let expected = app.origin.trim_start_matches("http://");
    let headers = request.headers();
    let api_path = request.uri().path().starts_with("/api/");
    let host = headers.get(header::HOST).and_then(|v| v.to_str().ok());
    let origin = headers.get(header::ORIGIN).and_then(|v| v.to_str().ok());
    // Top-level links from another app/site must be allowed to load the public
    // shell. Only API calls need the stricter same-origin/session checks.
    if host != Some(expected)
        || api_path
            && (origin.is_some_and(|v| v != app.origin)
                || headers
                    .get("sec-fetch-site")
                    .is_some_and(|v| v == "cross-site"))
    {
        return (StatusCode::FORBIDDEN, "Local dashboard origin required").into_response();
    }
    let path = request.uri().path();
    if path.starts_with("/api/") && path != "/api/health" {
        let token = headers
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "));
        if token != Some(app.token.as_str()) {
            return (
                StatusCode::UNAUTHORIZED,
                Json(json!({"error":"请通过 vdw dashboard 打印的完整地址打开看板"})),
            )
                .into_response();
        }
    }
    let mut response = next.run(request).await;
    let headers = response.headers_mut();
    headers.insert(
        "x-content-type-options",
        HeaderValue::from_static("nosniff"),
    );
    headers.insert("referrer-policy", HeaderValue::from_static("no-referrer"));
    headers.insert("content-security-policy",HeaderValue::from_static("default-src 'self'; script-src 'self'; style-src 'self'; img-src 'self' data:; connect-src 'self'; frame-ancestors 'none'; base-uri 'none'; form-action 'self'"));
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}
async fn health(State(app): State<App>) -> Json<Value> {
    Json(
        json!({"version":env!("CARGO_PKG_VERSION"),"build":app.build,"instance":hash(app.token.as_bytes())}),
    )
}
async fn static_file(request: Request) -> Response {
    let path = request.uri().path();
    if request.method() != axum::http::Method::GET || path.starts_with("/api/") {
        return StatusCode::NOT_FOUND.into_response();
    }
    let file = if path == "/" {
        "index.html"
    } else {
        path.trim_start_matches('/')
    };
    if file == "manifest.json" {
        return StatusCode::NOT_FOUND.into_response();
    }
    match Assets::get(file) {
        Some(asset) => Response::builder()
            .header(
                header::CONTENT_TYPE,
                mime_guess::from_path(file).first_or_octet_stream().as_ref(),
            )
            .body(Body::from(asset.data.into_owned()))
            .unwrap(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}
async fn workspaces(State(app): State<App>) -> ApiResult {
    let _permit = app.scan.acquire().await?;
    if let Some((at, value)) = &*app.cache.lock().await {
        if at.elapsed() < Duration::from_secs(10) {
            return Ok(Json(value.clone()));
        }
    }
    let value = blocking(service::snapshot).await?;
    *app.cache.lock().await = Some((Instant::now(), value.clone()));
    Ok(Json(value))
}
async fn refresh(State(app): State<App>) -> ApiResult {
    clear_snapshot(&app).await;
    workspaces(State(app)).await
}
async fn clear_snapshot(app: &App) {
    let _scan = app.scan.acquire().await.unwrap();
    *app.cache.lock().await = None;
}
async fn configuration() -> ApiResult {
    Ok(Json(blocking(service::configuration).await?))
}
#[derive(Deserialize)]
struct PrefRequest {
    version: String,
    preferences: preferences::Preferences,
}
async fn save_preferences(Json(req): Json<PrefRequest>) -> ApiResult {
    let prefs = blocking(move || preferences::store(req.preferences, &req.version)).await?;
    Ok(Json(
        json!({"preferences":prefs,"version":preferences::version(&prefs)?}),
    ))
}
async fn issued(app: &App, plan: Plan) -> ApiResult {
    app.issued.lock().await.insert(plan.plan_id.clone());
    Ok(Json(serde_json::to_value(plan)?))
}
async fn config_preview(
    State(app): State<App>,
    Json(req): Json<service::ConfigRequest>,
) -> ApiResult {
    issued(&app, blocking(move || service::preview_config(req)).await?).await
}
#[derive(Deserialize)]
struct ArchiveRequest {
    workspace_ids: Vec<String>,
}
async fn archive(State(app): State<App>, Json(req): Json<ArchiveRequest>) -> ApiResult {
    if req.workspace_ids.is_empty() || req.workspace_ids.len() > 100 {
        return Err(anyhow::anyhow!("Select 1–100 workspaces").into());
    }
    let plan = blocking(move || {
        for id in &req.workspace_ids {
            service::exact_workspace(id)?;
        }
        Engine::open()?.preview_archive(&req.workspace_ids, false)
    })
    .await?;
    issued(&app, plan).await
}
async fn trash_preview(State(app): State<App>, Path(id): Path<String>) -> ApiResult {
    issued(
        &app,
        blocking(move || Engine::open()?.preview_trash(&id)).await?,
    )
    .await
}
#[derive(Deserialize)]
struct RestoreRequest {
    archive_id: String,
    target: Option<PathBuf>,
    branch: Option<String>,
}
use std::path::PathBuf;
async fn restore(State(app): State<App>, Json(req): Json<RestoreRequest>) -> ApiResult {
    issued(
        &app,
        blocking(move || Engine::open()?.preview_restore(&req.archive_id, req.target, req.branch))
            .await?,
    )
    .await
}
async fn read_plan(State(app): State<App>, Path(id): Path<String>) -> ApiResult {
    let plan = blocking(move || service::plan(&id)).await?;
    issued(&app, plan).await
}
#[derive(Deserialize)]
struct Confirm {
    confirm: bool,
}
async fn execute(
    State(app): State<App>,
    Path(id): Path<String>,
    Json(req): Json<Confirm>,
) -> std::result::Result<(StatusCode, Json<Value>), ApiError> {
    if !req.confirm || !app.issued.lock().await.contains(&id) {
        return Err(anyhow::anyhow!(
            "Preview this plan in this dashboard session before confirming"
        )
        .into());
    }
    let mut jobs = app.jobs.lock().await;
    if let Some(existing) = jobs.get(&id) {
        return Ok((StatusCode::ACCEPTED, Json(existing.clone())));
    }
    let queued = json!({"plan_id":id,"state":"queued"});
    jobs.insert(id.clone(), queued.clone());
    drop(jobs);
    let task_app = app.clone();
    tokio::spawn(async move {
        task_app
            .jobs
            .lock()
            .await
            .insert(id.clone(), json!({"plan_id":id,"state":"running"}));
        let key = id.clone();
        let result = blocking(move || crate::services::operations::execute_plan(&key)).await;
        let value = match result {
            Ok(op) => {
                json!({"plan_id":id,"state":op.state,"operation_id":op.operation_id,"operation":op})
            }
            Err(error) => json!({"plan_id":id,"state":"failed","error":format!("{error:#}")}),
        };
        // No terminal job is visible until older scans have finished and their cache is invalidated.
        clear_snapshot(&task_app).await;
        task_app.jobs.lock().await.insert(id, value);
    });
    Ok((StatusCode::ACCEPTED, Json(queued)))
}
async fn job(State(app): State<App>, Path(id): Path<String>) -> ApiResult {
    if let Some(value) = app.jobs.lock().await.get(&id).cloned() {
        return Ok(Json(value));
    }
    let plan = blocking(move || service::plan(&id)).await?;
    if let Some(op) = plan.operation_id {
        return Ok(Json(
            blocking(move || {
                service::operation(&op).map(|o| json!({"state":o.state,"operation":o}))
            })
            .await?,
        ));
    }
    Ok(Json(json!({"state":"pending","plan_id":plan.plan_id})))
}
async fn operations() -> ApiResult {
    Ok(Json(blocking(service::operations).await?))
}
async fn undo_preview(State(app): State<App>, Path(id): Path<String>) -> ApiResult {
    issued(
        &app,
        blocking(move || Engine::open()?.preview_undo(&id)).await?,
    )
    .await
}
#[derive(Deserialize)]
struct OpenRequest {
    editor: Option<String>,
}
async fn open_workspace(
    State(app): State<App>,
    Path(id): Path<String>,
    Json(req): Json<OpenRequest>,
) -> ApiResult {
    let data = blocking(service::snapshot).await?;
    let row = data["workspaces"]
        .as_array()
        .and_then(|rows| {
            rows.iter()
                .find(|row| row["workspace"]["workspace_id"] == id)
        })
        .cloned()
        .context("Workspace not found")?;
    let (root, selected, launch_error) = blocking(move || {
        let mut e = Engine::open()?;
        let ws = service::exact_workspace(&id)?;
        if ws.lifecycle_state != "active" || ws.current_operation_id.is_some() {
            bail!("Workspace must be active and idle; restore or finish its operation first");
        }
        let root = std::fs::canonicalize(&ws.path).context("Workspace directory is missing")?;
        let (selected, launch_error) =
            match preferences::selected_editor(req.editor.as_deref(), &id) {
                Ok(selected) => {
                    let error = preferences::launch(&root, Some(&selected), &id)
                        .err()
                        .map(|error| format!("{error:#}"));
                    (selected, error)
                }
                Err(error) => ("auto".into(), Some(format!("{error:#}"))),
            };
        if launch_error.is_none() {
            e.touch(&id, "dashboard_open")?;
        }
        Ok((root, selected, launch_error))
    })
    .await?;
    *app.cache.lock().await = None;
    // Open immediately, then inspect remote refs before preparing the editor context.
    let remote = check_remotes(&app, &row, false).await?;
    let prompt = handoff::diagnostic_prompt(&row, &remote);
    let editor = selected.clone();
    let diagnostics = match prompt {
        Some(prompt) => blocking(move || Ok(handoff::deliver(&editor, &root, prompt))).await?,
        None => Value::Null,
    };
    Ok(Json(
        json!({"state":if launch_error.is_some() {"launch_failed"} else {"launch_requested"},"launch_error":launch_error,"editor":selected,"remote":remote,"diagnostics":diagnostics}),
    ))
}
async fn remote(State(app): State<App>, Path(id): Path<String>) -> ApiResult {
    let selected = id.clone();
    let data = blocking(move || service::workspace_snapshot(&selected)).await?;
    let row = data["workspaces"]
        .as_array()
        .and_then(|rows| {
            rows.iter()
                .find(|row| row["workspace"]["workspace_id"] == id)
        })
        .context("Workspace not found")?;
    // A manual check deliberately bypasses TTL so a just-deleted branch is visible immediately.
    Ok(Json(check_remotes(&app, row, true).await?))
}
async fn check_remotes(app: &App, row: &Value, force: bool) -> Result<Value> {
    let seconds = blocking(preferences::load).await?.remote_cache_seconds;
    let mut pending = tokio::task::JoinSet::new();
    for repo in row["repositories"]
        .as_array()
        .context("Repository list unavailable")?
    {
        let app = app.clone();
        let repo = repo.clone();
        pending.spawn(async move {
            let source = repo["remote"].as_str().unwrap_or("");
            let path = repo["id"].as_str().unwrap_or("").to_owned();
            if !service::is_remote(source) {
                return Ok::<_, anyhow::Error>((path, remotes::local()));
            }
            let branch = repo["development_branch"]
                .as_str()
                .filter(|name| !name.is_empty());
            let cache_key = format!("{source}\0{}", branch.unwrap_or(""));
            if !force {
                if let Some((at, value)) = app.remote.lock().await.get(&cache_key) {
                    if at.elapsed() < Duration::from_secs(seconds) {
                        return Ok((path, value.clone()));
                    }
                }
            }
            let _permit = app.network.acquire().await?;
            let value = remotes::inspect(source, branch).await;
            app.remote
                .lock()
                .await
                .insert(cache_key, (Instant::now(), value.clone()));
            Ok((path, value))
        });
    }
    let mut results = BTreeMap::new();
    while let Some(result) = pending.join_next().await {
        let (path, value) = result??;
        results.insert(path, value);
    }
    Ok(json!(results))
}

async fn events(
    State(_app): State<App>,
) -> Sse<impl tokio_stream::Stream<Item = std::result::Result<Event, Infallible>>> {
    let stream = IntervalStream::new(tokio::time::interval(Duration::from_secs(5))).map(|_| {
        Ok(Event::default()
            .event("tick")
            .data(chrono::Utc::now().to_rfc3339()))
    });
    Sse::new(stream).keep_alive(KeepAlive::default())
}
