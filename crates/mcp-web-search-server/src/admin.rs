use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::{Request, StatusCode};
use axum::middleware::Next;
use axum::response::{Html, IntoResponse, Response};
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;
use uuid::Uuid;

use crate::auth::OAuthState;

// --- Constants ---

const MAX_LOG_ENTRIES: usize = 1000;
const ADMIN_SESSION_TTL: Duration = Duration::from_secs(86400); // 24 hours
/// Max MCP request body we'll buffer for logging. Large enough for typical
/// `interact` payloads; oversize requests get 413 rather than being silently
/// truncated to an empty body (which would corrupt JSON-RPC and kill the session).
const MAX_LOGGED_BODY_BYTES: usize = 1024 * 1024;

// --- State ---

pub struct AdminState {
    request_log: RwLock<VecDeque<RequestLogEntry>>,
    admin_sessions: RwLock<HashMap<String, Instant>>,
    start_time: Instant,
    next_log_id: AtomicU64,
}

impl AdminState {
    pub fn new() -> Self {
        Self {
            request_log: RwLock::new(VecDeque::new()),
            admin_sessions: RwLock::new(HashMap::new()),
            start_time: Instant::now(),
            next_log_id: AtomicU64::new(1),
        }
    }

    /// Prune expired admin sessions. Returns the number evicted.
    pub(crate) async fn evict_expired_sessions(&self) -> usize {
        let mut sessions = self.admin_sessions.write().await;
        let before = sessions.len();
        sessions.retain(|_, created_at| created_at.elapsed() < ADMIN_SESSION_TTL);
        before - sessions.len()
    }
}

#[derive(Serialize, Clone)]
pub struct RequestLogEntry {
    id: u64,
    timestamp_epoch_ms: u64,
    method: String,
    tool_name: Option<String>,
    params_summary: Option<String>,
    duration_ms: u64,
    success: bool,
    token_prefix: Option<String>,
}

// --- Combined state for admin handlers ---

type AdminAppState = Arc<(Arc<OAuthState>, Arc<AdminState>)>;

// --- Router ---

pub fn router(oauth: Arc<OAuthState>, admin: Arc<AdminState>) -> Router {
    let combined: AdminAppState = Arc::new((oauth, admin));

    // Login does not require admin auth
    let public_api = Router::new()
        .route("/login", post(admin_login))
        .with_state(combined.clone());

    // Everything else requires admin cookie
    let protected_api = Router::new()
        .route("/logout", post(admin_logout))
        .route("/dashboard", get(dashboard_handler))
        .route("/logs", get(logs_handler))
        .route("/clients", get(clients_handler))
        .route("/clients/{client_id}", delete(delete_client_handler))
        .route("/tokens", get(tokens_handler).post(create_token_handler))
        .route("/tokens/{token_prefix}", delete(delete_token_handler))
        .layer(axum::middleware::from_fn_with_state(
            combined.clone(),
            admin_auth_middleware,
        ))
        .with_state(combined);

    Router::new()
        .route("/admin", get(admin_spa))
        .nest("/admin/api", public_api.merge(protected_api))
}

// --- SPA ---

const ADMIN_HTML: &str = include_str!("../static/admin.html");

async fn admin_spa() -> Html<&'static str> {
    Html(ADMIN_HTML)
}

// --- Logging Middleware ---

pub async fn logging_middleware(
    State(state): State<Arc<AdminState>>,
    request: Request<Body>,
    next: Next,
) -> Response {
    let token_prefix = request
        .headers()
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|h| h.strip_prefix("Bearer "))
        .map(|t| t[..8.min(t.len())].to_string());

    let (parts, body) = request.into_parts();
    let bytes = match axum::body::to_bytes(body, MAX_LOGGED_BODY_BYTES).await {
        Ok(b) => b,
        Err(e) => {
            tracing::warn!(error = %e, limit = MAX_LOGGED_BODY_BYTES, "request body exceeded log buffer cap");
            return (
                StatusCode::PAYLOAD_TOO_LARGE,
                format!("request body exceeds {MAX_LOGGED_BODY_BYTES} byte limit"),
            )
                .into_response();
        }
    };

    let (method, tool_name, params_summary) = parse_jsonrpc_for_logging(&bytes);

    let request = Request::from_parts(parts, Body::from(bytes));
    let start = Instant::now();
    let response = next.run(request).await;
    let duration_ms = start.elapsed().as_millis() as u64;
    let success = response.status().is_success();

    // Only log actual JSON-RPC calls
    if let Some(method) = method {
        let now_epoch_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        let id = state.next_log_id.fetch_add(1, Ordering::Relaxed);

        let entry = RequestLogEntry {
            id,
            timestamp_epoch_ms: now_epoch_ms,
            method,
            tool_name,
            params_summary,
            duration_ms,
            success,
            token_prefix,
        };

        let mut log = state.request_log.write().await;
        if log.len() >= MAX_LOG_ENTRIES {
            log.pop_front();
        }
        log.push_back(entry);
    }

    response
}

fn parse_jsonrpc_for_logging(bytes: &[u8]) -> (Option<String>, Option<String>, Option<String>) {
    let Ok(v) = serde_json::from_slice::<serde_json::Value>(bytes) else {
        return (None, None, None);
    };
    let method = v.get("method").and_then(|m| m.as_str()).map(String::from);
    let tool_name = v
        .get("params")
        .and_then(|p| p.get("name"))
        .and_then(|n| n.as_str())
        .map(String::from);
    let params_summary = v.get("params").and_then(|p| p.get("arguments")).map(|a| {
        let s = a.to_string();
        if s.len() > 200 {
            format!("{}...", &s[..200])
        } else {
            s
        }
    });
    (method, tool_name, params_summary)
}

// --- Admin Auth ---

async fn admin_auth_middleware(
    State(state): State<AdminAppState>,
    request: Request<Body>,
    next: Next,
) -> Response {
    let (_, admin) = state.as_ref();

    let session_id = request
        .headers()
        .get("cookie")
        .and_then(|v| v.to_str().ok())
        .and_then(parse_admin_session_cookie);

    match session_id {
        Some(sid) => {
            let sessions = admin.admin_sessions.read().await;
            match sessions.get(&sid) {
                Some(created_at) if created_at.elapsed() < ADMIN_SESSION_TTL => {
                    drop(sessions);
                    next.run(request).await
                }
                _ => {
                    drop(sessions);
                    StatusCode::UNAUTHORIZED.into_response()
                }
            }
        }
        None => StatusCode::UNAUTHORIZED.into_response(),
    }
}

fn parse_admin_session_cookie(cookie_header: &str) -> Option<String> {
    cookie_header.split(';').find_map(|part| {
        let part = part.trim();
        part.strip_prefix("admin_session=").map(|v| v.to_string())
    })
}

// --- Login / Logout ---

#[derive(Deserialize)]
struct LoginRequest {
    password: String,
}

async fn admin_login(
    State(state): State<AdminAppState>,
    Json(req): Json<LoginRequest>,
) -> Response {
    let (oauth, admin) = state.as_ref();

    if !oauth.verify_admin_password(&req.password) {
        return (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({"error": "invalid password"})),
        )
            .into_response();
    }

    let session_id = Uuid::new_v4().to_string();
    admin
        .admin_sessions
        .write()
        .await
        .insert(session_id.clone(), Instant::now());

    (
        StatusCode::OK,
        [(
            "Set-Cookie",
            format!("admin_session={session_id}; HttpOnly; SameSite=Strict; Path=/admin"),
        )],
        Json(serde_json::json!({"ok": true})),
    )
        .into_response()
}

async fn admin_logout(State(state): State<AdminAppState>, request: Request<Body>) -> Response {
    let (_, admin) = state.as_ref();

    if let Some(sid) = request
        .headers()
        .get("cookie")
        .and_then(|v| v.to_str().ok())
        .and_then(parse_admin_session_cookie)
    {
        admin.admin_sessions.write().await.remove(&sid);
    }

    (
        StatusCode::OK,
        [(
            "Set-Cookie",
            "admin_session=; HttpOnly; SameSite=Strict; Path=/admin; Max-Age=0".to_string(),
        )],
        Json(serde_json::json!({"ok": true})),
    )
        .into_response()
}

// --- Dashboard ---

#[derive(Serialize)]
struct DashboardResponse {
    total_requests: usize,
    active_clients: usize,
    active_tokens: usize,
    requests_per_minute: f64,
    uptime_secs: u64,
}

async fn dashboard_handler(State(state): State<AdminAppState>) -> Json<DashboardResponse> {
    let (oauth, admin) = state.as_ref();

    let log = admin.request_log.read().await;
    let total_requests = log.len();

    let now_epoch_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    let one_min_ago = now_epoch_ms.saturating_sub(60_000);
    let recent_count = log
        .iter()
        .rev()
        .take_while(|e| e.timestamp_epoch_ms >= one_min_ago)
        .count();
    drop(log);

    let requests_per_minute = recent_count as f64;
    let active_clients = oauth.active_client_count().await;
    let active_tokens = oauth.active_token_count().await;
    let uptime_secs = admin.start_time.elapsed().as_secs();

    Json(DashboardResponse {
        total_requests,
        active_clients,
        active_tokens,
        requests_per_minute,
        uptime_secs,
    })
}

// --- Logs ---

#[derive(Deserialize)]
struct LogsQuery {
    limit: Option<usize>,
    offset: Option<usize>,
}

#[derive(Serialize)]
struct LogsResponse {
    entries: Vec<RequestLogEntry>,
    total: usize,
}

async fn logs_handler(
    State(state): State<AdminAppState>,
    Query(q): Query<LogsQuery>,
) -> Json<LogsResponse> {
    let (_, admin) = state.as_ref();
    let log = admin.request_log.read().await;
    let total = log.len();
    let limit = q.limit.unwrap_or(50).min(200);
    let offset = q.offset.unwrap_or(0);

    // Return newest first
    let entries: Vec<RequestLogEntry> =
        log.iter().rev().skip(offset).take(limit).cloned().collect();

    Json(LogsResponse { entries, total })
}

// --- Clients ---

async fn clients_handler(State(state): State<AdminAppState>) -> Json<serde_json::Value> {
    let (oauth, _) = state.as_ref();
    let clients = oauth.list_clients().await;
    Json(serde_json::json!({ "clients": clients }))
}

async fn delete_client_handler(
    State(state): State<AdminAppState>,
    Path(client_id): Path<String>,
) -> StatusCode {
    let (oauth, _) = state.as_ref();
    if oauth.revoke_client(&client_id).await {
        StatusCode::OK
    } else {
        StatusCode::NOT_FOUND
    }
}

// --- Tokens ---

async fn tokens_handler(State(state): State<AdminAppState>) -> Json<serde_json::Value> {
    let (oauth, _) = state.as_ref();
    let tokens = oauth.list_tokens().await;
    Json(serde_json::json!({ "tokens": tokens }))
}

#[derive(Deserialize)]
struct CreateTokenRequest {
    label: Option<String>,
    expires_in_secs: Option<u64>,
}

async fn create_token_handler(
    State(state): State<AdminAppState>,
    Json(req): Json<CreateTokenRequest>,
) -> Response {
    let (oauth, _) = state.as_ref();

    let label = req
        .label
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    // Default 1 hour to match OAuth-minted tokens; cap is enforced inside OAuthState.
    let ttl = Duration::from_secs(req.expires_in_secs.unwrap_or(3600));
    if ttl.is_zero() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "expires_in_secs must be > 0"})),
        )
            .into_response();
    }

    let created = oauth.create_admin_token(label, ttl).await;
    (StatusCode::CREATED, Json(created)).into_response()
}

async fn delete_token_handler(
    State(state): State<AdminAppState>,
    Path(token_prefix): Path<String>,
) -> StatusCode {
    let (oauth, _) = state.as_ref();
    if oauth.revoke_token(&token_prefix).await {
        StatusCode::OK
    } else {
        StatusCode::NOT_FOUND
    }
}
