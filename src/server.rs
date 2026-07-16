use crate::open_target::{resolve_open_target, OpenTargetError, TargetKind};
use crate::protocol::{
    DriverState, ElementDomInfo, ElementRef, ExecResult, RectInfo, Session, SnapshotCache, TabInfo,
    WsIncoming,
};
use crate::{config, html};
use anyhow::{anyhow, Result};
use axum::body::Body;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path as AxumPath, Query, State};
use axum::http::{header::ORIGIN, HeaderMap, Request, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};
use tokio::sync::{mpsc, oneshot, Mutex, Notify};
use uuid::Uuid;

const HOST: &str = "127.0.0.1";
const API_PORT: u16 = 18767;
const LOOPBACK_API_ORIGIN: &str = "http://127.0.0.1:18767";
const LOCALHOST_API_ORIGIN: &str = "http://localhost:18767";
const CHROME_EXTENSION_ORIGIN_PREFIX: &str = "chrome-extension://";
// daemon 在无 CLI/API 业务请求后自动退出，避免浏览器扩展长期保持“已连接”浮层。
const IDLE_SHUTDOWN_TTL: Duration = Duration::from_secs(300);
const IDLE_SHUTDOWN_CHECK_INTERVAL: Duration = Duration::from_secs(5);

#[derive(Clone)]
pub struct AppState {
    driver: Arc<Mutex<DriverState>>,
    started_at: SystemTime,
    last_activity: Arc<Mutex<Instant>>,
    shutdown: mpsc::UnboundedSender<()>,
    sessions_ready: Arc<Notify>,
    extension_port: u16,
    api_token: String,
}

#[derive(Debug, Deserialize)]
struct ScanRequest {
    #[serde(default)]
    tabs_only: bool,
    switch_tab_id: Option<String>,
    browser: Option<String>,
    profile: Option<String>,
    #[serde(default)]
    text_only: bool,
}

#[derive(Debug, Deserialize)]
struct ExecRequest {
    #[serde(default)]
    script: String,
    switch_tab_id: Option<String>,
    browser: Option<String>,
    profile: Option<String>,
    #[serde(default)]
    no_monitor: bool,
    wait_js: Option<String>,
    #[serde(default = "default_wait_timeout")]
    wait_timeout: f64,
    #[serde(default = "default_wait_interval")]
    wait_interval: f64,
}

#[derive(Debug, Deserialize)]
struct OpenRequest {
    target: Option<String>,
    url: Option<String>,
    cwd: Option<PathBuf>,
    #[serde(default = "default_active")]
    active: bool,
    switch_tab_id: Option<String>,
    browser: Option<String>,
    profile: Option<String>,
    session: Option<String>,
    group_title: Option<String>,
    #[serde(default)]
    window: bool,
    #[serde(default)]
    allow_focus: bool,
    #[serde(default = "default_open_readiness_timeout")]
    readiness_timeout: f64,
}

#[derive(Debug, Deserialize)]
struct CloseRequest {
    tab_id: String,
    browser: Option<String>,
    profile: Option<String>,
}

#[derive(Debug, Deserialize)]
struct SnapshotRequest {
    switch_tab_id: Option<String>,
    browser: Option<String>,
    profile: Option<String>,
    #[serde(default)]
    offset: usize,
    #[serde(default = "default_snapshot_limit")]
    limit: usize,
    #[serde(default)]
    details: bool,
    #[serde(default = "default_high_level_timeout")]
    timeout: f64,
}

#[derive(Debug, Deserialize)]
struct TargetActionRequest {
    target: String,
    switch_tab_id: Option<String>,
    browser: Option<String>,
    profile: Option<String>,
    #[serde(default)]
    monitor: bool,
    wait_js: Option<String>,
    #[serde(default = "default_wait_timeout")]
    wait_timeout: f64,
    #[serde(default = "default_wait_interval")]
    wait_interval: f64,
    #[serde(default = "default_high_level_timeout")]
    timeout: f64,
}

#[derive(Debug, Deserialize)]
struct FillRequest {
    target: String,
    #[serde(default)]
    value: String,
    switch_tab_id: Option<String>,
    browser: Option<String>,
    profile: Option<String>,
    #[serde(default)]
    append: bool,
    #[serde(default)]
    clear: bool,
    #[serde(default)]
    has_value: bool,
    #[serde(default)]
    monitor: bool,
    wait_js: Option<String>,
    #[serde(default = "default_wait_timeout")]
    wait_timeout: f64,
    #[serde(default = "default_wait_interval")]
    wait_interval: f64,
    #[serde(default = "default_high_level_timeout")]
    timeout: f64,
}

#[derive(Debug, Deserialize)]
struct SendKeysRequest {
    keys: String,
    target: Option<String>,
    switch_tab_id: Option<String>,
    browser: Option<String>,
    profile: Option<String>,
    #[serde(default)]
    monitor: bool,
    wait_js: Option<String>,
    #[serde(default = "default_wait_timeout")]
    wait_timeout: f64,
    #[serde(default = "default_wait_interval")]
    wait_interval: f64,
    #[serde(default = "default_high_level_timeout")]
    timeout: f64,
}

#[derive(Debug, Deserialize)]
struct ScreenshotRequest {
    switch_tab_id: Option<String>,
    browser: Option<String>,
    profile: Option<String>,
    target: Option<String>,
    selector: Option<String>,
    out: Option<PathBuf>,
    #[serde(default = "default_screenshot_format")]
    format: String,
    quality: Option<u8>,
    #[serde(default)]
    full_page: bool,
    #[serde(default = "default_high_level_timeout")]
    timeout: f64,
}

#[derive(Debug, Deserialize)]
struct SavePdfRequest {
    switch_tab_id: Option<String>,
    browser: Option<String>,
    profile: Option<String>,
    out: Option<PathBuf>,
    #[serde(default = "default_pdf_paper")]
    paper: String,
    #[serde(default)]
    landscape: bool,
    #[serde(default = "default_pdf_scale")]
    scale: f64,
    #[serde(default = "default_true")]
    print_background: bool,
    #[serde(default = "default_high_level_timeout")]
    timeout: f64,
}

#[derive(Debug, Deserialize, Default)]
struct TabsQuery {
    browser: Option<String>,
    profile: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
struct LookupTabQuery {
    browser: Option<String>,
    profile: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
struct TabtreeQuery {
    tab: Option<String>,
    browser: Option<String>,
    profile: Option<String>,
    #[serde(default)]
    full: bool,
}

#[derive(Debug, Clone)]
struct SessionTreeItem {
    browser_id: String,
    profile_id: String,
    profile_label: Option<String>,
    tab_id: String,
    session_key: String,
    title: String,
    url: String,
}

#[derive(Debug, Deserialize)]
struct ProfileLabelRequest {
    label: Option<String>,
    switch_tab_id: Option<String>,
    browser: Option<String>,
    profile: Option<String>,
}

#[derive(Debug, Deserialize)]
struct TabRequest {
    switch_tab_id: Option<String>,
    browser: Option<String>,
    profile: Option<String>,
}

#[derive(Debug, Deserialize)]
struct NetworkListRequest {
    switch_tab_id: Option<String>,
    browser: Option<String>,
    profile: Option<String>,
    filter: Option<String>,
    #[serde(default = "default_debug_list_limit")]
    limit: usize,
}

#[derive(Debug, Deserialize)]
struct NetworkDetailRequest {
    switch_tab_id: Option<String>,
    browser: Option<String>,
    profile: Option<String>,
    request_id: String,
}

#[derive(Debug, Deserialize)]
struct ConsoleListRequest {
    switch_tab_id: Option<String>,
    browser: Option<String>,
    profile: Option<String>,
    level: Option<String>,
    #[serde(default = "default_debug_list_limit")]
    limit: usize,
}

fn default_active() -> bool {
    true
}

fn default_wait_timeout() -> f64 {
    3.0
}

fn default_open_readiness_timeout() -> f64 {
    10.0
}

fn default_wait_interval() -> f64 {
    0.1
}

fn default_snapshot_limit() -> usize {
    200
}

fn default_high_level_timeout() -> f64 {
    30.0
}

fn default_screenshot_format() -> String {
    "png".to_string()
}

fn default_pdf_paper() -> String {
    "a4".to_string()
}

fn default_pdf_scale() -> f64 {
    1.0
}

fn default_true() -> bool {
    true
}

fn default_debug_list_limit() -> usize {
    100
}

#[derive(Debug, Clone, Default)]
struct SessionSelector {
    tab_id: Option<String>,
    browser: Option<String>,
    profile: Option<String>,
}

impl SessionSelector {
    fn new(tab_id: Option<String>, browser: Option<String>, profile: Option<String>) -> Self {
        Self {
            tab_id,
            browser,
            profile,
        }
    }
}

pub async fn run_daemon() -> Result<()> {
    let extension_port = config::load_or_create()?.extension_port;
    let addr: SocketAddr = format!("{HOST}:{API_PORT}").parse()?;
    let listener = tokio::net::TcpListener::bind(addr).await?;
    let api_token = Uuid::new_v4().simple().to_string();
    config::save_api_token(&api_token)?;
    let (shutdown_tx, mut shutdown_rx) = mpsc::unbounded_channel::<()>();
    let state = AppState {
        driver: Arc::new(Mutex::new(DriverState::default())),
        started_at: SystemTime::now(),
        last_activity: Arc::new(Mutex::new(Instant::now())),
        shutdown: shutdown_tx,
        sessions_ready: Arc::new(Notify::new()),
        extension_port,
        api_token,
    };

    let ws_state = state.clone();
    tokio::spawn(async move {
        if let Err(err) = run_ws_server(ws_state).await {
            eprintln!("WebSocket 服务异常: {err:?}");
        }
    });

    let idle_state = state.clone();
    tokio::spawn(async move {
        monitor_idle_shutdown(idle_state).await;
    });

    let app = build_api_router(state.clone());

    println!("agent-browser-cli rust server listening on http://{addr}");
    let cleanup_state = state.clone();
    let serve_result = axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            let _ = shutdown_rx.recv().await;
            cleanup_on_shutdown(&cleanup_state).await;
        })
        .await;
    // Keep the last token file after shutdown. The next daemon rotates it after binding the API
    // port, avoiding an exiting daemon racing with and deleting a newer daemon's token.
    serve_result?;
    Ok(())
}

fn build_api_router(state: AppState) -> Router {
    Router::new()
        .route("/", get(root))
        .route("/health", get(health))
        .route("/tabs", get(tabs))
        .route("/tabtree", get(tabtree))
        .route("/lookup/tab/:tab_id", get(lookup_tab))
        .route("/lookup/browser/:browser_id", get(lookup_browser))
        .route("/lookup/profile/:profile", get(lookup_profile))
        .route("/profile-label", post(set_profile_label))
        .route("/scan", post(scan))
        .route("/exec", post(exec))
        .route("/open", post(open_tab))
        .route("/close", post(close))
        .route("/snapshot", post(snapshot))
        .route("/click", post(click))
        .route("/fill", post(fill))
        .route("/send-keys", post(send_keys))
        .route("/mouse-click", post(mouse_click))
        .route("/screenshot", post(screenshot))
        .route("/save-pdf", post(save_pdf))
        .route("/network/start", post(network_start))
        .route("/network/list", post(network_list))
        .route("/network/detail", post(network_detail))
        .route("/network/clear", post(network_clear))
        .route("/network/stop", post(network_stop))
        .route("/console/start", post(console_start))
        .route("/console/list", post(console_list))
        .route("/console/clear", post(console_clear))
        .route("/console/stop", post(console_stop))
        .route("/shutdown", post(shutdown))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            require_api_access,
        ))
        .with_state(state)
}

async fn require_api_access(
    State(state): State<AppState>,
    request: Request<Body>,
    next: Next,
) -> Response {
    if let Err((status, code, message)) = validate_api_access(request.headers(), &state.api_token) {
        return (
            status,
            Json(json!({ "ok": false, "error": message, "error_code": code })),
        )
            .into_response();
    }
    next.run(request).await
}

fn validate_api_access(
    headers: &HeaderMap,
    expected_token: &str,
) -> std::result::Result<(), (StatusCode, &'static str, &'static str)> {
    if let Some(origin) = headers.get(ORIGIN) {
        let allowed = origin
            .to_str()
            .map(|value| matches!(value, LOOPBACK_API_ORIGIN | LOCALHOST_API_ORIGIN))
            .unwrap_or(false);
        if !allowed {
            return Err((
                StatusCode::FORBIDDEN,
                "origin_not_allowed",
                "Browser origins are not allowed to access the local API",
            ));
        }
    }
    let authorized = headers
        .get(config::API_TOKEN_HEADER)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value == expected_token);
    if !authorized {
        return Err((
            StatusCode::UNAUTHORIZED,
            "invalid_api_token",
            "A valid API token is required",
        ));
    }
    Ok(())
}

fn build_ws_router(state: AppState) -> Router {
    Router::new().route("/", get(ws_handler)).with_state(state)
}

async fn run_ws_server(state: AppState) -> Result<()> {
    let extension_port = state.extension_port;
    let app = build_ws_router(state);
    let addr: SocketAddr = format!("{HOST}:{extension_port}").parse()?;
    let listener = tokio::net::TcpListener::bind(addr).await?;
    println!("WebSocket server running on ws://{addr}");
    axum::serve(listener, app).await?;
    Ok(())
}

fn is_allowed_extension_origin(headers: &HeaderMap) -> bool {
    let Some(origin) = headers.get(ORIGIN).and_then(|value| value.to_str().ok()) else {
        return false;
    };
    let Some(extension_id) = origin.strip_prefix(CHROME_EXTENSION_ORIGIN_PREFIX) else {
        return false;
    };
    extension_id.len() == 32 && extension_id.bytes().all(|byte| matches!(byte, b'a'..=b'p'))
}

async fn ws_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> Response {
    if !is_allowed_extension_origin(&headers) {
        return (
            StatusCode::FORBIDDEN,
            "WebSocket connections require a Chrome extension Origin",
        )
            .into_response();
    }
    ws.on_upgrade(move |socket| handle_socket(socket, state))
        .into_response()
}

async fn monitor_idle_shutdown(state: AppState) {
    loop {
        tokio::time::sleep(IDLE_SHUTDOWN_CHECK_INTERVAL).await;
        let idle_for = state.last_activity.lock().await.elapsed();
        if idle_for >= IDLE_SHUTDOWN_TTL {
            eprintln!(
                "agent-browser-cli daemon idle for {}s, shutting down",
                idle_for.as_secs()
            );
            cleanup_on_shutdown(&state).await;
            let _ = state.shutdown.send(());
            break;
        }
    }
}

async fn handle_socket(mut socket: WebSocket, state: AppState) {
    let (tx, mut rx) = mpsc::unbounded_channel::<String>();
    let mut registered_ids: Vec<String> = Vec::new();
    loop {
        tokio::select! {
            Some(outgoing) = rx.recv() => {
                if socket.send(Message::Text(outgoing)).await.is_err() {
                    break;
                }
            }
            msg = socket.recv() => {
                let Some(Ok(msg)) = msg else { break; };
                let Message::Text(text) = msg else { continue; };
                if text.trim() == r#"{"type":"ping"}"# {
                    continue;
                }
                match serde_json::from_str::<WsIncoming>(&text) {
                    Ok(incoming) => handle_ws_message(incoming, &state, tx.clone(), &mut registered_ids).await,
                    Err(err) => eprintln!("WebSocket 消息解析失败: {err}: {text}"),
                }
            }
        }
    }
    let mut driver = state.driver.lock().await;
    for id in registered_ids {
        if let Some(session) = driver.sessions.get_mut(&id) {
            session.disconnected_at = Some(Instant::now());
        }
    }
}

fn should_set_default_session(driver: &DriverState, session_key: &str) -> bool {
    match driver.preferred_default_session_key.as_deref() {
        Some(preferred) => preferred == session_key,
        None => driver.default_session_key.is_none(),
    }
}

async fn handle_ws_message(
    incoming: WsIncoming,
    state: &AppState,
    sender: mpsc::UnboundedSender<String>,
    registered_ids: &mut Vec<String>,
) {
    let mut driver = state.driver.lock().await;
    match incoming {
        WsIncoming::ExtReady {
            browser_id,
            profile_id,
            profile_label,
            extension_version,
            file_scheme_access,
            tabs,
        }
        | WsIncoming::TabsUpdate {
            browser_id,
            profile_id,
            profile_label,
            extension_version,
            file_scheme_access,
            tabs,
        } => {
            let current: std::collections::HashSet<String> = tabs
                .iter()
                .map(|t| {
                    let tab_id = t.id.to_string().trim_matches('"').to_string();
                    crate::protocol::make_session_key(&browser_id, &profile_id, &tab_id)
                })
                .collect();
            for session in driver.sessions.values_mut() {
                if session.info.tab_type == "ext_ws"
                    && session.browser_id == browser_id
                    && session.profile_id == profile_id
                    && !current.contains(&session.session_key)
                {
                    session.disconnected_at = Some(Instant::now());
                }
            }
            for tab in tabs {
                let info = tab.into_tab_info(&browser_id, &profile_id, profile_label.clone());
                let session_key = info.session_key.clone();
                let tab_id = info.tab_id.clone();
                if !registered_ids.contains(&session_key) {
                    registered_ids.push(session_key.clone());
                }
                driver.latest_session_key = Some(session_key.clone());
                if should_set_default_session(&driver, &session_key) {
                    driver.default_session_key = Some(session_key.clone());
                    if driver.preferred_default_session_key.as_deref() == Some(&session_key) {
                        driver.preferred_default_session_key = None;
                    }
                }
                driver.sessions.insert(
                    session_key.clone(),
                    Session {
                        session_key,
                        tab_id,
                        browser_id: browser_id.clone(),
                        profile_id: profile_id.clone(),
                        profile_label: profile_label.clone(),
                        extension_version: extension_version.clone(),
                        file_scheme_access,
                        info,
                        sender: sender.clone(),
                        disconnected_at: None,
                    },
                );
            }
            state.sessions_ready.notify_waiters();
        }
        WsIncoming::Ack { id } => {
            driver.acked.insert(id.clone());
            if let Some(pending) = driver.pending.get_mut(&id) {
                pending.delivered_at = Some(Instant::now());
            }
        }
        WsIncoming::Result {
            id,
            result,
            new_tabs,
        } => {
            if let Some(pending) = driver.pending.remove(&id) {
                let _ = pending.tx.send(Ok(ExecResult {
                    data: Some(result),
                    result: None,
                    closed: None,
                    new_tabs,
                }));
            }
            driver.acked.remove(&id);
            driver.active_exec_sessions.remove(&id);
        }
        WsIncoming::Error {
            id,
            error,
            new_tabs,
        } => {
            if let Some(pending) = driver.pending.remove(&id) {
                let mut value = json!({ "error": error });
                if let Some(tabs) = new_tabs {
                    value["newTabs"] = tabs;
                }
                let _ = pending.tx.send(Err(anyhow!(value.to_string())));
            }
            driver.acked.remove(&id);
            driver.active_exec_sessions.remove(&id);
        }
        WsIncoming::Other => {}
    }
}

async fn root() -> &'static str {
    "agent-browser-cli"
}

async fn health(State(state): State<AppState>) -> Json<Value> {
    let active_tabs_count = active_tabs(&state, false).await.len();
    let extension_connected = has_extension_connection(&state).await;
    let profile_capabilities = extension_profile_capabilities(&state).await;
    let ready = extension_connected && active_tabs_count > 0;
    let uptime = state
        .started_at
        .elapsed()
        .map(|d| d.as_secs_f64())
        .unwrap_or_default();
    let idle_for = state.last_activity.lock().await.elapsed().as_secs_f64();
    let ttl = IDLE_SHUTDOWN_TTL.as_secs_f64();
    let configured_extension_port = config::load_existing()
        .map(|config| config.extension_port)
        .unwrap_or(state.extension_port);
    Json(json!({
        "ok": true,
        "running": true,
        "ready": ready,
        "ports": {
            "api": API_PORT,
            "extension": {
                "configured": configured_extension_port,
                "listening": state.extension_port,
                "matched": configured_extension_port == state.extension_port
            }
        },
        "connection": {
            "extension_connected": extension_connected,
            "active_tabs": active_tabs_count,
            "profiles": profile_capabilities
        },
        "uptime": uptime,
        "idle_for": idle_for,
        "ttl": ttl,
        "ttl_remaining": (ttl - idle_for).max(0.0)
    }))
}

async fn tabs(State(state): State<AppState>, Query(query): Query<TabsQuery>) -> Json<Value> {
    touch(&state).await;
    match active_tabs_filtered(
        &state,
        true,
        query.browser.as_deref(),
        query.profile.as_deref(),
    )
    .await
    {
        Ok(tabs) => Json(
            json!({ "ok": true, "result": { "status": "success", "metadata": { "tabs_count": tabs.len(), "tabs": tabs } } }),
        ),
        Err(err) => Json(json!({ "ok": false, "error": err.to_string() })),
    }
}

async fn tabtree(State(state): State<AppState>, Query(query): Query<TabtreeQuery>) -> Json<Value> {
    touch(&state).await;
    wait_for_sessions(&state, Duration::from_secs(5)).await;
    let items: Vec<SessionTreeItem> = {
        let driver = state.driver.lock().await;
        driver
            .sessions
            .values()
            .filter(|s| s.is_active())
            .filter(|s| {
                query
                    .browser
                    .as_deref()
                    .map(|browser| s.browser_id == browser)
                    .unwrap_or(true)
            })
            .filter(|s| {
                query
                    .profile
                    .as_deref()
                    .map(|profile| {
                        s.profile_id == profile || s.profile_label.as_deref() == Some(profile)
                    })
                    .unwrap_or(true)
            })
            .filter(|s| {
                query
                    .tab
                    .as_deref()
                    .map(|tab| s.tab_id == tab || s.session_key == tab)
                    .unwrap_or(true)
            })
            .map(|s| SessionTreeItem {
                browser_id: s.browser_id.clone(),
                profile_id: s.profile_id.clone(),
                profile_label: s.profile_label.clone(),
                tab_id: s.tab_id.clone(),
                session_key: s.session_key.clone(),
                title: s.info.title.clone(),
                url: s.info.url.clone(),
            })
            .collect()
    };

    let mut browser_map: HashMap<String, HashMap<String, Vec<SessionTreeItem>>> = HashMap::new();
    for item in items {
        browser_map
            .entry(item.browser_id.clone())
            .or_default()
            .entry(item.profile_id.clone())
            .or_default()
            .push(item);
    }

    let mut browsers: Vec<Value> = browser_map
        .into_iter()
        .map(|(browser_id, profiles)| {
            let mut profile_nodes: Vec<Value> = profiles
                .into_iter()
                .map(|(profile_id, mut sessions)| {
                    sessions.sort_by(|a, b| a.tab_id.cmp(&b.tab_id));
                    let profile_label = sessions.iter().find_map(|s| s.profile_label.clone());
                    let tabs: Vec<Value> = sessions
                        .iter()
                        .map(|s| tabtree_tab_json(s, query.full))
                        .collect();
                    json!({
                        "profile_id": profile_id,
                        "profile_label": profile_label,
                        "tabs_count": tabs.len(),
                        "tabs": tabs,
                    })
                })
                .collect();
            profile_nodes.sort_by(|a, b| {
                a.get("profile_label")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .cmp(b.get("profile_label").and_then(Value::as_str).unwrap_or(""))
                    .then_with(|| {
                        a.get("profile_id")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .cmp(b.get("profile_id").and_then(Value::as_str).unwrap_or(""))
                    })
            });
            let tabs_count: usize = profile_nodes
                .iter()
                .map(|p| p.get("tabs_count").and_then(Value::as_u64).unwrap_or(0) as usize)
                .sum();
            json!({
                "browser_id": browser_id,
                "profiles_count": profile_nodes.len(),
                "tabs_count": tabs_count,
                "profiles": profile_nodes,
            })
        })
        .collect();
    browsers.sort_by(|a, b| {
        a.get("browser_id")
            .and_then(Value::as_str)
            .unwrap_or("")
            .cmp(b.get("browser_id").and_then(Value::as_str).unwrap_or(""))
    });
    let profiles_count: usize = browsers
        .iter()
        .map(|b| b.get("profiles_count").and_then(Value::as_u64).unwrap_or(0) as usize)
        .sum();
    let tabs_count: usize = browsers
        .iter()
        .map(|b| b.get("tabs_count").and_then(Value::as_u64).unwrap_or(0) as usize)
        .sum();
    Json(json!({
        "ok": true,
        "result": {
            "status": "success",
            "compact": !query.full,
            "filters": {
                "tab": query.tab,
                "browser": query.browser,
                "profile": query.profile,
            },
            "browsers_count": browsers.len(),
            "profiles_count": profiles_count,
            "tabs_count": tabs_count,
            "browsers": browsers,
        }
    }))
}

fn tabtree_tab_json(item: &SessionTreeItem, full: bool) -> Value {
    if full {
        json!({
            "tab_id": item.tab_id,
            "session_key": item.session_key,
            "title": item.title,
            "url": item.url,
        })
    } else {
        json!({
            "tab_id": item.tab_id,
            "title": item.title,
            "url": truncate_for_tree(&item.url, 120),
        })
    }
}

fn truncate_for_tree(input: &str, max_chars: usize) -> String {
    if input.chars().count() <= max_chars {
        return input.to_string();
    }
    format!("{}...", input.chars().take(max_chars).collect::<String>())
}

async fn lookup_tab(
    State(state): State<AppState>,
    AxumPath(tab_id): AxumPath<String>,
    Query(query): Query<LookupTabQuery>,
) -> Json<Value> {
    touch(&state).await;
    wait_for_sessions(&state, Duration::from_secs(5)).await;
    let driver = state.driver.lock().await;
    let matches: Vec<&Session> = driver
        .sessions
        .values()
        .filter(|s| s.is_active())
        .filter(|s| s.tab_id == tab_id || s.session_key == tab_id)
        .filter(|s| {
            query
                .browser
                .as_deref()
                .map(|browser| s.browser_id == browser)
                .unwrap_or(true)
        })
        .filter(|s| {
            query
                .profile
                .as_deref()
                .map(|profile| {
                    s.profile_id == profile || s.profile_label.as_deref() == Some(profile)
                })
                .unwrap_or(true)
        })
        .collect();
    if matches.is_empty() {
        return Json(json!({ "ok": false, "error": format!("tab {tab_id} 未连接") }));
    }
    if matches.len() > 1 {
        let sessions: Vec<Value> = matches.iter().map(|s| session_lookup_json(s)).collect();
        return Json(json!({
            "ok": false,
            "error": format!("tab {tab_id} 存在歧义，匹配到 {} 个会话，请补 --profile 或 --browser", sessions.len()),
            "matches": sessions,
        }));
    }
    Json(json!({ "ok": true, "result": session_lookup_json(matches[0]) }))
}

async fn lookup_browser(
    State(state): State<AppState>,
    AxumPath(browser_id): AxumPath<String>,
) -> Json<Value> {
    touch(&state).await;
    wait_for_sessions(&state, Duration::from_secs(5)).await;
    let driver = state.driver.lock().await;
    let sessions: Vec<&Session> = driver
        .sessions
        .values()
        .filter(|s| s.is_active() && s.browser_id == browser_id)
        .collect();
    if sessions.is_empty() {
        return Json(json!({ "ok": false, "error": format!("browser {browser_id} 未连接") }));
    }
    let first = sessions[0];
    Json(json!({
        "ok": true,
        "result": {
            "browser_id": browser_id,
            "profile_id": first.profile_id,
            "profile_label": first.profile_label,
            "tabs_count": sessions.len(),
            "tabs": sessions.iter().map(|s| session_lookup_json(s)).collect::<Vec<_>>(),
        }
    }))
}

async fn lookup_profile(
    State(state): State<AppState>,
    AxumPath(profile): AxumPath<String>,
) -> Json<Value> {
    touch(&state).await;
    wait_for_sessions(&state, Duration::from_secs(5)).await;
    let driver = state.driver.lock().await;
    let sessions: Vec<&Session> = driver
        .sessions
        .values()
        .filter(|s| {
            s.is_active()
                && (s.profile_id == profile || s.profile_label.as_deref() == Some(profile.as_str()))
        })
        .collect();
    if sessions.is_empty() {
        return Json(json!({ "ok": false, "error": format!("profile {profile} 未连接") }));
    }
    let profile_ids: HashSet<String> = sessions.iter().map(|s| s.profile_id.clone()).collect();
    if profile_ids.len() > 1 {
        let matches: Vec<Value> = sessions.iter().map(|s| session_lookup_json(s)).collect();
        return Json(json!({
            "ok": false,
            "error": format!("profile {profile:?} 存在歧义，匹配到多个 profile"),
            "matches": matches,
        }));
    }
    let first = sessions[0];
    let mut browser_ids: Vec<String> = sessions.iter().map(|s| s.browser_id.clone()).collect();
    browser_ids.sort();
    browser_ids.dedup();
    Json(json!({
        "ok": true,
        "result": {
            "profile_id": first.profile_id,
            "profile_label": first.profile_label,
            "browser_ids": browser_ids,
            "browser_count": browser_ids.len(),
            "tabs_count": sessions.len(),
            "tabs": sessions.iter().map(|s| session_lookup_json(s)).collect::<Vec<_>>(),
        }
    }))
}

fn session_lookup_json(session: &Session) -> Value {
    json!({
        "browser_id": session.browser_id,
        "profile_id": session.profile_id,
        "profile_label": session.profile_label,
        "tab_id": session.tab_id,
        "session_key": session.session_key,
        "title": session.info.title,
        "url": session.info.url,
    })
}

async fn set_profile_label(
    State(state): State<AppState>,
    Json(req): Json<ProfileLabelRequest>,
) -> Json<Value> {
    touch(&state).await;
    let result = update_profile_label(&state, req).await;
    Json(match result {
        Ok(value) => json!({ "ok": true, "result": value }),
        Err(err) => json!({ "ok": false, "error": err.to_string() }),
    })
}

async fn scan(State(state): State<AppState>, Json(req): Json<ScanRequest>) -> Json<Value> {
    touch(&state).await;
    let result = scan_page(
        &state,
        req.tabs_only,
        SessionSelector::new(req.switch_tab_id, req.browser, req.profile),
        req.text_only,
    )
    .await;
    Json(match result {
        Ok(value) => json!({ "ok": true, "result": value }),
        Err(err) => json!({ "ok": false, "error": err.to_string() }),
    })
}

async fn exec(State(state): State<AppState>, Json(req): Json<ExecRequest>) -> Json<Value> {
    touch(&state).await;
    let script = if req.wait_js.is_some() && !is_extension_json(&req.script) {
        wrap_script_with_wait(
            &req.script,
            req.wait_js.as_deref().unwrap_or_default(),
            req.wait_timeout,
            req.wait_interval,
        )
    } else {
        req.script.clone()
    };
    let result = execute_page_js(
        &state,
        &script,
        SessionSelector::new(req.switch_tab_id, req.browser, req.profile),
        req.no_monitor,
    )
    .await;
    Json(match result {
        Ok(value) => {
            json!({ "ok": true, "result": value, "combined_wait": req.wait_js.is_some() && !is_extension_json(&req.script) })
        }
        Err(err) => json!({ "ok": false, "error": err.to_string() }),
    })
}

async fn open_tab(State(state): State<AppState>, Json(req): Json<OpenRequest>) -> Json<Value> {
    touch(&state).await;
    let target = match resolve_request_target(req.target.as_deref(), req.url.as_deref()) {
        Ok(target) => target,
        Err(err) => return Json(open_target_error_json(err)),
    };
    let resolved = match resolve_open_target(target, req.cwd.as_deref()) {
        Ok(resolved) => resolved,
        Err(err) => return Json(open_target_error_json(err)),
    };
    let mut selector = SessionSelector::new(req.switch_tab_id, req.browser, req.profile);

    if resolved.kind == TargetKind::LocalResource
        && (!req.readiness_timeout.is_finite() || req.readiness_timeout <= 0.0)
    {
        return Json(json!({
            "ok": false,
            "error": "File readiness timeout must be a finite number greater than zero",
            "error_code": "invalid_readiness_timeout",
            "details": { "readiness_timeout": req.readiness_timeout }
        }));
    }

    if resolved.kind == TargetKind::LocalResource {
        let status_payload = json!({ "cmd": "status" }).to_string();
        let status_result =
            match execute_page_js(&state, &status_payload, selector.clone(), true).await {
                Ok(value) => value,
                Err(err) => {
                    return Json(json!({
                        "ok": false,
                        "error": format!("Could not query extension file capability: {err}"),
                        "error_code": "extension_upgrade_required",
                        "details": { "minimum_extension_version": "2.1" }
                    }))
                }
            };
        if let Some(executor_session_key) = status_result.get("session_key").and_then(Value::as_str)
        {
            selector = SessionSelector::new(Some(executor_session_key.to_string()), None, None);
        }
        let status = status_result
            .get("js_return")
            .cloned()
            .unwrap_or(Value::Null);
        let installed_version = status
            .get("extensionVersion")
            .and_then(Value::as_str)
            .unwrap_or("0");
        if !version_at_least(installed_version, "2.1") {
            return Json(json!({
                "ok": false,
                "error": format!("Agent Browser CLI Bridge {installed_version} does not support local files; version 2.1 or newer is required"),
                "error_code": "extension_upgrade_required",
                "details": {
                    "installed_extension_version": installed_version,
                    "minimum_extension_version": "2.1"
                }
            }));
        }
        if status.get("fileSchemeAccess").and_then(Value::as_bool) != Some(true) {
            return Json(json!({
                "ok": false,
                "error": "Agent Browser CLI Bridge is not allowed to access file URLs",
                "error_code": "file_scheme_access_denied",
                "details": {
                    "extension_id": status.get("extensionId").cloned().unwrap_or(Value::Null),
                    "instructions": [
                        "Open chrome://extensions",
                        "Select Agent Browser CLI Bridge",
                        "Enable Allow access to file URLs"
                    ]
                }
            }));
        }
    }

    let group_title = req.group_title.or(req.session);
    let payload = json!({
        "cmd": "openTab",
        "url": resolved.navigation_url.clone(),
        "active": req.active,
        "window": req.window,
        "allowFocus": req.allow_focus,
        "groupTitle": group_title,
    })
    .to_string();
    let value = match execute_page_js(&state, &payload, selector, true).await {
        Ok(value) => value,
        Err(err) => return Json(json!({ "ok": false, "error": err.to_string() })),
    };
    let result = normalize_open_result(&state, value.clone()).await;
    let opened_tab_id = result
        .get("opened_tab_id")
        .and_then(Value::as_str)
        .map(str::to_string);
    let expected_session_key = result
        .get("opened_session_key")
        .and_then(Value::as_str)
        .map(str::to_string);

    if resolved.kind == TargetKind::LocalResource {
        let ready = if let Some(key) = expected_session_key.as_deref() {
            wait_for_session_key(&state, key, req.readiness_timeout).await
        } else {
            false
        };
        if !ready {
            return Json(json!({
                "ok": false,
                "error": format!("File tab was created but did not become controllable within {} seconds", req.readiness_timeout),
                "error_code": "file_tab_readiness_timeout",
                "opened_tab_id": opened_tab_id,
                "opened_session_key": expected_session_key,
                "details": {
                    "timeout_seconds": req.readiness_timeout,
                    "tab_preserved": true,
                    "navigation_url": resolved.navigation_url
                }
            }));
        }
    }

    if req.active {
        let mut driver = state.driver.lock().await;
        if let Some(key) = expected_session_key.clone() {
            if driver.sessions.get(&key).is_some_and(Session::is_active) {
                driver.default_session_key = Some(key);
                driver.preferred_default_session_key = None;
            } else {
                driver.preferred_default_session_key = Some(key);
            }
        }
    }
    Json(json!({ "ok": true, "result": result }))
}

fn resolve_request_target<'a>(
    target: Option<&'a str>,
    url: Option<&'a str>,
) -> Result<&'a str, OpenTargetError> {
    match (target, url) {
        (Some(target), Some(url)) if target != url => Err(OpenTargetError {
            code: "ambiguous_open_target",
            message: "Open request contains conflicting target and url fields".to_string(),
            details: json!({ "target": target, "url": url }),
        }),
        (Some(target), _) => Ok(target),
        (_, Some(url)) => Ok(url),
        (None, None) => Err(OpenTargetError {
            code: "missing_open_target",
            message: "Open request requires target or url".to_string(),
            details: json!({}),
        }),
    }
}

fn open_target_error_json(err: OpenTargetError) -> Value {
    json!({
        "ok": false,
        "error": err.message,
        "error_code": err.code,
        "details": err.details,
    })
}

fn version_at_least(installed: &str, minimum: &str) -> bool {
    let parse = |value: &str| {
        value
            .split('.')
            .map(|part| part.parse::<u64>().unwrap_or(0))
            .collect::<Vec<_>>()
    };
    let mut installed = parse(installed);
    let mut minimum = parse(minimum);
    let len = installed.len().max(minimum.len());
    installed.resize(len, 0);
    minimum.resize(len, 0);
    installed >= minimum
}

async fn wait_for_session_key(state: &AppState, session_key: &str, timeout: f64) -> bool {
    let deadline = Instant::now() + Duration::from_secs_f64(timeout.max(0.1));
    loop {
        {
            let driver = state.driver.lock().await;
            if driver
                .sessions
                .get(session_key)
                .is_some_and(Session::is_active)
            {
                return true;
            }
        }
        let now = Instant::now();
        if now >= deadline {
            return false;
        }
        let remaining = deadline.saturating_duration_since(now);
        tokio::select! {
            _ = state.sessions_ready.notified() => {}
            _ = tokio::time::sleep(remaining.min(Duration::from_millis(100))) => {}
        }
    }
}

async fn normalize_open_result(state: &AppState, value: Value) -> Value {
    let opened = value.get("js_return").cloned().unwrap_or(Value::Null);
    let opened_tab_id = opened.get("id").and_then(|v| {
        v.as_i64()
            .map(|n| n.to_string())
            .or_else(|| v.as_str().map(str::to_string))
    });
    let executor_session_key = value
        .get("session_key")
        .and_then(Value::as_str)
        .map(str::to_string);
    let opened_session_key = if let Some(tab_id) = opened_tab_id.as_deref() {
        match executor_session_key.as_deref() {
            Some(executor_key) => derive_session_key(executor_key, tab_id),
            None => find_unique_session_key_by_tab_id(state, tab_id).await,
        }
    } else {
        None
    };
    json!({
        "status": "success",
        "opened_tab_id": opened_tab_id,
        "opened_session_key": opened_session_key,
        "window_id": opened.get("windowId").cloned().unwrap_or(Value::Null),
        "window": opened.get("window").and_then(Value::as_bool).unwrap_or(false),
        "url": opened.get("url").cloned().unwrap_or(Value::Null),
        "title": opened.get("title").cloned().unwrap_or(Value::Null),
        "active": opened.get("active").cloned().unwrap_or(Value::Null),
        "group": opened.get("group").cloned().unwrap_or(Value::Null),
        "metadata": {
            "executor": {
                "tab_id": value.get("tab_id").cloned().unwrap_or(Value::Null),
                "session_key": value.get("session_key").cloned().unwrap_or(Value::Null)
            },
            "raw": opened
        }
    })
}

async fn find_unique_session_key_by_tab_id(state: &AppState, tab_id: &str) -> Option<String> {
    let driver = state.driver.lock().await;
    let mut matches = driver
        .sessions
        .values()
        .filter(|session| session.is_active() && session.tab_id == tab_id)
        .map(|session| session.session_key.clone());
    let session_key = matches.next()?;
    if matches.next().is_some() {
        return None;
    }
    Some(session_key)
}

fn derive_session_key(executor_session_key: &str, opened_tab_id: &str) -> Option<String> {
    let mut parts = executor_session_key.splitn(3, ':');
    let browser_id = parts.next()?;
    let profile_id = parts.next()?;
    let executor_tab_id = parts.next()?;
    if browser_id.is_empty() || profile_id.is_empty() || executor_tab_id.is_empty() {
        return None;
    }
    Some(crate::protocol::make_session_key(
        browser_id,
        profile_id,
        opened_tab_id,
    ))
}

async fn close(State(state): State<AppState>, Json(req): Json<CloseRequest>) -> Json<Value> {
    touch(&state).await;
    let selector = SessionSelector::new(Some(req.tab_id), req.browser, req.profile);
    let session_key = match select_tab(&state, selector).await {
        Ok(value) => value,
        Err(err) => return Json(json!({ "ok": false, "error": err.to_string() })),
    };
    let physical_tab_id = match session_tab_id(&state, &session_key).await {
        Ok(value) => value,
        Err(err) => return Json(json!({ "ok": false, "error": err.to_string() })),
    };
    let payload = json!({
        "cmd": "closeTab",
        "tabId": physical_tab_id.parse::<i64>().unwrap_or_default(),
    })
    .to_string();
    let result = execute_page_js(
        &state,
        &payload,
        SessionSelector::new(Some(session_key.clone()), None, None),
        true,
    )
    .await;
    {
        let mut driver = state.driver.lock().await;
        driver.sessions.remove(&session_key);
        driver.snapshots.remove(&session_key);
        driver
            .active_exec_sessions
            .retain(|_, active_session_key| active_session_key != &session_key);
        if driver.default_session_key.as_deref() == Some(&session_key) {
            driver.default_session_key = driver
                .latest_session_key
                .clone()
                .filter(|id| id != &session_key);
        }
        if driver.latest_session_key.as_deref() == Some(&session_key) {
            driver.latest_session_key = None;
        }
    }
    Json(match result {
        Ok(value) => json!({ "ok": true, "result": value }),
        Err(err) => json!({ "ok": false, "error": err.to_string() }),
    })
}

async fn shutdown(State(state): State<AppState>) -> Json<Value> {
    touch(&state).await;
    let cleanup = cleanup_on_shutdown(&state).await;
    let _ = state.shutdown.send(());
    Json(json!({
        "ok": true,
        "status": "shutdown_requested",
        "cleanup": cleanup
    }))
}

async fn cleanup_on_shutdown(state: &AppState) -> Value {
    let (snapshot_count, pending_count, active_exec_count, sessions) = {
        let mut driver = state.driver.lock().await;
        let snapshot_count = driver.snapshots.len();
        let pending_count = driver.pending.len();
        let active_exec_count = driver.active_exec_sessions.len();
        let sessions: Vec<(String, String, tokio::sync::mpsc::UnboundedSender<String>)> = driver
            .sessions
            .iter()
            .filter(|(_, session)| session.is_active())
            .map(|(key, session)| (key.clone(), session.tab_id.clone(), session.sender.clone()))
            .collect();
        driver.snapshots.clear();
        driver.pending.clear();
        driver.active_exec_sessions.clear();
        driver.acked.clear();
        (snapshot_count, pending_count, active_exec_count, sessions)
    };

    let mut plugin_results = Vec::new();
    for (session_key, tab_id, sender) in sessions {
        let exec_id = uuid::Uuid::new_v4().to_string();
        let payload = json!({
            "id": exec_id,
            "code": json!({ "cmd": "debugClearAll" }).to_string(),
            "tabId": tab_id.parse::<i64>().unwrap_or_default()
        })
        .to_string();
        let ok = sender.send(payload).is_ok();
        plugin_results.push(json!({ "session_key": session_key, "tab_id": tab_id, "sent": ok }));
    }

    json!({
        "daemon": {
            "snapshots": snapshot_count,
            "pending": pending_count,
            "active_exec_sessions": active_exec_count
        },
        "plugin": {
            "sent": plugin_results
        }
    })
}

fn normalize_profile_label(label: Option<String>) -> Result<Option<String>> {
    let Some(label) = label else {
        return Ok(None);
    };
    let trimmed = label.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    if trimmed.chars().count() > 40 {
        return Err(anyhow!("profile label 长度不能超过 40 个字符"));
    }
    if !trimmed
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
    {
        return Err(anyhow!("profile label 只能包含英文、数字、-、_、."));
    }
    Ok(Some(trimmed.to_string()))
}

async fn update_profile_label(state: &AppState, req: ProfileLabelRequest) -> Result<Value> {
    let label = normalize_profile_label(req.label)?;
    let session_key = select_tab(
        state,
        SessionSelector::new(req.switch_tab_id, req.browser, req.profile),
    )
    .await?;
    let (profile_id, browser_id, tab_id, sender) = {
        let driver = state.driver.lock().await;
        let session = driver
            .sessions
            .get(&session_key)
            .filter(|s| s.is_active())
            .ok_or_else(|| anyhow!("会话ID {session_key} 未连接"))?;
        if let Some(label) = label.as_deref() {
            let duplicate = driver.sessions.values().find(|s| {
                s.is_active()
                    && s.profile_id != session.profile_id
                    && s.profile_label.as_deref() == Some(label)
            });
            if let Some(duplicate) = duplicate {
                return Err(anyhow!(
                    "profile label {label:?} 已被 profile {} 使用",
                    duplicate.profile_id
                ));
            }
        }
        (
            session.profile_id.clone(),
            session.browser_id.clone(),
            session.tab_id.clone(),
            session.sender.clone(),
        )
    };

    let exec_id = Uuid::new_v4().to_string();
    let payload = json!({
        "id": exec_id,
        "code": json!({ "cmd": "setProfileLabel", "label": label }).to_string(),
        "tabId": tab_id.parse::<i64>().unwrap_or_default()
    })
    .to_string();
    let (tx, rx) = oneshot::channel();
    {
        let mut driver = state.driver.lock().await;
        driver.pending.insert(
            exec_id.clone(),
            crate::protocol::PendingExec {
                delivered_at: None,
                tx,
            },
        );
        driver
            .active_exec_sessions
            .insert(exec_id.clone(), session_key.clone());
    }
    sender
        .send(payload)
        .map_err(|_| anyhow!("浏览器扩展连接已断开"))?;
    match tokio::time::timeout(Duration::from_secs(10), rx).await {
        Ok(Ok(Ok(_))) => {}
        Ok(Ok(Err(err))) => return Err(err),
        Ok(Err(_)) => return Err(anyhow!("执行结果通道已关闭")),
        Err(_) => {
            let mut driver = state.driver.lock().await;
            driver.acked.remove(&exec_id);
            driver.pending.remove(&exec_id);
            driver.active_exec_sessions.remove(&exec_id);
            return Err(anyhow!("设置 profile label 超时"));
        }
    }

    let mut driver = state.driver.lock().await;
    for session in driver.sessions.values_mut() {
        if session.profile_id == profile_id && session.browser_id == browser_id {
            session.profile_label = label.clone();
            session.info.profile_label = label.clone();
        }
    }
    Ok(json!({
        "status": "success",
        "profile_id": profile_id,
        "browser_id": browser_id,
        "profile_label": label,
    }))
}

async fn active_tabs(state: &AppState, wait_ready: bool) -> Vec<TabInfo> {
    active_tabs_filtered(state, wait_ready, None, None)
        .await
        .unwrap_or_default()
}

async fn active_tabs_filtered(
    state: &AppState,
    wait_ready: bool,
    browser: Option<&str>,
    profile: Option<&str>,
) -> Result<Vec<TabInfo>> {
    if wait_ready {
        wait_for_sessions(state, Duration::from_secs(5)).await;
    }
    let driver = state.driver.lock().await;
    if let Some(profile) = profile {
        let matched_profile_ids: HashSet<String> = driver
            .sessions
            .values()
            .filter(|s| s.is_active())
            .filter(|s| browser.map(|b| s.browser_id == b).unwrap_or(true))
            .filter(|s| s.profile_label.as_deref() == Some(profile))
            .map(|s| s.profile_id.clone())
            .collect();
        if matched_profile_ids.len() > 1 {
            let mut ids: Vec<String> = matched_profile_ids.into_iter().collect();
            ids.sort();
            return Err(anyhow!(
                "profile label {profile:?} 存在歧义，匹配到多个 profile: {}",
                ids.join(", ")
            ));
        }
    }
    Ok(driver
        .sessions
        .values()
        .filter(|s| s.is_active())
        .filter(|s| browser.map(|b| s.browser_id == b).unwrap_or(true))
        .filter(|s| {
            profile
                .map(|p| s.profile_id == p || s.profile_label.as_deref() == Some(p))
                .unwrap_or(true)
        })
        .map(|s| {
            let mut info = s.info.clone();
            if info.url.len() > 50 {
                info.url = format!("{}...", info.url.chars().take(50).collect::<String>());
            }
            info
        })
        .collect())
}

async fn extension_profile_capabilities(state: &AppState) -> Vec<Value> {
    let driver = state.driver.lock().await;
    let mut profiles: HashMap<(String, String), Value> = HashMap::new();
    for session in driver
        .sessions
        .values()
        .filter(|session| session.is_active())
    {
        profiles
            .entry((session.browser_id.clone(), session.profile_id.clone()))
            .or_insert_with(|| {
                json!({
                    "browser_id": session.browser_id,
                    "profile_id": session.profile_id,
                    "profile_label": session.profile_label,
                    "extension_version": session.extension_version,
                    "file_scheme_access": session.file_scheme_access,
                })
            });
    }
    let mut values: Vec<Value> = profiles.into_values().collect();
    values.sort_by(|a, b| {
        a.get("profile_id")
            .and_then(Value::as_str)
            .unwrap_or("")
            .cmp(b.get("profile_id").and_then(Value::as_str).unwrap_or(""))
    });
    values
}

async fn has_extension_connection(state: &AppState) -> bool {
    let driver = state.driver.lock().await;
    driver
        .sessions
        .values()
        .any(|session| session.info.tab_type == "ext_ws" && session.is_active())
}

async fn wait_for_sessions(state: &AppState, timeout: Duration) -> bool {
    if has_active_sessions(state).await {
        return true;
    }
    let deadline = Instant::now() + timeout;
    loop {
        let now = Instant::now();
        if now >= deadline {
            return has_active_sessions(state).await;
        }
        let remaining = deadline.saturating_duration_since(now);
        tokio::select! {
            _ = state.sessions_ready.notified() => {
                if has_active_sessions(state).await {
                    return true;
                }
            }
            _ = tokio::time::sleep(remaining.min(Duration::from_millis(200))) => {
                if has_active_sessions(state).await {
                    return true;
                }
            }
        }
    }
}

async fn has_active_sessions(state: &AppState) -> bool {
    let driver = state.driver.lock().await;
    driver.sessions.values().any(|s| s.is_active())
}

async fn scan_page(
    state: &AppState,
    tabs_only: bool,
    selector: SessionSelector,
    text_only: bool,
) -> Result<Value> {
    if selector.tab_id.is_some() || selector.browser.is_some() || selector.profile.is_some() {
        let session_key = select_tab(state, selector.clone()).await?;
        state.driver.lock().await.default_session_key = Some(session_key);
    }
    let tabs = active_tabs(state, true).await;
    if tabs.is_empty() {
        return Ok(
            json!({ "status": "error", "msg": "没有可用的浏览器标签页，查L3记忆分析原因。" }),
        );
    }
    let default_session_key = state.driver.lock().await.default_session_key.clone();
    let mut result = json!({
        "status": "success",
        "metadata": {
            "tabs_count": tabs.len(),
            "tabs": tabs,
            "active_tab": default_session_key,
        }
    });
    if !tabs_only {
        let content = get_html(state, true, 35000, text_only).await?;
        result["content"] = Value::String(content);
    }
    Ok(result)
}

async fn execute_page_js(
    state: &AppState,
    script: &str,
    selector: SessionSelector,
    no_monitor: bool,
) -> Result<Value> {
    let executor_session_key = select_tab(state, selector).await?;
    let before = if no_monitor {
        None
    } else {
        get_html(state, false, 9_999_999, false).await.ok()
    };
    let before_tabs: std::collections::HashSet<String> = active_tabs(state, true)
        .await
        .into_iter()
        .map(|t| t.session_key)
        .collect();
    let execution = execute_raw_js_with_session(
        state,
        script,
        Duration::from_secs(15),
        Some(&executor_session_key),
    )
    .await?;
    let current_session_key = execution.session_key;
    let current_tab_id = execution.tab_id;
    let response = execution.result;
    let mut result = json!({
        "status": "success",
        "js_return": response.data.or(response.result).unwrap_or(Value::Null),
        "tab_id": current_tab_id,
        "session_key": current_session_key,
    });
    if let Some(tabs) = response.new_tabs {
        result["newTabs"] = tabs;
    } else {
        let after_tabs = active_tabs(state, false).await;
        let new_tabs: Vec<_> = after_tabs
            .into_iter()
            .filter(|t| !before_tabs.contains(&t.session_key))
            .map(|t| json!({ "id": t.id, "tab_id": t.tab_id, "session_key": t.session_key, "url": t.url }))
            .collect();
        if !new_tabs.is_empty() {
            result["newTabs"] = json!(new_tabs);
        }
    }
    if !no_monitor {
        if let Some(before_html) = before {
            if let Ok(current_html) = get_html(state, false, 9_999_999, false).await {
                result["change"] = html::changed_elements(&before_html, &current_html);
            }
        }
    }
    Ok(result)
}

struct ExecWithSession {
    result: ExecResult,
    session_key: String,
    tab_id: String,
}

async fn execute_raw_js(state: &AppState, code: &str, timeout: Duration) -> Result<ExecResult> {
    Ok(execute_raw_js_with_session(state, code, timeout, None)
        .await?
        .result)
}

async fn execute_raw_js_with_session(
    state: &AppState,
    code: &str,
    timeout: Duration,
    requested_session_key: Option<&str>,
) -> Result<ExecWithSession> {
    let (session_key, tab_id, sender) = {
        wait_for_sessions(state, Duration::from_secs(5)).await;
        let driver = state.driver.lock().await;
        let session_key = if let Some(requested) = requested_session_key {
            driver
                .sessions
                .get(requested)
                .filter(|session| session.is_active())
                .map(|session| session.session_key.clone())
                .ok_or_else(|| anyhow!("会话ID {requested} 未连接"))?
        } else {
            driver
                .default_session_key
                .as_ref()
                .and_then(|key| {
                    driver
                        .sessions
                        .get(key)
                        .filter(|s| s.is_active())
                        .map(|s| s.session_key.clone())
                })
                .or_else(|| {
                    driver.latest_session_key.as_ref().and_then(|key| {
                        driver
                            .sessions
                            .get(key)
                            .filter(|s| s.is_active())
                            .map(|s| s.session_key.clone())
                    })
                })
                .or_else(|| {
                    driver
                        .sessions
                        .values()
                        .find(|s| s.is_active())
                        .map(|s| s.session_key.clone())
                })
                .ok_or_else(|| anyhow!("没有可用的浏览器标签页，查L3记忆分析原因。"))?
        };
        let session = driver
            .sessions
            .get(&session_key)
            .filter(|s| s.is_active())
            .ok_or_else(|| anyhow!("会话ID {session_key} 未连接"))?;
        (session_key, session.tab_id.clone(), session.sender.clone())
    };
    let exec_id = Uuid::new_v4().to_string();
    let payload =
        json!({ "id": exec_id, "code": code, "tabId": tab_id.parse::<i64>().unwrap_or_default() })
            .to_string();
    let (tx, rx) = oneshot::channel();
    {
        let mut driver = state.driver.lock().await;
        driver.pending.insert(
            exec_id.clone(),
            crate::protocol::PendingExec {
                delivered_at: None,
                tx,
            },
        );
        driver
            .active_exec_sessions
            .insert(exec_id.clone(), session_key.clone());
    }
    sender
        .send(payload)
        .map_err(|_| anyhow!("浏览器扩展连接已断开"))?;
    let result = match tokio::time::timeout(timeout, rx).await {
        Ok(Ok(value)) => value,
        Ok(Err(_)) => Err(anyhow!("执行结果通道已关闭")),
        Err(_) => {
            let mut driver = state.driver.lock().await;
            let acked = driver.acked.remove(&exec_id);
            driver.pending.remove(&exec_id);
            driver.active_exec_sessions.remove(&exec_id);
            if acked {
                Ok(ExecResult {
                    data: None,
                    result: Some(json!(format!(
                        "No response data in {}s (ACK received, script may still be running)",
                        timeout.as_secs()
                    ))),
                    closed: None,
                    new_tabs: None,
                })
            } else {
                Ok(ExecResult {
                    data: None,
                    result: Some(json!(format!(
                        "No response data in {}s (no ACK, script may not have been delivered)",
                        timeout.as_secs()
                    ))),
                    closed: None,
                    new_tabs: None,
                })
            }
        }
    }?;
    Ok(ExecWithSession {
        result,
        session_key,
        tab_id,
    })
}

async fn get_html(
    state: &AppState,
    cutlist: bool,
    maxchars: usize,
    text_only: bool,
) -> Result<String> {
    let opt = html::js_opt_html();
    let page_script = format!(
        "{opt}\nreturn optHTML({});",
        if text_only { "true" } else { "false" }
    );
    let response = execute_raw_js(state, &page_script, Duration::from_secs(30)).await?;
    let mut page = response
        .data
        .unwrap_or(Value::Null)
        .as_str()
        .unwrap_or_default()
        .to_string();
    if text_only {
        return Ok(clean_text(&page));
    }
    page = html::optimize_html_for_tokens(&page);
    if cutlist {
        let list_script = format!(
            "{}\nreturn findMainList(document.body);",
            html::js_find_main_list()
        );
        let _ = execute_raw_js(state, &list_script, Duration::from_secs(10)).await;
    }
    if page.len() > maxchars {
        page = html::smart_truncate(page, maxchars);
    }
    Ok(page)
}

fn clean_text(input: &str) -> String {
    input
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

fn is_extension_json(script: &str) -> bool {
    let trimmed = script.trim();
    trimmed.starts_with('{')
        && serde_json::from_str::<Value>(trimmed)
            .ok()
            .and_then(|v| v.get("cmd").cloned())
            .is_some()
}

fn wrap_script_with_wait(script: &str, wait_js: &str, timeout: f64, interval: f64) -> String {
    format!(
        r#"
const __agentBrowserMain = {script_json};
const __agentBrowserWait = {wait_json};
const __agentBrowserTimeoutMs = {timeout_ms};
const __agentBrowserIntervalMs = {interval_ms};
const AsyncFunction = Object.getPrototypeOf(async function(){{}}).constructor;
const __runUser = async (code) => {{
  const trimmed = String(code || '').trim();
  if (!trimmed) return undefined;
  if (/^return\b/.test(trimmed)) return await (new AsyncFunction(trimmed))();
  try {{
    const value = eval(trimmed);
    return value instanceof Promise ? await value : value;
  }} catch (e) {{
    if (e instanceof SyntaxError && (/return/i.test(e.message) || /await/i.test(e.message))) {{
      return await (new AsyncFunction(trimmed))();
    }}
    throw e;
  }}
}};
const __mainResult = await __runUser(__agentBrowserMain);
let __matched = false;
let __waitValue = undefined;
let __waitError = null;
const __deadline = Date.now() + __agentBrowserTimeoutMs;
while (true) {{
  try {{
    __waitValue = await __runUser(__agentBrowserWait);
    __waitError = null;
    if (__waitValue) {{ __matched = true; break; }}
  }} catch (e) {{
    __waitError = e.message || String(e);
  }}
  if (Date.now() >= __deadline) break;
  await new Promise(resolve => setTimeout(resolve, __agentBrowserIntervalMs));
}}
return {{ result: __mainResult, wait: {{ ok: __matched, matched: __matched, value: __waitValue, error: __waitError }} }};
"#,
        script_json = serde_json::to_string(script).unwrap_or_else(|_| "\"\"".to_string()),
        wait_json = serde_json::to_string(wait_js).unwrap_or_else(|_| "\"\"".to_string()),
        timeout_ms = (timeout.max(0.0) * 1000.0) as u64,
        interval_ms = ((interval.max(0.02)) * 1000.0) as u64,
    )
}

async fn touch(state: &AppState) {
    *state.last_activity.lock().await = Instant::now();
}

#[allow(dead_code)]
async fn not_found() -> (StatusCode, Json<Value>) {
    (
        StatusCode::NOT_FOUND,
        Json(json!({ "ok": false, "error": "not found" })),
    )
}

#[derive(Debug, Clone, Serialize)]
struct CdpCallResult {
    ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    data: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OperationKind {
    Click,
    Fill,
    MouseClick,
}

#[derive(Debug, Clone)]
struct OperationOptions {
    monitor: bool,
    wait_js: Option<String>,
    wait_timeout: f64,
    wait_interval: f64,
    timeout: f64,
}

#[derive(Debug, Deserialize)]
struct AxNode {
    #[serde(rename = "nodeId")]
    node_id: String,
    #[serde(rename = "childIds", default)]
    child_ids: Vec<String>,
    #[serde(rename = "backendDOMNodeId")]
    backend_dom_node_id: Option<i64>,
    role: Option<AxValue>,
    name: Option<AxValue>,
    value: Option<AxValue>,
    description: Option<AxValue>,
}

#[derive(Debug, Deserialize)]
struct AxValue {
    value: Option<Value>,
    #[serde(rename = "type")]
    value_type: Option<String>,
}

async fn snapshot(State(state): State<AppState>, Json(req): Json<SnapshotRequest>) -> Json<Value> {
    touch(&state).await;
    Json(match snapshot_page(&state, req).await {
        Ok(value) => json!({ "ok": true, "result": value }),
        Err(err) => json!({ "ok": false, "error": err.to_string() }),
    })
}

async fn click(State(state): State<AppState>, Json(req): Json<TargetActionRequest>) -> Json<Value> {
    touch(&state).await;
    let options = OperationOptions {
        monitor: req.monitor,
        wait_js: req.wait_js,
        wait_timeout: req.wait_timeout,
        wait_interval: req.wait_interval,
        timeout: req.timeout,
    };
    Json(
        match run_target_operation(
            &state,
            OperationKind::Click,
            SessionSelector::new(req.switch_tab_id, req.browser, req.profile),
            &req.target,
            None,
            options,
        )
        .await
        {
            Ok(value) => json!({ "ok": true, "result": value }),
            Err(err) => json!({ "ok": false, "error": err.to_string() }),
        },
    )
}

async fn fill(State(state): State<AppState>, Json(req): Json<FillRequest>) -> Json<Value> {
    touch(&state).await;
    if req.clear && req.has_value {
        return Json(json!({ "ok": false, "error": "fill: --clear 不能和 value 同时使用" }));
    }
    if req.clear && req.append {
        return Json(json!({ "ok": false, "error": "fill: --clear 不能和 --append 同时使用" }));
    }
    if !req.clear && !req.has_value {
        return Json(
            json!({ "ok": false, "error": "fill: value is required unless --clear is used" }),
        );
    }
    let value = if req.clear { String::new() } else { req.value };
    let options = OperationOptions {
        monitor: req.monitor,
        wait_js: req.wait_js,
        wait_timeout: req.wait_timeout,
        wait_interval: req.wait_interval,
        timeout: req.timeout,
    };
    let payload = json!({ "value": value, "append": req.append, "clear": req.clear });
    Json(
        match run_target_operation(
            &state,
            OperationKind::Fill,
            SessionSelector::new(req.switch_tab_id, req.browser, req.profile),
            &req.target,
            Some(payload),
            options,
        )
        .await
        {
            Ok(value) => json!({ "ok": true, "result": value }),
            Err(err) => json!({ "ok": false, "error": err.to_string() }),
        },
    )
}

async fn mouse_click(
    State(state): State<AppState>,
    Json(req): Json<TargetActionRequest>,
) -> Json<Value> {
    touch(&state).await;
    let options = OperationOptions {
        monitor: req.monitor,
        wait_js: req.wait_js,
        wait_timeout: req.wait_timeout,
        wait_interval: req.wait_interval,
        timeout: req.timeout,
    };
    Json(
        match run_target_operation(
            &state,
            OperationKind::MouseClick,
            SessionSelector::new(req.switch_tab_id, req.browser, req.profile),
            &req.target,
            None,
            options,
        )
        .await
        {
            Ok(value) => json!({ "ok": true, "result": value }),
            Err(err) => json!({ "ok": false, "error": err.to_string() }),
        },
    )
}

async fn send_keys(State(state): State<AppState>, Json(req): Json<SendKeysRequest>) -> Json<Value> {
    touch(&state).await;
    let options = OperationOptions {
        monitor: req.monitor,
        wait_js: req.wait_js,
        wait_timeout: req.wait_timeout,
        wait_interval: req.wait_interval,
        timeout: req.timeout,
    };
    let payload = json!({ "keys": req.keys, "target": req.target });
    Json(
        match run_send_keys_operation(
            &state,
            SessionSelector::new(req.switch_tab_id, req.browser, req.profile),
            payload,
            options,
        )
        .await
        {
            Ok(value) => json!({ "ok": true, "result": value }),
            Err(err) => json!({ "ok": false, "error": err.to_string() }),
        },
    )
}

async fn snapshot_page(state: &AppState, req: SnapshotRequest) -> Result<Value> {
    let limit = req.limit.clamp(1, 1000);
    let tab_id = select_tab(
        state,
        SessionSelector::new(req.switch_tab_id, req.browser, req.profile),
    )
    .await?;
    let timeout = Duration::from_secs_f64(req.timeout.max(0.1));
    let tabs = active_tabs(state, true).await;
    let tab = tabs
        .iter()
        .find(|tab| tab.session_key == tab_id)
        .cloned()
        .ok_or_else(|| anyhow!("snapshot: tab {tab_id} is not active"))?;
    let ax = cdp_call(
        state,
        &tab_id,
        "Accessibility.getFullAXTree",
        json!({}),
        timeout,
    )
    .await?;
    if !ax.ok {
        return Err(anyhow!(ax.error.unwrap_or_else(|| {
            "snapshot: Accessibility.getFullAXTree failed".to_string()
        })));
    }
    let nodes_value = ax
        .data
        .and_then(|v| v.get("nodes").cloned())
        .ok_or_else(|| anyhow!("snapshot: CDP response missing nodes"))?;
    let nodes: Vec<AxNode> = serde_json::from_value(nodes_value)?;
    let mut builder = SnapshotBuilder::new(req.offset, limit, req.details);
    let tree = builder.build(nodes);
    let refs_for_details: Vec<_> = builder
        .refs
        .values()
        .filter(|entry| entry.index >= req.offset && entry.index < req.offset + limit)
        .cloned()
        .collect();
    let mut all_refs = builder.refs;
    let detail_map =
        collect_dom_details(state, &tab_id, &refs_for_details, req.details, timeout).await;
    for (ref_id, detail) in detail_map {
        if let Some(entry) = all_refs.get_mut(&ref_id) {
            entry.dom = Some(detail);
        }
    }
    let tree = if req.details {
        attach_details_to_tree(tree, &all_refs)
    } else {
        tree
    };
    let total = all_refs.len();
    let returned = all_refs
        .values()
        .filter(|entry| entry.index >= req.offset && entry.index < req.offset + limit)
        .count();
    // `active_tabs` 返回的是面向 CLI 展示的 TabInfo，其中长 URL 会被截断。
    // snapshot 缓存必须保存完整 URL，否则后续 `@e` 操作会把截断 URL
    // 和页面真实 `location.href` 比较，误判为 `ref expired`。
    let snapshot_url = current_location(state, &tab_id, timeout)
        .await
        .unwrap_or_else(|_| tab.url.clone());
    {
        let mut driver = state.driver.lock().await;
        let generation = driver
            .snapshots
            .get(&tab_id)
            .map(|cache| cache.generation + 1)
            .unwrap_or(1);
        driver.snapshots.insert(
            tab_id.clone(),
            SnapshotCache {
                generation,
                url: snapshot_url,
                refs: all_refs,
            },
        );
    }
    let has_more = req.offset + returned < total;
    Ok(json!({
        "status": "success",
        "url": tab.url,
        "title": tab.title,
        "tab_id": tab.tab_id,
        "session_key": tab_id,
        "tree": tree,
        "pagination": {
            "total_operable": total,
            "offset": req.offset,
            "limit": limit,
            "returned": returned,
            "has_more": has_more,
            "next_offset": if has_more { Some(req.offset + limit) } else { None::<usize> }
        }
    }))
}

struct SnapshotBuilder {
    offset: usize,
    limit: usize,
    operable_seen: usize,
    refs: HashMap<String, ElementRef>,
    nodes: HashMap<String, AxNode>,
}

impl SnapshotBuilder {
    fn new(offset: usize, limit: usize, _details: bool) -> Self {
        Self {
            offset,
            limit,
            operable_seen: 0,
            refs: HashMap::new(),
            nodes: HashMap::new(),
        }
    }

    fn build(&mut self, nodes: Vec<AxNode>) -> Vec<Value> {
        let mut child_ids = std::collections::HashSet::new();
        for node in &nodes {
            for child_id in &node.child_ids {
                child_ids.insert(child_id.clone());
            }
        }
        let mut roots: Vec<String> = nodes
            .iter()
            .filter(|node| !child_ids.contains(&node.node_id))
            .map(|node| node.node_id.clone())
            .collect();
        if roots.is_empty() {
            if let Some(first) = nodes.first() {
                roots.push(first.node_id.clone());
            }
        }
        self.nodes = nodes
            .into_iter()
            .map(|node| (node.node_id.clone(), node))
            .collect();
        roots
            .iter()
            .filter_map(|id| self.format_node(id))
            .flat_map(|value| match value {
                Value::Array(items) => items,
                other => vec![other],
            })
            .collect()
    }

    fn format_node(&mut self, id: &str) -> Option<Value> {
        let (role, name, value, description, backend_dom_node_id, child_ids) = {
            let node = self.nodes.get(id)?;
            (
                ax_string(&node.role),
                ax_string(&node.name),
                ax_string(&node.value),
                ax_string(&node.description),
                node.backend_dom_node_id,
                node.child_ids.clone(),
            )
        };
        let mut children = Vec::new();
        for child_id in child_ids {
            if let Some(child) = self.format_node(&child_id) {
                match child {
                    Value::Array(items) => children.extend(items),
                    other => children.push(other),
                }
            }
        }
        let role_str = role.unwrap_or_default();
        let skip_role = role_str.is_empty() || role_str == "none" || role_str == "generic";
        let operable = is_operable_role(&role_str) && backend_dom_node_id.is_some();
        let mut obj = serde_json::Map::new();
        obj.insert("role".to_string(), json!(role_str));
        if let Some(name) = non_empty(name) {
            obj.insert("name".to_string(), json!(truncate_text(&name, 160)));
        }
        if let Some(value) = non_empty(value) {
            obj.insert("value".to_string(), json!(truncate_text(&value, 160)));
        }
        if let Some(description) = non_empty(description.clone()) {
            obj.insert(
                "description".to_string(),
                json!(truncate_text(&description, 160)),
            );
        }
        let mut include_self = false;
        if operable {
            self.operable_seen += 1;
            let index = self.operable_seen - 1;
            let ref_id = format!("@e{}", self.operable_seen);
            self.refs.insert(
                ref_id.clone(),
                ElementRef {
                    ref_id: ref_id.clone(),
                    backend_dom_node_id: backend_dom_node_id.unwrap_or_default(),
                    index,
                    role: role_str.clone(),
                    name: obj
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                    value: obj.get("value").and_then(Value::as_str).map(str::to_string),
                    description,
                    dom: None,
                },
            );
            if index >= self.offset && index < self.offset + self.limit {
                include_self = true;
                obj.insert("ref".to_string(), json!(ref_id));
            }
        }
        if skip_role {
            if children.is_empty() {
                None
            } else if children.len() == 1 {
                children.into_iter().next()
            } else {
                Some(Value::Array(children))
            }
        } else {
            if !children.is_empty() {
                obj.insert("children".to_string(), Value::Array(children));
            }
            if include_self || obj.contains_key("children") || role_is_context(&role_str) {
                Some(Value::Object(obj))
            } else {
                None
            }
        }
    }
}

fn attach_details_to_tree(mut tree: Vec<Value>, refs: &HashMap<String, ElementRef>) -> Vec<Value> {
    for item in &mut tree {
        attach_details_to_node(item, refs);
    }
    tree
}

fn attach_details_to_node(node: &mut Value, refs: &HashMap<String, ElementRef>) {
    let Some(obj) = node.as_object_mut() else {
        return;
    };
    if let Some(ref_id) = obj.get("ref").and_then(Value::as_str) {
        if let Some(entry) = refs.get(ref_id).and_then(|entry| entry.dom.as_ref()) {
            if let Ok(Value::Object(map)) = serde_json::to_value(entry) {
                for (key, value) in map {
                    if !value.is_null() {
                        obj.insert(key, value);
                    }
                }
            }
        }
    }
    if let Some(children) = obj.get_mut("children").and_then(Value::as_array_mut) {
        for child in children {
            attach_details_to_node(child, refs);
        }
    }
}

async fn collect_dom_details(
    state: &AppState,
    tab_id: &str,
    refs: &[ElementRef],
    include_errors: bool,
    timeout: Duration,
) -> HashMap<String, ElementDomInfo> {
    let mut map = HashMap::new();
    for entry in refs {
        let detail = match element_dom_info(state, tab_id, entry.backend_dom_node_id, timeout).await
        {
            Ok(value) => value,
            Err(err) => ElementDomInfo {
                tag: None,
                text: None,
                placeholder: None,
                input_type: None,
                href: None,
                disabled: None,
                readonly: None,
                checked: None,
                selected: None,
                rect: None,
                selector: None,
                visible: None,
                dom_error: include_errors.then(|| err.to_string()),
            },
        };
        map.insert(entry.ref_id.clone(), detail);
    }
    map
}

async fn element_dom_info(
    state: &AppState,
    tab_id: &str,
    backend_dom_node_id: i64,
    timeout: Duration,
) -> Result<ElementDomInfo> {
    let script = r#"function() {
      const el = this;
      const rect = el.getBoundingClientRect ? el.getBoundingClientRect() : null;
      const style = el.ownerDocument.defaultView.getComputedStyle(el);
      const visible = !!rect && rect.width > 0 && rect.height > 0 && style.visibility !== 'hidden' && style.display !== 'none';
      function cssPath(node) {
        if (!node || node.nodeType !== 1) return '';
        if (node.id) return '#' + CSS.escape(node.id);
        const parts = [];
        let cur = node;
        while (cur && cur.nodeType === 1 && parts.length < 5) {
          let part = cur.localName;
          if (cur.classList && cur.classList.length) part += '.' + [...cur.classList].slice(0, 2).map(c => CSS.escape(c)).join('.');
          const parent = cur.parentElement;
          if (parent) {
            const same = [...parent.children].filter(c => c.localName === cur.localName);
            if (same.length > 1) part += `:nth-of-type(${same.indexOf(cur) + 1})`;
          }
          parts.unshift(part);
          cur = parent;
        }
        return parts.join(' > ');
      }
      const text = (el.innerText || el.textContent || '').trim().slice(0, 160);
      return {
        tag: el.tagName || '', text,
        placeholder: el.getAttribute && el.getAttribute('placeholder') || undefined,
        type: el.getAttribute && (el.getAttribute('type') || undefined),
        href: el.href ? String(el.href).slice(0, 300) : undefined,
        disabled: !!el.disabled,
        readonly: !!el.readOnly,
        checked: typeof el.checked === 'boolean' ? el.checked : undefined,
        selected: typeof el.selected === 'boolean' ? el.selected : undefined,
        rect: rect ? { x: rect.x, y: rect.y, width: rect.width, height: rect.height } : undefined,
        selector: cssPath(el), visible
      };
    }"#;
    let value = batch_cdp_call(
        state,
        tab_id,
        vec![
            json!({ "cmd": "cdp", "method": "DOM.resolveNode", "params": { "backendNodeId": backend_dom_node_id } }),
            json!({ "cmd": "cdp", "method": "Runtime.callFunctionOn", "params": { "objectId": "$0.object.objectId", "functionDeclaration": script, "returnByValue": true } }),
        ],
        timeout,
    )
    .await?;
    let value = value
        .get("results")
        .and_then(Value::as_array)
        .and_then(|items| items.get(1))
        .or_else(|| value.as_array().and_then(|items| items.get(1)))
        .and_then(|data| data.get("result").and_then(|r| r.get("value")).cloned())
        .ok_or_else(|| anyhow!("DOM detail response missing value"))?;
    let rect = value
        .get("rect")
        .and_then(|rect| serde_json::from_value::<RectInfo>(rect.clone()).ok());
    Ok(ElementDomInfo {
        tag: value.get("tag").and_then(Value::as_str).map(str::to_string),
        text: value
            .get("text")
            .and_then(Value::as_str)
            .map(str::to_string),
        placeholder: value
            .get("placeholder")
            .and_then(Value::as_str)
            .map(str::to_string),
        input_type: value
            .get("type")
            .and_then(Value::as_str)
            .map(str::to_string),
        href: value
            .get("href")
            .and_then(Value::as_str)
            .map(str::to_string),
        disabled: value.get("disabled").and_then(Value::as_bool),
        readonly: value.get("readonly").and_then(Value::as_bool),
        checked: value.get("checked").and_then(Value::as_bool),
        selected: value.get("selected").and_then(Value::as_bool),
        rect,
        selector: value
            .get("selector")
            .and_then(Value::as_str)
            .map(str::to_string),
        visible: value.get("visible").and_then(Value::as_bool),
        dom_error: None,
    })
}

async fn run_target_operation(
    state: &AppState,
    kind: OperationKind,
    selector: SessionSelector,
    target: &str,
    payload: Option<Value>,
    options: OperationOptions,
) -> Result<Value> {
    let tab_id = select_tab(state, selector).await?;
    let before = if options.monitor {
        get_html(state, false, 9_999_999, false).await.ok()
    } else {
        None
    };
    let timeout = Duration::from_secs_f64(options.timeout.max(0.1));
    let object_id = resolve_target_object(state, &tab_id, target, timeout).await?;
    let operation = match kind {
        OperationKind::Click => dom_click(state, &tab_id, &object_id, timeout).await?,
        OperationKind::Fill => {
            dom_fill(
                state,
                &tab_id,
                &object_id,
                payload.unwrap_or_else(|| json!({})),
                timeout,
            )
            .await?
        }
        OperationKind::MouseClick => dom_mouse_click(state, &tab_id, &object_id, timeout).await?,
    };
    finish_operation(state, tab_id, operation, before, options).await
}

async fn run_send_keys_operation(
    state: &AppState,
    selector: SessionSelector,
    payload: Value,
    options: OperationOptions,
) -> Result<Value> {
    let tab_id = select_tab(state, selector).await?;
    let before = if options.monitor {
        get_html(state, false, 9_999_999, false).await.ok()
    } else {
        None
    };
    let timeout = Duration::from_secs_f64(options.timeout.max(0.1));
    if let Some(target) = payload.get("target").and_then(Value::as_str) {
        let object_id = resolve_target_object(state, &tab_id, target, timeout).await?;
        focus_object(state, &tab_id, &object_id, timeout).await?;
    }
    let operation = dispatch_keys(
        state,
        &tab_id,
        payload
            .get("keys")
            .and_then(Value::as_str)
            .unwrap_or_default(),
        timeout,
    )
    .await?;
    finish_operation(state, tab_id, operation, before, options).await
}

async fn finish_operation(
    state: &AppState,
    session_key: String,
    operation: Value,
    before: Option<String>,
    options: OperationOptions,
) -> Result<Value> {
    let physical_tab_id = session_tab_id(state, &session_key)
        .await
        .unwrap_or_default();
    let mut result = json!({ "status": "success", "tab_id": physical_tab_id, "session_key": session_key, "operation": operation });
    if let Some(wait_js) = options.wait_js.as_deref() {
        let wait =
            wait_for_js_condition(state, wait_js, options.wait_timeout, options.wait_interval)
                .await?;
        if !wait.get("ok").and_then(Value::as_bool).unwrap_or(false) {
            return Err(anyhow!(json!({ "error": format!("wait-js timeout after {}s", options.wait_timeout), "operation": result["operation"].clone(), "wait": wait }).to_string()));
        }
        result["wait"] = wait;
    }
    if let Some(before_html) = before {
        match get_html(state, false, 9_999_999, false).await {
            Ok(after) => result["change"] = html::changed_elements(&before_html, &after),
            Err(err) => result["monitor"] = json!({ "ok": false, "error": err.to_string() }),
        }
    }
    Ok(result)
}

async fn wait_for_js_condition(
    state: &AppState,
    wait_js: &str,
    timeout: f64,
    interval: f64,
) -> Result<Value> {
    let script = format!(
        r#"
const __waitCode = {wait_json};
const __deadline = Date.now() + {timeout_ms};
const __interval = {interval_ms};
const AsyncFunction = Object.getPrototypeOf(async function(){{}}).constructor;
async function __run(code) {{
  const trimmed = String(code || '').trim();
  if (/^return\b/.test(trimmed)) return await (new AsyncFunction(trimmed))();
  const value = eval(trimmed);
  return value instanceof Promise ? await value : value;
}}
let value = null, error = null, ok = false;
while (Date.now() <= __deadline) {{
  try {{ value = await __run(__waitCode); error = null; if (value) {{ ok = true; break; }} }}
  catch (e) {{ error = e.message || String(e); }}
  await new Promise(resolve => setTimeout(resolve, __interval));
}}
return {{ ok, matched: ok, value, error }};
"#,
        wait_json = serde_json::to_string(wait_js).unwrap_or_else(|_| "\"\"".to_string()),
        timeout_ms = (timeout.max(0.0) * 1000.0) as u64,
        interval_ms = (interval.max(0.02) * 1000.0) as u64,
    );
    Ok(execute_raw_js(
        state,
        &script,
        Duration::from_secs_f64(timeout.max(0.1) + 1.0),
    )
    .await?
    .data
    .unwrap_or(Value::Null))
}

async fn select_tab(state: &AppState, selector: SessionSelector) -> Result<String> {
    wait_for_sessions(state, Duration::from_secs(5)).await;
    let mut driver = state.driver.lock().await;
    let mut matches: Vec<&Session> = driver
        .sessions
        .values()
        .filter(|s| s.is_active())
        .filter(|s| {
            selector
                .browser
                .as_deref()
                .map(|b| s.browser_id == b)
                .unwrap_or(true)
        })
        .filter(|s| {
            selector
                .profile
                .as_deref()
                .map(|p| s.profile_id == p || s.profile_label.as_deref() == Some(p))
                .unwrap_or(true)
        })
        .collect();

    if let Some(tab_id) = selector.tab_id.as_deref() {
        matches.retain(|s| s.tab_id == tab_id || s.session_key == tab_id);
        if matches.is_empty() {
            return Err(anyhow!("会话ID {tab_id} 未连接"));
        }
        if matches.len() > 1 {
            return Err(anyhow!(
                "ambiguous tab {tab_id}, matched {} sessions; please specify --profile or --browser",
                matches.len()
            ));
        }
        let session_key = matches[0].session_key.clone();
        driver.default_session_key = Some(session_key.clone());
        return Ok(session_key);
    }

    if let Some(profile) = selector.profile.as_deref() {
        let matched_profile_ids: HashSet<String> = matches
            .iter()
            .filter(|s| s.profile_label.as_deref() == Some(profile))
            .map(|s| s.profile_id.clone())
            .collect();
        if matched_profile_ids.len() > 1 {
            let mut ids: Vec<String> = matched_profile_ids.into_iter().collect();
            ids.sort();
            return Err(anyhow!(
                "profile label {profile:?} 存在歧义，匹配到多个 profile: {}",
                ids.join(", ")
            ));
        }
    }

    if selector.browser.is_some() || selector.profile.is_some() {
        let preferred = driver
            .default_session_key
            .as_ref()
            .and_then(|key| matches.iter().find(|s| &s.session_key == key))
            .or_else(|| {
                driver
                    .latest_session_key
                    .as_ref()
                    .and_then(|key| matches.iter().find(|s| &s.session_key == key))
            })
            .or_else(|| matches.first());
        let session_key = preferred
            .map(|s| s.session_key.clone())
            .ok_or_else(|| anyhow!("没有匹配 --browser/--profile 的可用浏览器标签页"))?;
        driver.default_session_key = Some(session_key.clone());
        return Ok(session_key);
    }

    let session_key = driver
        .default_session_key
        .as_ref()
        .and_then(|key| {
            driver
                .sessions
                .get(key)
                .filter(|s| s.is_active())
                .map(|s| s.session_key.clone())
        })
        .or_else(|| {
            driver.latest_session_key.as_ref().and_then(|key| {
                driver
                    .sessions
                    .get(key)
                    .filter(|s| s.is_active())
                    .map(|s| s.session_key.clone())
            })
        })
        .or_else(|| {
            driver
                .sessions
                .values()
                .find(|s| s.is_active())
                .map(|s| s.session_key.clone())
        })
        .ok_or_else(|| anyhow!("没有可用的浏览器标签页"))?;
    driver.default_session_key = Some(session_key.clone());
    Ok(session_key)
}

async fn session_tab_id(state: &AppState, session_key: &str) -> Result<String> {
    let driver = state.driver.lock().await;
    driver
        .sessions
        .get(session_key)
        .filter(|s| s.is_active())
        .map(|s| s.tab_id.clone())
        .ok_or_else(|| anyhow!("会话ID {session_key} 未连接"))
}

async fn cdp_call(
    state: &AppState,
    session_key: &str,
    method: &str,
    params: Value,
    timeout: Duration,
) -> Result<CdpCallResult> {
    let tab_id = session_tab_id(state, session_key).await?;
    let script = json!({ "cmd": "cdp", "tabId": tab_id.parse::<i64>().unwrap_or_default(), "method": method, "params": params }).to_string();
    let response = execute_raw_js_on_tab(state, &script, session_key, timeout).await?;
    let value = response.data.or(response.result).unwrap_or(Value::Null);
    match value.get("ok").and_then(Value::as_bool) {
        Some(true) => Ok(CdpCallResult {
            ok: true,
            data: value.get("data").cloned(),
            error: None,
        }),
        Some(false) => Ok(CdpCallResult {
            ok: false,
            data: value.get("data").cloned(),
            error: value
                .get("error")
                .and_then(Value::as_str)
                .map(str::to_string)
                .or_else(|| Some(value.to_string())),
        }),
        None => Ok(CdpCallResult {
            ok: true,
            data: Some(value),
            error: None,
        }),
    }
}

async fn execute_raw_js_on_tab(
    state: &AppState,
    code: &str,
    session_key: &str,
    timeout: Duration,
) -> Result<ExecResult> {
    let previous = {
        let mut driver = state.driver.lock().await;
        let previous = driver.default_session_key.clone();
        driver.default_session_key = Some(session_key.to_string());
        previous
    };
    let result = execute_raw_js(state, code, timeout).await;
    {
        let mut driver = state.driver.lock().await;
        if driver.default_session_key.as_deref() == Some(session_key) {
            driver.default_session_key = previous;
        }
    }
    result
}

async fn batch_cdp_call(
    state: &AppState,
    session_key: &str,
    commands: Vec<Value>,
    timeout: Duration,
) -> Result<Value> {
    let tab_id = session_tab_id(state, session_key).await?;
    let script = json!({
        "cmd": "batch",
        "tabId": tab_id.parse::<i64>().unwrap_or_default(),
        "commands": commands,
    })
    .to_string();
    let response = execute_raw_js_on_tab(state, &script, session_key, timeout).await?;
    let value = response.data.or(response.result).unwrap_or(Value::Null);
    if value.get("ok").and_then(Value::as_bool) == Some(false) {
        return Err(anyhow!(value
            .get("error")
            .and_then(Value::as_str)
            .unwrap_or("batch cdp failed")
            .to_string()));
    }
    Ok(value)
}

async fn resolve_target_object(
    state: &AppState,
    tab_id: &str,
    target: &str,
    timeout: Duration,
) -> Result<String> {
    if is_ref_target(target) {
        let entry = {
            let driver = state.driver.lock().await;
            let cache = driver.snapshots.get(tab_id).ok_or_else(|| {
                anyhow!("unknown ref {target} in tab {tab_id}; run snapshot --tab {tab_id} again")
            })?;
            cache.refs.get(target).cloned().ok_or_else(|| {
                anyhow!("unknown ref {target} in tab {tab_id}; run snapshot --tab {tab_id} again")
            })?
        };
        let current_url = current_location(state, tab_id, timeout)
            .await
            .unwrap_or_default();
        let snapshot_url = state
            .driver
            .lock()
            .await
            .snapshots
            .get(tab_id)
            .map(|cache| cache.url.clone())
            .unwrap_or_default();
        if !snapshot_url.is_empty() && !current_url.is_empty() && current_url != snapshot_url {
            return Err(anyhow!("ref expired: page url changed, run snapshot again"));
        }
        let object_id = format!("__backend:{}", entry.backend_dom_node_id);
        Ok(object_id)
    } else {
        Ok(format!("__selector:{}", target))
    }
}

async fn current_location(state: &AppState, tab_id: &str, timeout: Duration) -> Result<String> {
    let response = execute_raw_js_on_tab(state, "return location.href", tab_id, timeout).await?;
    Ok(response
        .data
        .unwrap_or(Value::Null)
        .as_str()
        .unwrap_or_default()
        .to_string())
}

fn is_ref_target(target: &str) -> bool {
    let Some(rest) = target.strip_prefix("@e") else {
        return false;
    };
    !rest.is_empty() && rest.chars().all(|c| c.is_ascii_digit())
}

async fn call_function_resolved(
    state: &AppState,
    tab_id: &str,
    object_id: &str,
    function_declaration: &str,
    timeout: Duration,
) -> Result<Value> {
    let value = if let Some(raw) = object_id.strip_prefix("__backend:") {
        let backend: i64 = raw.parse()?;
        batch_cdp_call(
            state,
            tab_id,
            vec![
                json!({ "cmd": "cdp", "method": "DOM.resolveNode", "params": { "backendNodeId": backend } }),
                json!({ "cmd": "cdp", "method": "Runtime.callFunctionOn", "params": { "objectId": "$0.object.objectId", "functionDeclaration": function_declaration, "returnByValue": true } }),
            ],
            timeout,
        )
        .await?
    } else if let Some(selector) = object_id.strip_prefix("__selector:") {
        let expression = format!(
            "document.querySelector({})",
            serde_json::to_string(selector)?
        );
        batch_cdp_call(
            state,
            tab_id,
            vec![
                json!({ "cmd": "cdp", "method": "Runtime.evaluate", "params": { "expression": expression, "returnByValue": false } }),
                json!({ "cmd": "cdp", "method": "Runtime.callFunctionOn", "params": { "objectId": "$0.result.objectId", "functionDeclaration": function_declaration, "returnByValue": true } }),
            ],
            timeout,
        )
        .await?
    } else {
        return call_function_value(state, tab_id, object_id, function_declaration, timeout).await;
    };
    value
        .get("results")
        .and_then(Value::as_array)
        .and_then(|items| items.get(1))
        .or_else(|| value.as_array().and_then(|items| items.get(1)))
        .and_then(|data| data.get("result").and_then(|r| r.get("value")).cloned())
        .ok_or_else(|| anyhow!("Runtime.callFunctionOn response missing value"))
}

async fn dom_click(
    state: &AppState,
    tab_id: &str,
    object_id: &str,
    timeout: Duration,
) -> Result<Value> {
    let script = r#"function() {
      this.scrollIntoView({ block: 'center', inline: 'center' });
      this.click();
      return { success: true, tag: this.tagName, text: (this.textContent || '').slice(0, 100) };
    }"#;
    call_function_resolved(state, tab_id, object_id, script, timeout).await
}

async fn dom_fill(
    state: &AppState,
    tab_id: &str,
    object_id: &str,
    payload: Value,
    timeout: Duration,
) -> Result<Value> {
    let value = payload
        .get("value")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let append = payload
        .get("append")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let script = format!(
        r#"function() {{
      const __value = {value_json};
      const __append = {append};
      const el = this;
      el.focus();
      const role = el.getAttribute && (el.getAttribute('role') || '').toLowerCase();
      const tag = (el.tagName || '').toLowerCase();
      const editable = el.isContentEditable || tag === 'input' || tag === 'textarea' || role === 'textbox' || role === 'searchbox';
      if (!editable) return {{ error: 'fill target is not editable' }};
      if (el.isContentEditable) {{
        if (!__append) el.textContent = '';
        const sel = window.getSelection();
        if (sel) {{ const range = document.createRange(); range.selectNodeContents(el); range.collapse(false); sel.removeAllRanges(); sel.addRange(range); }}
        let inserted = false;
        try {{ inserted = document.execCommand('insertText', false, __value); }} catch (_) {{ inserted = false; }}
        if (!inserted) {{ el.textContent = (__append ? el.textContent : '') + __value; el.dispatchEvent(new InputEvent('input', {{ inputType: 'insertText', data: __value, bubbles: true }})); }}
        el.dispatchEvent(new Event('change', {{ bubbles: true }}));
        return {{ success: true, tag: el.tagName, mode: 'contenteditable', value_length: __value.length }};
      }}
      if (tag === 'input' || tag === 'textarea') {{
        const setter = Object.getOwnPropertyDescriptor(tag === 'textarea' ? HTMLTextAreaElement.prototype : HTMLInputElement.prototype, 'value')?.set;
        const next = __append ? (el.value || '') + __value : __value;
        if (setter) setter.call(el, next); else el.value = next;
        el.dispatchEvent(new Event('input', {{ bubbles: true }}));
        el.dispatchEvent(new Event('change', {{ bubbles: true }}));
        return {{ success: true, tag: el.tagName, mode: 'value', value_length: __value.length }};
      }}
      try {{ document.execCommand('insertText', false, __value); }} catch (_) {{}}
      el.dispatchEvent(new Event('input', {{ bubbles: true }}));
      el.dispatchEvent(new Event('change', {{ bubbles: true }}));
      return {{ success: true, tag: el.tagName, mode: 'role-textbox', value_length: __value.length }};
    }}"#,
        value_json = serde_json::to_string(value).unwrap_or_else(|_| "\"\"".to_string()),
        append = append,
    );
    let value = call_function_resolved(state, tab_id, object_id, &script, timeout).await?;
    if let Some(error) = value.get("error").and_then(Value::as_str) {
        return Err(anyhow!(error.to_string()));
    }
    Ok(value)
}

async fn dom_mouse_click(
    state: &AppState,
    tab_id: &str,
    object_id: &str,
    timeout: Duration,
) -> Result<Value> {
    let box_data = if let Some(raw) = object_id.strip_prefix("__backend:") {
        let backend: i64 = raw.parse()?;
        batch_cdp_call(
            state,
            tab_id,
            vec![
                json!({ "cmd": "cdp", "method": "DOM.resolveNode", "params": { "backendNodeId": backend } }),
                json!({ "cmd": "cdp", "method": "Runtime.callFunctionOn", "params": { "objectId": "$0.object.objectId", "functionDeclaration": "function() { this.scrollIntoView({ block: 'center', inline: 'center' }); }", "returnByValue": true } }),
                json!({ "cmd": "cdp", "method": "Runtime.callFunctionOn", "params": { "objectId": "$0.object.objectId", "functionDeclaration": "function() { const r = this.getBoundingClientRect(); return { x: r.x, y: r.y, width: r.width, height: r.height }; }", "returnByValue": true } }),
            ],
            timeout,
        )
        .await?
    } else if let Some(selector) = object_id.strip_prefix("__selector:") {
        let expression = format!(
            "document.querySelector({})",
            serde_json::to_string(selector)?
        );
        batch_cdp_call(
            state,
            tab_id,
            vec![
                json!({ "cmd": "cdp", "method": "Runtime.evaluate", "params": { "expression": expression, "returnByValue": false } }),
                json!({ "cmd": "cdp", "method": "Runtime.callFunctionOn", "params": { "objectId": "$0.result.objectId", "functionDeclaration": "function() { this.scrollIntoView({ block: 'center', inline: 'center' }); }", "returnByValue": true } }),
                json!({ "cmd": "cdp", "method": "Runtime.callFunctionOn", "params": { "objectId": "$0.result.objectId", "functionDeclaration": "function() { const r = this.getBoundingClientRect(); return { x: r.x, y: r.y, width: r.width, height: r.height }; }", "returnByValue": true } }),
            ],
            timeout,
        )
        .await?
    } else {
        focus_scroll(state, tab_id, object_id, timeout).await?;
        let result = cdp_call(
            state,
            tab_id,
            "DOM.getBoxModel",
            json!({ "objectId": object_id }),
            timeout,
        )
        .await?;
        json!({ "results": [null, null, result.data.unwrap_or(Value::Null)] })
    };
    let rect = box_data
        .get("results")
        .and_then(Value::as_array)
        .and_then(|items| items.get(2))
        .and_then(|data| data.get("result").and_then(|r| r.get("value")).cloned())
        .or_else(|| {
            box_data
                .as_array()
                .and_then(|items| items.get(2))
                .and_then(|data| data.get("result").and_then(|r| r.get("value")).cloned())
        });
    let (x, y) = if let Some(rect) = rect {
        let width = rect
            .get("width")
            .and_then(Value::as_f64)
            .unwrap_or_default();
        let height = rect
            .get("height")
            .and_then(Value::as_f64)
            .unwrap_or_default();
        if width <= 0.0 || height <= 0.0 {
            return Err(anyhow!("mouse-click: element has no layout box (display:none / detached / zero-size). Use click for DOM-level fallback."));
        }
        (
            rect.get("x").and_then(Value::as_f64).unwrap_or_default() + width / 2.0,
            rect.get("y").and_then(Value::as_f64).unwrap_or_default() + height / 2.0,
        )
    } else {
        let content = box_data
            .get("results")
            .and_then(Value::as_array)
            .and_then(|items| items.get(2))
            .and_then(|data| data.get("model").and_then(|m| m.get("content")).cloned())
            .and_then(|v| v.as_array().cloned())
            .ok_or_else(|| anyhow!("mouse-click: element has no layout box (display:none / detached / zero-size). Use click for DOM-level fallback."))?;
        if content.len() < 8 {
            return Err(anyhow!("mouse-click: element has no layout box (display:none / detached / zero-size). Use click for DOM-level fallback."));
        }
        let n = |idx: usize| content.get(idx).and_then(Value::as_f64).unwrap_or_default();
        (
            (n(0) + n(2) + n(4) + n(6)) / 4.0,
            (n(1) + n(3) + n(5) + n(7)) / 4.0,
        )
    };
    for params in [
        json!({ "type": "mouseMoved", "x": x, "y": y, "button": "none", "buttons": 0 }),
        json!({ "type": "mousePressed", "x": x, "y": y, "button": "left", "buttons": 1, "clickCount": 1 }),
        json!({ "type": "mouseReleased", "x": x, "y": y, "button": "left", "buttons": 0, "clickCount": 1 }),
    ] {
        let result = cdp_call(state, tab_id, "Input.dispatchMouseEvent", params, timeout).await?;
        if !result.ok {
            return Err(anyhow!(result
                .error
                .unwrap_or_else(|| "mouse-click failed".to_string())));
        }
    }
    Ok(json!({ "success": true, "x": x.round(), "y": y.round() }))
}

async fn focus_object(
    state: &AppState,
    tab_id: &str,
    object_id: &str,
    timeout: Duration,
) -> Result<()> {
    let script = "function() { this.focus && this.focus(); return true; }";
    call_function_resolved(state, tab_id, object_id, script, timeout).await?;
    Ok(())
}

async fn focus_scroll(
    state: &AppState,
    tab_id: &str,
    object_id: &str,
    timeout: Duration,
) -> Result<()> {
    let script =
        "function() { this.scrollIntoView({ block: 'center', inline: 'center' }); return true; }";
    call_function_resolved(state, tab_id, object_id, script, timeout).await?;
    Ok(())
}

async fn call_function_value(
    state: &AppState,
    tab_id: &str,
    object_id: &str,
    function_declaration: &str,
    timeout: Duration,
) -> Result<Value> {
    let result = cdp_call(
        state,
        tab_id,
        "Runtime.callFunctionOn",
        json!({ "objectId": object_id, "functionDeclaration": function_declaration, "returnByValue": true }),
        timeout,
    )
    .await?;
    if !result.ok {
        return Err(anyhow!(result
            .error
            .unwrap_or_else(|| "Runtime.callFunctionOn failed".to_string())));
    }
    let data = result.data.unwrap_or(Value::Null);
    if let Some(details) = data.get("exceptionDetails") {
        return Err(anyhow!(details.to_string()));
    }
    Ok(data
        .get("result")
        .and_then(|r| r.get("value"))
        .cloned()
        .unwrap_or(Value::Null))
}

async fn dispatch_keys(
    state: &AppState,
    tab_id: &str,
    keys: &str,
    timeout: Duration,
) -> Result<Value> {
    let platform = if cfg!(target_os = "macos") {
        "mac"
    } else {
        "other"
    };
    let segments: Vec<&str> = keys.split_whitespace().collect();
    if segments.is_empty() {
        return Err(anyhow!("send-keys: keys is required"));
    }
    let mut dispatched = 0;
    for segment in segments {
        let key = parse_key_segment(segment, platform)?;
        dispatch_key_segment(state, tab_id, key, timeout).await?;
        dispatched += 1;
    }
    Ok(json!({ "success": true, "dispatched": dispatched }))
}

#[derive(Debug, Clone)]
struct KeySegment {
    modifiers: Vec<KeySpec>,
    modifier_bits: i64,
    key: KeySpec,
}

#[derive(Debug, Clone)]
struct KeySpec {
    bit: i64,
    key: String,
    code: String,
    vkc: i64,
    text: Option<String>,
}

fn parse_key_segment(segment: &str, platform: &str) -> Result<KeySegment> {
    let parts: Vec<&str> = segment
        .split('+')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect();
    if parts.is_empty() {
        return Err(anyhow!("send-keys: empty segment"));
    }
    let mut modifier_bits = 0;
    let mut modifiers = Vec::new();
    for part in &parts[..parts.len().saturating_sub(1)] {
        let spec = modifier_spec(part, platform)
            .ok_or_else(|| anyhow!("send-keys: {part} is not a modifier"))?;
        modifier_bits |= spec.bit;
        modifiers.push(spec);
    }
    let mut key = key_spec(parts[parts.len() - 1])?;
    if modifier_bits & 8 != 0
        && key.key.len() == 1
        && key.key.chars().all(|c| c.is_ascii_lowercase())
    {
        key.key = key.key.to_uppercase();
        key.text = key.text.as_ref().map(|_| key.key.clone());
    }
    Ok(KeySegment {
        modifiers,
        modifier_bits,
        key,
    })
}

fn modifier_spec(input: &str, platform: &str) -> Option<KeySpec> {
    let name = input.to_ascii_lowercase();
    let (bit, key, code, vkc) = match name.as_str() {
        "alt" => (1, "Alt", "AltLeft", 18),
        "ctrl" | "control" => (2, "Control", "ControlLeft", 17),
        "cmd" | "meta" => (4, "Meta", "MetaLeft", 91),
        "shift" => (8, "Shift", "ShiftLeft", 16),
        "mod" if platform == "mac" => (4, "Meta", "MetaLeft", 91),
        "mod" => (2, "Control", "ControlLeft", 17),
        _ => return None,
    };
    Some(KeySpec {
        bit,
        key: key.to_string(),
        code: code.to_string(),
        vkc,
        text: None,
    })
}

fn key_spec(input: &str) -> Result<KeySpec> {
    let name = input.to_ascii_lowercase();
    let (key, code, vkc, text) = match name.as_str() {
        "enter" | "return" => (
            "Enter".to_string(),
            "Enter".to_string(),
            13,
            Some("\r".to_string()),
        ),
        "escape" | "esc" => ("Escape".to_string(), "Escape".to_string(), 27, None),
        "tab" => ("Tab".to_string(), "Tab".to_string(), 9, None),
        "backspace" => ("Backspace".to_string(), "Backspace".to_string(), 8, None),
        "delete" => ("Delete".to_string(), "Delete".to_string(), 46, None),
        "space" => (
            " ".to_string(),
            "Space".to_string(),
            32,
            Some(" ".to_string()),
        ),
        "arrowup" | "up" => ("ArrowUp".to_string(), "ArrowUp".to_string(), 38, None),
        "arrowdown" | "down" => ("ArrowDown".to_string(), "ArrowDown".to_string(), 40, None),
        "arrowleft" | "left" => ("ArrowLeft".to_string(), "ArrowLeft".to_string(), 37, None),
        "arrowright" | "right" => ("ArrowRight".to_string(), "ArrowRight".to_string(), 39, None),
        "home" => ("Home".to_string(), "Home".to_string(), 36, None),
        "end" => ("End".to_string(), "End".to_string(), 35, None),
        _ if input.len() == 1 && input.chars().all(|c| c.is_ascii_alphabetic()) => {
            let lower = input.to_ascii_lowercase();
            let upper = input.to_ascii_uppercase();
            (
                lower.clone(),
                format!("Key{upper}"),
                upper.as_bytes()[0] as i64,
                Some(lower),
            )
        }
        _ if input.len() == 1 && input.chars().all(|c| c.is_ascii_digit()) => (
            input.to_string(),
            format!("Digit{input}"),
            input.as_bytes()[0] as i64,
            Some(input.to_string()),
        ),
        _ => return Err(anyhow!("send-keys: unknown key {input}")),
    };
    Ok(KeySpec {
        bit: 0,
        key,
        code,
        vkc,
        text,
    })
}

async fn dispatch_key_segment(
    state: &AppState,
    tab_id: &str,
    segment: KeySegment,
    timeout: Duration,
) -> Result<()> {
    let mut active_bits = 0;
    for modifier in &segment.modifiers {
        active_bits |= modifier.bit;
        dispatch_key_event(
            state,
            tab_id,
            "keyDown",
            active_bits,
            modifier,
            None,
            timeout,
        )
        .await?;
    }
    let text = if segment.modifier_bits & !8 == 0 {
        segment.key.text.as_deref()
    } else {
        None
    };
    dispatch_key_event(
        state,
        tab_id,
        "keyDown",
        segment.modifier_bits,
        &segment.key,
        text,
        timeout,
    )
    .await?;
    dispatch_key_event(
        state,
        tab_id,
        "keyUp",
        segment.modifier_bits,
        &segment.key,
        None,
        timeout,
    )
    .await?;
    for modifier in segment.modifiers.iter().rev() {
        active_bits &= !modifier.bit;
        dispatch_key_event(state, tab_id, "keyUp", active_bits, modifier, None, timeout).await?;
    }
    Ok(())
}

async fn dispatch_key_event(
    state: &AppState,
    tab_id: &str,
    event_type: &str,
    modifiers: i64,
    spec: &KeySpec,
    text: Option<&str>,
    timeout: Duration,
) -> Result<()> {
    let mut params = json!({ "type": event_type, "modifiers": modifiers, "key": spec.key, "code": spec.code, "windowsVirtualKeyCode": spec.vkc });
    if let Some(text) = text {
        params["text"] = json!(text);
    }
    let result = cdp_call(state, tab_id, "Input.dispatchKeyEvent", params, timeout).await?;
    if !result.ok {
        return Err(anyhow!(result
            .error
            .unwrap_or_else(|| "send-keys failed".to_string())));
    }
    Ok(())
}

fn ax_string(value: &Option<AxValue>) -> Option<String> {
    value.as_ref().and_then(|v| {
        let value = v.value.as_ref()?;
        let is_string_like = matches!(
            v.value_type.as_deref(),
            Some("string")
                | Some("computedString")
                | Some("token")
                | Some("internalRole")
                | Some("role")
        );
        Some(if is_string_like {
            value.as_str().unwrap_or_default().to_string()
        } else if let Some(s) = value.as_str() {
            s.to_string()
        } else if let Some(n) = value.as_i64() {
            n.to_string()
        } else if let Some(n) = value.as_u64() {
            n.to_string()
        } else if let Some(n) = value.as_f64() {
            n.to_string()
        } else if let Some(b) = value.as_bool() {
            b.to_string()
        } else {
            value.to_string().trim_matches('"').to_string()
        })
    })
}

fn non_empty(value: Option<String>) -> Option<String> {
    value.filter(|value| !value.trim().is_empty())
}

fn truncate_text(value: &str, max: usize) -> String {
    if value.chars().count() <= max {
        value.to_string()
    } else {
        format!("{}...", value.chars().take(max).collect::<String>())
    }
}

fn is_operable_role(role: &str) -> bool {
    matches!(
        role,
        "button"
            | "link"
            | "textbox"
            | "searchbox"
            | "checkbox"
            | "radio"
            | "combobox"
            | "listbox"
            | "menuitem"
            | "menuitemcheckbox"
            | "menuitemradio"
            | "option"
            | "slider"
            | "spinbutton"
            | "switch"
            | "tab"
            | "treeitem"
    )
}

fn role_is_context(role: &str) -> bool {
    matches!(
        role,
        "form" | "dialog" | "alert" | "heading" | "main" | "navigation" | "region"
    )
}

async fn extension_control(
    state: &AppState,
    session_key: &str,
    command: &str,
    payload: Value,
) -> Result<Value> {
    let mut body = serde_json::Map::new();
    body.insert("cmd".to_string(), json!(command));
    let tab_id = session_tab_id(state, session_key).await?;
    body.insert(
        "tabId".to_string(),
        json!(tab_id.parse::<i64>().unwrap_or_default()),
    );
    if let Value::Object(map) = payload {
        for (key, value) in map {
            body.insert(key, value);
        }
    }
    let response = execute_raw_js_on_tab(
        state,
        &Value::Object(body).to_string(),
        session_key,
        Duration::from_secs(30),
    )
    .await?;
    let value = response.data.or(response.result).unwrap_or(Value::Null);
    if value.get("ok").and_then(Value::as_bool) == Some(false) {
        return Err(anyhow!(value
            .get("error")
            .and_then(Value::as_str)
            .unwrap_or("extension command failed")
            .to_string()));
    }
    Ok(value)
}

async fn network_start(State(state): State<AppState>, Json(req): Json<TabRequest>) -> Json<Value> {
    touch(&state).await;
    let result = async {
        let tab_id = select_tab(
            &state,
            SessionSelector::new(req.switch_tab_id, req.browser, req.profile),
        )
        .await?;
        extension_control(&state, &tab_id, "networkStart", json!({})).await
    }
    .await;
    Json(match result {
        Ok(value) => json!({ "ok": true, "result": value }),
        Err(err) => json!({ "ok": false, "error": err.to_string() }),
    })
}

async fn network_list(
    State(state): State<AppState>,
    Json(req): Json<NetworkListRequest>,
) -> Json<Value> {
    touch(&state).await;
    let result = async {
        let tab_id = select_tab(
            &state,
            SessionSelector::new(req.switch_tab_id, req.browser, req.profile),
        )
        .await?;
        extension_control(
            &state,
            &tab_id,
            "networkList",
            json!({ "filter": req.filter, "limit": req.limit }),
        )
        .await
    }
    .await;
    Json(match result {
        Ok(value) => json!({ "ok": true, "result": value }),
        Err(err) => json!({ "ok": false, "error": err.to_string() }),
    })
}

async fn network_detail(
    State(state): State<AppState>,
    Json(req): Json<NetworkDetailRequest>,
) -> Json<Value> {
    touch(&state).await;
    let result = async {
        let tab_id = select_tab(
            &state,
            SessionSelector::new(req.switch_tab_id, req.browser, req.profile),
        )
        .await?;
        extension_control(
            &state,
            &tab_id,
            "networkDetail",
            json!({ "requestId": req.request_id }),
        )
        .await
    }
    .await;
    Json(match result {
        Ok(value) => json!({ "ok": true, "result": value }),
        Err(err) => json!({ "ok": false, "error": err.to_string() }),
    })
}

async fn network_clear(State(state): State<AppState>, Json(req): Json<TabRequest>) -> Json<Value> {
    touch(&state).await;
    let result = async {
        let tab_id = select_tab(
            &state,
            SessionSelector::new(req.switch_tab_id, req.browser, req.profile),
        )
        .await?;
        extension_control(&state, &tab_id, "networkClear", json!({})).await
    }
    .await;
    Json(match result {
        Ok(value) => json!({ "ok": true, "result": value }),
        Err(err) => json!({ "ok": false, "error": err.to_string() }),
    })
}

async fn network_stop(State(state): State<AppState>, Json(req): Json<TabRequest>) -> Json<Value> {
    touch(&state).await;
    let result = async {
        let tab_id = select_tab(
            &state,
            SessionSelector::new(req.switch_tab_id, req.browser, req.profile),
        )
        .await?;
        extension_control(&state, &tab_id, "networkStop", json!({})).await
    }
    .await;
    Json(match result {
        Ok(value) => json!({ "ok": true, "result": value }),
        Err(err) => json!({ "ok": false, "error": err.to_string() }),
    })
}

async fn console_start(State(state): State<AppState>, Json(req): Json<TabRequest>) -> Json<Value> {
    touch(&state).await;
    let result = async {
        let tab_id = select_tab(
            &state,
            SessionSelector::new(req.switch_tab_id, req.browser, req.profile),
        )
        .await?;
        extension_control(&state, &tab_id, "consoleStart", json!({})).await
    }
    .await;
    Json(match result {
        Ok(value) => json!({ "ok": true, "result": value }),
        Err(err) => json!({ "ok": false, "error": err.to_string() }),
    })
}

async fn console_list(
    State(state): State<AppState>,
    Json(req): Json<ConsoleListRequest>,
) -> Json<Value> {
    touch(&state).await;
    let result = async {
        let tab_id = select_tab(
            &state,
            SessionSelector::new(req.switch_tab_id, req.browser, req.profile),
        )
        .await?;
        extension_control(
            &state,
            &tab_id,
            "consoleList",
            json!({ "level": req.level, "limit": req.limit }),
        )
        .await
    }
    .await;
    Json(match result {
        Ok(value) => json!({ "ok": true, "result": value }),
        Err(err) => json!({ "ok": false, "error": err.to_string() }),
    })
}

async fn console_clear(State(state): State<AppState>, Json(req): Json<TabRequest>) -> Json<Value> {
    touch(&state).await;
    let result = async {
        let tab_id = select_tab(
            &state,
            SessionSelector::new(req.switch_tab_id, req.browser, req.profile),
        )
        .await?;
        extension_control(&state, &tab_id, "consoleClear", json!({})).await
    }
    .await;
    Json(match result {
        Ok(value) => json!({ "ok": true, "result": value }),
        Err(err) => json!({ "ok": false, "error": err.to_string() }),
    })
}

async fn console_stop(State(state): State<AppState>, Json(req): Json<TabRequest>) -> Json<Value> {
    touch(&state).await;
    let result = async {
        let tab_id = select_tab(
            &state,
            SessionSelector::new(req.switch_tab_id, req.browser, req.profile),
        )
        .await?;
        extension_control(&state, &tab_id, "consoleStop", json!({})).await
    }
    .await;
    Json(match result {
        Ok(value) => json!({ "ok": true, "result": value }),
        Err(err) => json!({ "ok": false, "error": err.to_string() }),
    })
}

async fn screenshot(
    State(state): State<AppState>,
    Json(req): Json<ScreenshotRequest>,
) -> Json<Value> {
    touch(&state).await;
    Json(match screenshot_page(&state, req).await {
        Ok(value) => json!({ "ok": true, "result": value }),
        Err(err) => json!({ "ok": false, "error": err.to_string() }),
    })
}

async fn save_pdf(State(state): State<AppState>, Json(req): Json<SavePdfRequest>) -> Json<Value> {
    touch(&state).await;
    Json(match save_pdf_page(&state, req).await {
        Ok(value) => json!({ "ok": true, "result": value }),
        Err(err) => json!({ "ok": false, "error": err.to_string() }),
    })
}

async fn screenshot_page(state: &AppState, req: ScreenshotRequest) -> Result<Value> {
    let tab_id = select_tab(
        state,
        SessionSelector::new(req.switch_tab_id, req.browser, req.profile),
    )
    .await?;
    let timeout = Duration::from_secs_f64(req.timeout.max(0.1));
    let format = normalize_screenshot_format(&req.format)?;
    let target = match (req.target, req.selector) {
        (Some(_), Some(_)) => {
            return Err(anyhow!("screenshot: --target 不能和 --selector 同时使用"))
        }
        (Some(target), None) | (None, Some(target)) => Some(target),
        (None, None) => None,
    };
    let mut params = json!({ "format": format });
    if format == "jpeg" {
        let quality = req.quality.unwrap_or(80);
        if !(1..=100).contains(&quality) {
            return Err(anyhow!("screenshot: quality 必须在 1..=100"));
        }
        params["quality"] = json!(quality);
    }
    if let Some(target) = target.as_deref() {
        let rect = target_rect(state, &tab_id, target, timeout).await?;
        params["clip"] = json!({
            "x": rect.x,
            "y": rect.y,
            "width": rect.width,
            "height": rect.height,
            "scale": 1.0
        });
    } else if req.full_page {
        let metrics = cdp_call(state, &tab_id, "Page.getLayoutMetrics", json!({}), timeout).await?;
        if !metrics.ok {
            return Err(anyhow!(metrics.error.unwrap_or_else(|| {
                "screenshot: Page.getLayoutMetrics failed".to_string()
            })));
        }
        let content_size = metrics
            .data
            .and_then(|data| data.get("contentSize").cloned())
            .ok_or_else(|| anyhow!("screenshot: missing contentSize"))?;
        params["clip"] = json!({
            "x": content_size.get("x").and_then(Value::as_f64).unwrap_or(0.0),
            "y": content_size.get("y").and_then(Value::as_f64).unwrap_or(0.0),
            "width": content_size.get("width").and_then(Value::as_f64).unwrap_or(0.0),
            "height": content_size.get("height").and_then(Value::as_f64).unwrap_or(0.0),
            "scale": 1.0
        });
        params["captureBeyondViewport"] = json!(true);
    }
    let captured = cdp_call(state, &tab_id, "Page.captureScreenshot", params, timeout).await?;
    if !captured.ok {
        return Err(anyhow!(captured.error.unwrap_or_else(|| {
            "screenshot: Page.captureScreenshot failed".to_string()
        })));
    }
    let data = captured
        .data
        .and_then(|data| data.get("data").and_then(Value::as_str).map(str::to_string))
        .ok_or_else(|| anyhow!("screenshot: CDP response missing data"))?;
    let bytes = decode_base64(&data)?;
    let path = req.out.unwrap_or_else(|| default_screenshot_path(format));
    write_bytes(&path, &bytes)?;
    Ok(json!({
        "status": "success",
        "path": path,
        "format": format,
        "bytes": bytes.len(),
        "target": target,
        "full_page": req.full_page
    }))
}

async fn save_pdf_page(state: &AppState, req: SavePdfRequest) -> Result<Value> {
    let tab_id = select_tab(
        state,
        SessionSelector::new(req.switch_tab_id, req.browser, req.profile),
    )
    .await?;
    let timeout = Duration::from_secs_f64(req.timeout.max(0.1));
    let (paper_width, paper_height) = paper_size(&req.paper)?;
    if !(0.1..=2.0).contains(&req.scale) {
        return Err(anyhow!("save-pdf: scale 必须在 0.1..=2.0"));
    }
    let title = current_title(state, &tab_id, timeout)
        .await
        .unwrap_or_else(|_| "page".to_string());
    let result = cdp_call(
        state,
        &tab_id,
        "Page.printToPDF",
        json!({
            "printBackground": req.print_background,
            "landscape": req.landscape,
            "scale": req.scale,
            "paperWidth": paper_width,
            "paperHeight": paper_height,
            "preferCSSPageSize": true
        }),
        timeout,
    )
    .await?;
    if !result.ok {
        return Err(anyhow!(result
            .error
            .unwrap_or_else(|| "save-pdf: Page.printToPDF failed".to_string())));
    }
    let data = result
        .data
        .and_then(|data| data.get("data").and_then(Value::as_str).map(str::to_string))
        .ok_or_else(|| anyhow!("save-pdf: CDP response missing data"))?;
    let bytes = decode_base64(&data)?;
    const MAX_PDF_BYTES: usize = 50 * 1024 * 1024;
    if bytes.len() > MAX_PDF_BYTES {
        return Err(anyhow!("save-pdf: PDF exceeds max size 50MB"));
    }
    let path = req.out.unwrap_or_else(|| default_pdf_path(&title));
    write_bytes(&path, &bytes)?;
    Ok(json!({
        "status": "success",
        "path": path,
        "bytes": bytes.len(),
        "paper": req.paper.to_ascii_lowercase(),
        "landscape": req.landscape,
        "scale": req.scale,
        "print_background": req.print_background
    }))
}

async fn target_rect(
    state: &AppState,
    tab_id: &str,
    target: &str,
    timeout: Duration,
) -> Result<RectInfo> {
    let object_id = resolve_target_object(state, tab_id, target, timeout).await?;
    let script = "function() { this.scrollIntoView({ block: 'center', inline: 'center' }); const r = this.getBoundingClientRect(); return { x: r.x, y: r.y, width: r.width, height: r.height }; }";
    let value = call_function_resolved(state, tab_id, &object_id, script, timeout).await?;
    let rect: RectInfo = serde_json::from_value(value)?;
    if rect.width <= 0.0 || rect.height <= 0.0 {
        return Err(anyhow!("screenshot: target has no layout box"));
    }
    Ok(rect)
}

async fn current_title(state: &AppState, tab_id: &str, timeout: Duration) -> Result<String> {
    let response = execute_raw_js_on_tab(state, "return document.title", tab_id, timeout).await?;
    Ok(response
        .data
        .unwrap_or(Value::Null)
        .as_str()
        .unwrap_or("page")
        .to_string())
}

fn normalize_screenshot_format(format: &str) -> Result<&'static str> {
    match format.to_ascii_lowercase().as_str() {
        "png" => Ok("png"),
        "jpg" | "jpeg" => Ok("jpeg"),
        other => Err(anyhow!(
            "screenshot: unsupported format {other}, expected png or jpeg"
        )),
    }
}

fn paper_size(paper: &str) -> Result<(f64, f64)> {
    match paper.to_ascii_lowercase().as_str() {
        "letter" => Ok((8.5, 11.0)),
        "legal" => Ok((8.5, 14.0)),
        "a4" => Ok((8.27, 11.69)),
        "a3" => Ok((11.69, 16.54)),
        "tabloid" => Ok((11.0, 17.0)),
        other => Err(anyhow!("save-pdf: unsupported paper {other}")),
    }
}

fn default_screenshot_path(format: &str) -> PathBuf {
    PathBuf::from("/tmp/agent-browser-cli-screenshots").join(format!(
        "screenshot-{}.{}",
        timestamp_compact(),
        if format == "jpeg" { "jpg" } else { format }
    ))
}

fn default_pdf_path(title: &str) -> PathBuf {
    PathBuf::from("/tmp/agent-browser-cli-pdfs").join(format!(
        "{}-{}.pdf",
        sanitize_filename(title),
        timestamp_compact()
    ))
}

fn write_bytes(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, bytes)?;
    Ok(())
}

fn timestamp_compact() -> String {
    let secs = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or_default();
    secs.to_string()
}

fn sanitize_filename(input: &str) -> String {
    let mut out = String::new();
    for ch in input.chars() {
        let mapped = if matches!(ch, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|')
            || ch.is_control()
            || ch.is_whitespace()
        {
            '-'
        } else {
            ch
        };
        if mapped == '-' && out.ends_with('-') {
            continue;
        }
        out.push(mapped);
        if out.chars().count() >= 80 {
            break;
        }
    }
    let trimmed = out.trim_matches('-').to_string();
    if trimmed.is_empty() {
        "page".to_string()
    } else {
        trimmed
    }
}

fn decode_base64(input: &str) -> Result<Vec<u8>> {
    let mut out = Vec::with_capacity(input.len() * 3 / 4);
    let mut buf: u32 = 0;
    let mut bits = 0;
    for byte in input.bytes().filter(|b| !b.is_ascii_whitespace()) {
        if byte == b'=' {
            break;
        }
        let value = match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => return Err(anyhow!("invalid base64 data")),
        } as u32;
        buf = (buf << 6) | value;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push(((buf >> bits) & 0xff) as u8);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    fn test_state() -> AppState {
        let (shutdown, _shutdown_rx) = mpsc::unbounded_channel();
        AppState {
            driver: Arc::new(Mutex::new(DriverState::default())),
            started_at: SystemTime::now(),
            last_activity: Arc::new(Mutex::new(Instant::now())),
            shutdown,
            sessions_ready: Arc::new(Notify::new()),
            extension_port: config::DEFAULT_EXTENSION_PORT,
            api_token: "test-token".to_string(),
        }
    }

    fn insert_test_session(
        driver: &mut DriverState,
        browser_id: &str,
        profile_id: &str,
        tab_id: &str,
    ) -> mpsc::UnboundedReceiver<String> {
        let session_key = crate::protocol::make_session_key(browser_id, profile_id, tab_id);
        let (sender, receiver) = mpsc::unbounded_channel();
        driver.sessions.insert(
            session_key.clone(),
            Session {
                session_key: session_key.clone(),
                tab_id: tab_id.to_string(),
                browser_id: browser_id.to_string(),
                profile_id: profile_id.to_string(),
                profile_label: None,
                extension_version: Some("2.1".to_string()),
                file_scheme_access: Some(true),
                info: TabInfo {
                    id: tab_id.to_string(),
                    tab_id: tab_id.to_string(),
                    browser_id: browser_id.to_string(),
                    profile_id: profile_id.to_string(),
                    profile_label: None,
                    session_key,
                    url: "https://example.com".to_string(),
                    title: "test".to_string(),
                    tab_type: "ext_ws".to_string(),
                    connected_at: None,
                },
                sender,
                disconnected_at: None,
            },
        );
        receiver
    }

    #[test]
    fn api_access_requires_token_and_rejects_foreign_origins() {
        let mut headers = HeaderMap::new();
        headers.insert(
            config::API_TOKEN_HEADER,
            HeaderValue::from_static("test-token"),
        );
        assert!(validate_api_access(&headers, "test-token").is_ok());

        headers.insert(ORIGIN, HeaderValue::from_static("https://evil.example"));
        let error = validate_api_access(&headers, "test-token").unwrap_err();
        assert_eq!(error.0, StatusCode::FORBIDDEN);
        assert_eq!(error.1, "origin_not_allowed");

        headers.insert(ORIGIN, HeaderValue::from_static(LOOPBACK_API_ORIGIN));
        assert!(validate_api_access(&headers, "test-token").is_ok());
        headers.remove(config::API_TOKEN_HEADER);
        let error = validate_api_access(&headers, "test-token").unwrap_err();
        assert_eq!(error.0, StatusCode::UNAUTHORIZED);
        assert_eq!(error.1, "invalid_api_token");
    }

    #[test]
    fn websocket_origin_requires_a_valid_chrome_extension_id() {
        let mut headers = HeaderMap::new();
        assert!(!is_allowed_extension_origin(&headers));

        headers.insert(ORIGIN, HeaderValue::from_static("https://evil.example"));
        assert!(!is_allowed_extension_origin(&headers));

        headers.insert(
            ORIGIN,
            HeaderValue::from_static("chrome-extension://abcdefghijklmnopabcdefghijklmnop"),
        );
        assert!(is_allowed_extension_origin(&headers));

        headers.insert(
            ORIGIN,
            HeaderValue::from_static(
                "chrome-extension://abcdefghijklmnopabcdefghijklmnop.example.com",
            ),
        );
        assert!(!is_allowed_extension_origin(&headers));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn websocket_router_rejects_missing_and_non_extension_origins() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            axum::serve(listener, build_ws_router(test_state()))
                .await
                .unwrap();
        });

        let status_lines = tokio::task::spawn_blocking(move || {
            use std::io::{BufRead, BufReader, Write};

            let handshake = |origin: Option<&str>| {
                let mut stream = std::net::TcpStream::connect(addr).unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                let origin_header = origin
                    .map(|value| format!("Origin: {value}\r\n"))
                    .unwrap_or_default();
                let request = format!(
                    "GET / HTTP/1.1\r\n\
                     Host: {addr}\r\n\
                     Upgrade: websocket\r\n\
                     Connection: Upgrade\r\n\
                     Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\
                     Sec-WebSocket-Version: 13\r\n\
                     {origin_header}\r\n"
                );
                stream.write_all(request.as_bytes()).unwrap();
                stream.flush().unwrap();
                let mut status_line = String::new();
                BufReader::new(stream).read_line(&mut status_line).unwrap();
                status_line
            };

            (
                handshake(None),
                handshake(Some("https://evil.example")),
                handshake(Some("chrome-extension://abcdefghijklmnopabcdefghijklmnop")),
            )
        })
        .await
        .unwrap();
        server.abort();
        let _ = server.await;

        assert!(status_lines.0.starts_with("HTTP/1.1 403 Forbidden"));
        assert!(status_lines.1.starts_with("HTTP/1.1 403 Forbidden"));
        assert!(status_lines
            .2
            .starts_with("HTTP/1.1 101 Switching Protocols"));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn api_router_enforces_token_and_origin_checks() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            axum::serve(listener, build_api_router(test_state()))
                .await
                .unwrap();
        });

        let responses = tokio::task::spawn_blocking(move || {
            let client = reqwest::blocking::Client::new();
            let url = format!("http://{addr}/health");
            let send = |token: Option<&str>, origin: Option<&str>| {
                let mut request = client.get(&url);
                if let Some(token) = token {
                    request = request.header(config::API_TOKEN_HEADER, token);
                }
                if let Some(origin) = origin {
                    request = request.header("origin", origin);
                }
                let response = request.send().unwrap();
                let status = response.status();
                let body = response.json::<Value>().unwrap();
                (status, body)
            };
            (
                send(None, None),
                send(Some("wrong-token"), None),
                send(Some("test-token"), Some("https://evil.example")),
                send(Some("test-token"), None),
            )
        })
        .await
        .unwrap();
        server.abort();
        let _ = server.await;

        assert_eq!(responses.0 .0, StatusCode::UNAUTHORIZED);
        assert_eq!(responses.0 .1["error_code"], "invalid_api_token");
        assert_eq!(responses.1 .0, StatusCode::UNAUTHORIZED);
        assert_eq!(responses.1 .1["error_code"], "invalid_api_token");
        assert_eq!(responses.2 .0, StatusCode::FORBIDDEN);
        assert_eq!(responses.2 .1["error_code"], "origin_not_allowed");
        assert_eq!(responses.3 .0, StatusCode::OK);
        assert_eq!(responses.3 .1["running"], true);
    }

    #[tokio::test]
    async fn open_result_uses_executor_scope_when_browser_tab_ids_overlap() {
        let state = test_state();
        {
            let mut driver = state.driver.lock().await;
            let _receiver_a = insert_test_session(&mut driver, "browser-a", "profile-a", "7");
            let _receiver_b = insert_test_session(&mut driver, "browser-b", "profile-b", "42");
        }

        let result = normalize_open_result(
            &state,
            json!({
                "js_return": { "id": 42 },
                "tab_id": "7",
                "session_key": "browser-a:profile-a:7"
            }),
        )
        .await;
        assert_eq!(
            result.get("opened_session_key").and_then(Value::as_str),
            Some("browser-a:profile-a:42")
        );
    }

    #[tokio::test]
    async fn open_result_refuses_ambiguous_tab_id_without_executor_scope() {
        let state = test_state();
        {
            let mut driver = state.driver.lock().await;
            let _receiver_a = insert_test_session(&mut driver, "browser-a", "profile-a", "42");
            let _receiver_b = insert_test_session(&mut driver, "browser-b", "profile-b", "42");
        }

        let result = normalize_open_result(&state, json!({ "js_return": { "id": 42 } })).await;
        assert!(result.get("opened_session_key").is_some_and(Value::is_null));
    }

    #[tokio::test]
    async fn page_execution_reports_the_session_that_received_the_command() {
        let state = test_state();
        let (mut receiver_a, _receiver_b) = {
            let mut driver = state.driver.lock().await;
            let receiver_a = insert_test_session(&mut driver, "browser-a", "profile-a", "7");
            let receiver_b = insert_test_session(&mut driver, "browser-b", "profile-b", "8");
            driver.default_session_key = Some("browser-a:profile-a:7".to_string());
            (receiver_a, receiver_b)
        };

        let exec_state = state.clone();
        let execution = tokio::spawn(async move {
            execute_page_js(
                &exec_state,
                "return 1",
                SessionSelector::new(Some("browser-a:profile-a:7".to_string()), None, None),
                true,
            )
            .await
        });
        let payload = tokio::time::timeout(Duration::from_secs(1), receiver_a.recv())
            .await
            .expect("command should be dispatched")
            .expect("browser-a receiver should remain connected");
        let exec_id = serde_json::from_str::<Value>(&payload)
            .unwrap()
            .get("id")
            .and_then(Value::as_str)
            .unwrap()
            .to_string();

        state.driver.lock().await.default_session_key = Some("browser-b:profile-b:8".to_string());
        let (sender, _receiver) = mpsc::unbounded_channel();
        handle_ws_message(
            WsIncoming::Result {
                id: exec_id,
                result: json!(1),
                new_tabs: None,
            },
            &state,
            sender,
            &mut Vec::new(),
        )
        .await;

        let result = execution.await.unwrap().unwrap();
        assert_eq!(
            result.get("session_key").and_then(Value::as_str),
            Some("browser-a:profile-a:7")
        );
        assert_eq!(result.get("tab_id").and_then(Value::as_str), Some("7"));
    }

    #[test]
    fn pending_default_session_does_not_match_another_browser_tab() {
        let mut driver = DriverState {
            preferred_default_session_key: Some("browser-a:profile-a:42".to_string()),
            ..DriverState::default()
        };
        assert!(!should_set_default_session(
            &driver,
            "browser-b:profile-b:42"
        ));
        assert!(should_set_default_session(
            &driver,
            "browser-a:profile-a:42"
        ));
        driver.default_session_key = Some("browser-a:profile-a:7".to_string());
        assert!(should_set_default_session(
            &driver,
            "browser-a:profile-a:42"
        ));
    }

    #[test]
    fn request_target_accepts_compatible_fields() {
        assert_eq!(
            resolve_request_target(Some("./demo.html"), None).unwrap(),
            "./demo.html"
        );
        assert_eq!(
            resolve_request_target(None, Some("https://example.com")).unwrap(),
            "https://example.com"
        );
        assert_eq!(
            resolve_request_target(Some("same"), Some("same")).unwrap(),
            "same"
        );
    }

    #[test]
    fn request_target_rejects_missing_and_conflicting_fields() {
        assert_eq!(
            resolve_request_target(None, None).unwrap_err().code,
            "missing_open_target"
        );
        assert_eq!(
            resolve_request_target(Some("a"), Some("b"))
                .unwrap_err()
                .code,
            "ambiguous_open_target"
        );
    }

    #[test]
    fn extension_versions_compare_numerically() {
        assert!(version_at_least("2.1", "2.1"));
        assert!(version_at_least("2.10", "2.1"));
        assert!(version_at_least("2.1.1", "2.1"));
        assert!(!version_at_least("2.0", "2.1"));
        assert!(!version_at_least("unknown", "2.1"));
    }
}
