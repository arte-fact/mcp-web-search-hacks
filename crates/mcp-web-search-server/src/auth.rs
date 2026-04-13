use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use axum::body::Body;
use axum::extract::{Query, State};
use axum::http::{Request, StatusCode};
use axum::middleware::Next;
use axum::response::{Html, IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use axum::{Form, Json, Router};
use serde::{Deserialize, Serialize};
use sha2::Digest;
use tokio::sync::RwLock;
use uuid::Uuid;

// --- State ---

pub struct OAuthState {
    admin_password: String,
    base_url: String,
    clients: RwLock<HashMap<String, RegisteredClient>>,
    auth_codes: RwLock<HashMap<String, AuthCodeEntry>>,
    access_tokens: RwLock<HashMap<String, TokenEntry>>,
}

struct RegisteredClient {
    #[allow(dead_code)]
    client_secret: String,
    client_name: Option<String>,
    redirect_uris: Vec<String>,
    registered_at_epoch_ms: u64,
}

struct AuthCodeEntry {
    client_id: String,
    redirect_uri: String,
    code_challenge: String,
    created_at: Instant,
}

struct TokenEntry {
    client_id: String,
    label: Option<String>,
    created_at: Instant,
    created_at_epoch_ms: u64,
    expires_in: Duration,
}

const AUTH_CODE_TTL: Duration = Duration::from_secs(600);
const TOKEN_TTL: Duration = Duration::from_secs(3600);
const MAX_ADMIN_TOKEN_TTL: Duration = Duration::from_secs(365 * 24 * 60 * 60);

/// Sentinel client_id for bearer tokens minted directly from the admin panel
/// (i.e. not via the OAuth authorization code flow). It intentionally doesn't
/// correspond to any registered OAuth client — `list_clients`/`revoke_client`
/// look up real clients by UUID, so the sentinel never collides.
pub(crate) const ADMIN_TOKEN_CLIENT_ID: &str = "admin";

impl OAuthState {
    pub fn new(admin_password: String, base_url: String) -> Self {
        Self {
            admin_password,
            base_url: base_url.trim_end_matches('/').to_string(),
            clients: RwLock::new(HashMap::new()),
            auth_codes: RwLock::new(HashMap::new()),
            access_tokens: RwLock::new(HashMap::new()),
        }
    }
}

// --- Router ---

pub fn router(state: Arc<OAuthState>) -> Router {
    Router::new()
        .route(
            "/.well-known/oauth-protected-resource",
            get(protected_resource_metadata_handler),
        )
        .route(
            "/.well-known/oauth-authorization-server",
            get(metadata_handler),
        )
        .route("/register", post(register_handler))
        .route("/authorize", get(authorize_page).post(authorize_submit))
        .route("/token", post(token_handler))
        .with_state(state)
}

// --- Protected Resource Metadata (RFC 9728) ---

#[derive(Serialize)]
struct ProtectedResourceMetadata {
    resource: String,
    authorization_servers: Vec<String>,
    bearer_methods_supported: Vec<String>,
    scopes_supported: Vec<String>,
}

async fn protected_resource_metadata_handler(
    State(state): State<Arc<OAuthState>>,
) -> Json<ProtectedResourceMetadata> {
    Json(ProtectedResourceMetadata {
        resource: state.base_url.clone(),
        authorization_servers: vec![state.base_url.clone()],
        bearer_methods_supported: vec!["header".into()],
        scopes_supported: vec!["mcp".into()],
    })
}

// --- 1. Authorization Server Metadata (RFC 8414) ---

#[derive(Serialize)]
struct AuthorizationServerMetadata {
    issuer: String,
    authorization_endpoint: String,
    token_endpoint: String,
    registration_endpoint: String,
    response_types_supported: Vec<String>,
    grant_types_supported: Vec<String>,
    code_challenge_methods_supported: Vec<String>,
    token_endpoint_auth_methods_supported: Vec<String>,
    scopes_supported: Vec<String>,
}

async fn metadata_handler(
    State(state): State<Arc<OAuthState>>,
) -> Json<AuthorizationServerMetadata> {
    Json(AuthorizationServerMetadata {
        issuer: state.base_url.clone(),
        authorization_endpoint: format!("{}/authorize", state.base_url),
        token_endpoint: format!("{}/token", state.base_url),
        registration_endpoint: format!("{}/register", state.base_url),
        response_types_supported: vec!["code".into()],
        grant_types_supported: vec!["authorization_code".into()],
        code_challenge_methods_supported: vec!["S256".into()],
        token_endpoint_auth_methods_supported: vec!["client_secret_post".into()],
        scopes_supported: vec!["mcp".into()],
    })
}

// --- 2. Dynamic Client Registration (RFC 7591) ---

#[derive(Deserialize)]
struct RegistrationRequest {
    client_name: Option<String>,
    redirect_uris: Vec<String>,
}

#[derive(Serialize)]
struct RegistrationResponse {
    client_id: String,
    client_secret: String,
    client_name: Option<String>,
    redirect_uris: Vec<String>,
}

async fn register_handler(
    State(state): State<Arc<OAuthState>>,
    Json(req): Json<RegistrationRequest>,
) -> (StatusCode, Json<RegistrationResponse>) {
    let client_id = Uuid::new_v4().to_string();
    let client_secret = Uuid::new_v4().to_string();

    let now_epoch_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;

    state.clients.write().await.insert(
        client_id.clone(),
        RegisteredClient {
            client_secret: client_secret.clone(),
            client_name: req.client_name.clone(),
            redirect_uris: req.redirect_uris.clone(),
            registered_at_epoch_ms: now_epoch_ms,
        },
    );

    tracing::info!(client_id = %client_id, "registered new OAuth client");

    (
        StatusCode::CREATED,
        Json(RegistrationResponse {
            client_id,
            client_secret,
            client_name: req.client_name,
            redirect_uris: req.redirect_uris,
        }),
    )
}

// --- 3. Authorization Endpoint ---

#[derive(Deserialize)]
pub struct AuthorizeQuery {
    response_type: String,
    client_id: String,
    redirect_uri: String,
    state: Option<String>,
    code_challenge: Option<String>,
    code_challenge_method: Option<String>,
    scope: Option<String>,
}

async fn authorize_page(
    State(oauth): State<Arc<OAuthState>>,
    Query(q): Query<AuthorizeQuery>,
) -> Result<Html<String>, (StatusCode, String)> {
    if q.response_type != "code" {
        return Err((
            StatusCode::BAD_REQUEST,
            "unsupported response_type".to_string(),
        ));
    }

    let clients = oauth.clients.read().await;
    let client = clients
        .get(&q.client_id)
        .ok_or((StatusCode::BAD_REQUEST, "unknown client_id".to_string()))?;

    if !client.redirect_uris.contains(&q.redirect_uri) {
        return Err((
            StatusCode::BAD_REQUEST,
            "redirect_uri not registered".to_string(),
        ));
    }

    // PKCE is mandatory per MCP spec (2025-11-25)
    let code_challenge = match q.code_challenge.as_deref() {
        Some(c) if !c.is_empty() => c,
        _ => {
            return Err((
                StatusCode::BAD_REQUEST,
                "code_challenge is required (PKCE is mandatory)".to_string(),
            ));
        }
    };

    match q.code_challenge_method.as_deref() {
        Some("S256") => {}
        _ => {
            return Err((
                StatusCode::BAD_REQUEST,
                "code_challenge_method must be S256".to_string(),
            ));
        }
    }

    let html = format!(
        r#"<!DOCTYPE html>
<html>
<head><title>MCP Server Authorization</title>
<style>
body {{ font-family: system-ui, sans-serif; max-width: 400px; margin: 80px auto; padding: 0 20px; }}
h1 {{ font-size: 1.3em; }}
input[type=password] {{ width: 100%; padding: 8px; margin: 8px 0; box-sizing: border-box; }}
button {{ padding: 10px 20px; background: #2563eb; color: white; border: none; border-radius: 4px; cursor: pointer; }}
button:hover {{ background: #1d4ed8; }}
</style>
</head>
<body>
<h1>MCP Server Authorization</h1>
<p>Client <strong>{client_id}</strong> is requesting access{scope}.</p>
<form method="POST" action="/authorize">
  <input type="hidden" name="client_id" value="{client_id}">
  <input type="hidden" name="redirect_uri" value="{redirect_uri}">
  <input type="hidden" name="state" value="{state}">
  <input type="hidden" name="code_challenge" value="{code_challenge}">
  <input type="hidden" name="code_challenge_method" value="S256">
  <label>Admin Password</label>
  <input type="password" name="password" autofocus required>
  <button type="submit">Approve</button>
</form>
</body>
</html>"#,
        client_id = q.client_id,
        redirect_uri = q.redirect_uri,
        state = q.state.as_deref().unwrap_or(""),
        code_challenge = code_challenge,
        scope = q
            .scope
            .as_ref()
            .map(|s| format!(" (scope: {s})"))
            .unwrap_or_default(),
    );

    Ok(Html(html))
}

#[derive(Deserialize)]
struct AuthorizeForm {
    client_id: String,
    redirect_uri: String,
    state: Option<String>,
    code_challenge: Option<String>,
    #[allow(dead_code)]
    code_challenge_method: Option<String>,
    password: String,
}

async fn authorize_submit(
    State(oauth): State<Arc<OAuthState>>,
    Form(form): Form<AuthorizeForm>,
) -> Result<Redirect, (StatusCode, String)> {
    if form.password != oauth.admin_password {
        return Err((StatusCode::FORBIDDEN, "invalid password".to_string()));
    }

    let code = Uuid::new_v4().to_string();

    oauth.auth_codes.write().await.insert(
        code.clone(),
        AuthCodeEntry {
            client_id: form.client_id,
            redirect_uri: form.redirect_uri.clone(),
            code_challenge: form.code_challenge.unwrap_or_default(),
            created_at: Instant::now(),
        },
    );

    let separator = if form.redirect_uri.contains('?') {
        "&"
    } else {
        "?"
    };
    let mut url = format!("{}{}code={}", form.redirect_uri, separator, code);
    if let Some(state) = form.state {
        url.push_str(&format!("&state={state}"));
    }

    Ok(Redirect::to(&url))
}

// --- 4. Token Endpoint ---

#[derive(Deserialize)]
struct TokenRequest {
    grant_type: String,
    code: String,
    client_id: String,
    redirect_uri: Option<String>,
    code_verifier: Option<String>,
}

#[derive(Serialize)]
struct TokenResponse {
    access_token: String,
    token_type: String,
    expires_in: u64,
}

#[derive(Serialize)]
struct TokenError {
    error: String,
    error_description: String,
}

async fn token_handler(
    State(state): State<Arc<OAuthState>>,
    Form(req): Form<TokenRequest>,
) -> Result<Json<TokenResponse>, (StatusCode, Json<TokenError>)> {
    if req.grant_type != "authorization_code" {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(TokenError {
                error: "unsupported_grant_type".into(),
                error_description: "only authorization_code is supported".into(),
            }),
        ));
    }

    let entry = state
        .auth_codes
        .write()
        .await
        .remove(&req.code)
        .ok_or_else(|| {
            (
                StatusCode::BAD_REQUEST,
                Json(TokenError {
                    error: "invalid_grant".into(),
                    error_description: "invalid or expired authorization code".into(),
                }),
            )
        })?;

    if entry.created_at.elapsed() > AUTH_CODE_TTL {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(TokenError {
                error: "invalid_grant".into(),
                error_description: "authorization code expired".into(),
            }),
        ));
    }

    if entry.client_id != req.client_id {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(TokenError {
                error: "invalid_grant".into(),
                error_description: "client_id mismatch".into(),
            }),
        ));
    }

    if let Some(ref redirect_uri) = req.redirect_uri
        && *redirect_uri != entry.redirect_uri
    {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(TokenError {
                error: "invalid_grant".into(),
                error_description: "redirect_uri mismatch".into(),
            }),
        ));
    }

    // PKCE verification (mandatory per MCP spec 2025-11-25)
    let verifier = req.code_verifier.as_deref().ok_or_else(|| {
        (
            StatusCode::BAD_REQUEST,
            Json(TokenError {
                error: "invalid_grant".into(),
                error_description: "code_verifier is required (PKCE is mandatory)".into(),
            }),
        )
    })?;

    if !verify_pkce(verifier, &entry.code_challenge) {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(TokenError {
                error: "invalid_grant".into(),
                error_description: "PKCE verification failed".into(),
            }),
        ));
    }

    let access_token = Uuid::new_v4().to_string();
    let now_epoch_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    state.access_tokens.write().await.insert(
        access_token.clone(),
        TokenEntry {
            client_id: req.client_id.clone(),
            label: None,
            created_at: Instant::now(),
            created_at_epoch_ms: now_epoch_ms,
            expires_in: TOKEN_TTL,
        },
    );

    tracing::info!(client_id = %req.client_id, "issued access token");

    Ok(Json(TokenResponse {
        access_token,
        token_type: "bearer".into(),
        expires_in: TOKEN_TTL.as_secs(),
    }))
}

fn verify_pkce(code_verifier: &str, code_challenge: &str) -> bool {
    use base64::Engine;
    let hash = sha2::Sha256::digest(code_verifier.as_bytes());
    let computed = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(hash);
    computed == code_challenge
}

// --- Auth Middleware ---

// --- Admin Accessors ---

#[derive(Serialize)]
pub(crate) struct ClientInfo {
    pub client_id: String,
    pub client_name: Option<String>,
    pub redirect_uris: Vec<String>,
    pub registered_at_epoch_ms: u64,
    pub active_tokens: usize,
}

#[derive(Serialize)]
pub(crate) struct TokenInfo {
    pub token_prefix: String,
    pub client_id: String,
    pub label: Option<String>,
    pub created_at_epoch_ms: u64,
    pub expires_in_secs: u64,
    pub remaining_secs: u64,
}

#[derive(Serialize)]
pub(crate) struct CreatedAdminToken {
    pub access_token: String,
    pub token_prefix: String,
    pub label: Option<String>,
    pub expires_in_secs: u64,
}

impl OAuthState {
    pub(crate) fn verify_admin_password(&self, password: &str) -> bool {
        self.admin_password == password
    }

    pub(crate) async fn list_clients(&self) -> Vec<ClientInfo> {
        let clients = self.clients.read().await;
        let tokens = self.access_tokens.read().await;
        clients
            .iter()
            .map(|(id, c)| {
                let active_tokens = tokens
                    .values()
                    .filter(|t| t.client_id == *id && t.created_at.elapsed() < t.expires_in)
                    .count();
                ClientInfo {
                    client_id: id.clone(),
                    client_name: c.client_name.clone(),
                    redirect_uris: c.redirect_uris.clone(),
                    registered_at_epoch_ms: c.registered_at_epoch_ms,
                    active_tokens,
                }
            })
            .collect()
    }

    pub(crate) async fn list_tokens(&self) -> Vec<TokenInfo> {
        let tokens = self.access_tokens.read().await;
        tokens
            .iter()
            .filter_map(|(full_token, t)| {
                let elapsed = t.created_at.elapsed();
                if elapsed >= t.expires_in {
                    return None;
                }
                let remaining = t.expires_in - elapsed;
                Some(TokenInfo {
                    token_prefix: full_token[..8.min(full_token.len())].to_string(),
                    client_id: t.client_id.clone(),
                    label: t.label.clone(),
                    created_at_epoch_ms: t.created_at_epoch_ms,
                    expires_in_secs: t.expires_in.as_secs(),
                    remaining_secs: remaining.as_secs(),
                })
            })
            .collect()
    }

    /// Mint a bearer token directly (no OAuth flow). Used by the admin panel
    /// to hand out long-lived tokens for MCP clients that don't support OAuth.
    /// The TTL is clamped to [`MAX_ADMIN_TOKEN_TTL`].
    pub(crate) async fn create_admin_token(
        &self,
        label: Option<String>,
        ttl: Duration,
    ) -> CreatedAdminToken {
        let ttl = ttl.min(MAX_ADMIN_TOKEN_TTL);
        let access_token = Uuid::new_v4().to_string();
        let token_prefix = access_token[..8.min(access_token.len())].to_string();
        let created_at_epoch_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        self.access_tokens.write().await.insert(
            access_token.clone(),
            TokenEntry {
                client_id: ADMIN_TOKEN_CLIENT_ID.to_string(),
                label: label.clone(),
                created_at: Instant::now(),
                created_at_epoch_ms,
                expires_in: ttl,
            },
        );

        tracing::info!(
            token_prefix = %token_prefix,
            label = ?label,
            ttl_secs = ttl.as_secs(),
            "admin panel issued bearer token"
        );

        CreatedAdminToken {
            access_token,
            token_prefix,
            label,
            expires_in_secs: ttl.as_secs(),
        }
    }

    pub(crate) async fn revoke_client(&self, client_id: &str) -> bool {
        let removed = self.clients.write().await.remove(client_id).is_some();
        if removed {
            self.access_tokens
                .write()
                .await
                .retain(|_, t| t.client_id != client_id);
        }
        removed
    }

    pub(crate) async fn revoke_token(&self, token_prefix: &str) -> bool {
        let mut tokens = self.access_tokens.write().await;
        let key = tokens
            .keys()
            .find(|k| k.starts_with(token_prefix))
            .cloned();
        match key {
            Some(k) => tokens.remove(&k).is_some(),
            None => false,
        }
    }

    pub(crate) async fn active_client_count(&self) -> usize {
        self.clients.read().await.len()
    }

    pub(crate) async fn active_token_count(&self) -> usize {
        let tokens = self.access_tokens.read().await;
        tokens
            .values()
            .filter(|t| t.created_at.elapsed() < t.expires_in)
            .count()
    }
}

// --- Auth Middleware ---

pub async fn auth_middleware(
    State(state): State<Arc<OAuthState>>,
    request: Request<Body>,
    next: Next,
) -> Response {
    let auth_header = request
        .headers()
        .get("authorization")
        .and_then(|v| v.to_str().ok());

    let token = match auth_header.and_then(|h| h.strip_prefix("Bearer ")) {
        Some(t) => t,
        None => {
            return (
                StatusCode::UNAUTHORIZED,
                [(
                    "WWW-Authenticate",
                    format!(
                        "Bearer resource_metadata=\"{}/.well-known/oauth-protected-resource\"",
                        state.base_url
                    ),
                )],
                "authentication required",
            )
                .into_response();
        }
    };

    let tokens = state.access_tokens.read().await;
    match tokens.get(token) {
        Some(entry) if entry.created_at.elapsed() < entry.expires_in => {
            drop(tokens);
            next.run(request).await
        }
        _ => {
            drop(tokens);
            (
                StatusCode::UNAUTHORIZED,
                [(
                    "WWW-Authenticate",
                    format!(
                        "Bearer error=\"invalid_token\", resource_metadata=\"{}/.well-known/oauth-protected-resource\"",
                        state.base_url
                    ),
                )],
                "invalid or expired token",
            )
                .into_response()
        }
    }
}
