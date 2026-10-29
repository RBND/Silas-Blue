//! REST API routes, OAuth2/OTP handlers, and embedded SPA static asset serving (`rust-embed`).

use crate::auth::{
    build_clear_cookie, build_oauth_cookie, build_session_cookie, extract_cookie,
    verify_csrf, AuthManager, SessionClaims, OAUTH_PKCE_COOKIE_NAME, OAUTH_STATE_COOKIE_NAME,
};
use crate::ws::ws_logs_handler;
use axum::body::Body;
use axum::extract::{ConnectInfo, Path, Query, State};
use axum::http::{header, HeaderMap, HeaderValue, Method, Request, StatusCode, Uri};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::{delete, get, post, put};
use axum::{Json, Router};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use chrono::Utc;
use rand::RngCore;
use rust_embed::RustEmbed;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::Digest;
use silas_ai::detector::probe_local_providers;
use silas_ai::{validate_endpoint_url, ProviderManager, REMOTE_PRESETS};
use silas_core::config::{AppSettingsPatch, GuildConfigPatch};
use silas_core::events::{TelemetryBus, TelemetryEvent};
use silas_core::permissions::{
    has_guild_permission, AccessLevel, ConsoleDomain, MemberContext, PermissionDomain,
};
use silas_core::quota::calculate_window_period;
use silas_discord::BotController;
use silas_search::SearchCoordinator;
use silas_core::tagscript::{process_tagscript, TagScriptContext};
use silas_storage::{
    ActivityConfig, AutomodConfig, CommandChannelRule, GreetingsConfig, GuildCommandRestrictions,
    LoggingConfig, NewActivityRewardRule, NewCustomTrigger, NewModCase, NewReactionRoleEntry,
    NewReactionRoleGroup, NewWebAuditLog, Storage,
};
use silas_updater::{BotUpdater, CURRENT_VERSION};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use tower_http::limit::RequestBodyLimitLayer;

#[derive(RustEmbed)]
#[folder = "assets/"]
pub struct EmbeddedAssets;

pub struct WebState {
    pub storage: Storage,
    pub bot: BotController,
    pub ai: Arc<ProviderManager>,
    pub search: Arc<SearchCoordinator>,
    pub updater: Arc<BotUpdater>,
    pub telemetry: TelemetryBus,
    pub auth: Arc<AuthManager>,
}

/// Build the complete Axum router with security headers, 256 KB body limit, CSRF guard, and embedded SPA assets.
pub fn build_router(state: Arc<WebState>) -> Router {
    let api_router = Router::new()
        .route("/status", get(api_status))
        .route("/bot/start", post(api_bot_start))
        .route("/bot/stop", post(api_bot_stop))
        .route("/bot/restart", post(api_bot_restart))
        .route("/bot/profile", post(api_bot_profile))
        .route("/settings", get(api_get_settings).put(api_put_settings))
        .route("/ai/presets", get(api_ai_presets))
        .route("/ai/models", get(api_ai_models))
        .route("/ai/detect", post(api_ai_detect))
        .route("/ai/pull", post(api_ai_pull))
        .route("/search/test", post(api_search_test))
        .route("/guilds", get(api_list_guilds))
        .route(
            "/guilds/:guild_id/config",
            get(api_get_guild_config).put(api_put_guild_config),
        )
        .route("/guilds/:guild_id/icon", post(api_set_guild_icon))
        .route("/guilds/:guild_id/roles", get(api_get_guild_roles))
        .route("/guilds/:guild_id/channels", get(api_get_guild_channels))
        .route("/guilds/:guild_id/emojis", get(api_get_guild_emojis))
        .route("/guilds/:guild_id/members", get(api_search_guild_members))
        .route(
            "/guilds/:guild_id/grants",
            get(api_list_guild_grants).post(api_upsert_guild_grant),
        )
        .route(
            "/guilds/:guild_id/grants/:grant_id",
            delete(api_delete_guild_grant),
        )
        .route("/guilds/:guild_id/audit-log", get(api_get_guild_audit_log))
        .route("/audit-log", get(api_get_global_audit_log))
        .route("/guilds/:guild_id/quota/reset", post(api_reset_guild_quota))
        .route(
            "/guilds/:guild_id/memory/clear",
            post(api_clear_guild_memory),
        )
        .route(
            "/guilds/:guild_id/reddit",
            get(api_list_guild_reddit_subs).post(api_create_guild_reddit_sub),
        )
        .route(
            "/guilds/:guild_id/reddit/:sub_id",
            delete(api_delete_guild_reddit_sub),
        )
        .route(
            "/guilds/:guild_id/cases",
            get(api_list_mod_cases).post(api_create_mod_case),
        )
        .route(
            "/guilds/:guild_id/cases/:case_id/reason",
            put(api_update_mod_case_reason),
        )
        .route(
            "/guilds/:guild_id/cases/:case_id",
            delete(api_deactivate_mod_case),
        )
        .route(
            "/guilds/:guild_id/strikes",
            get(api_get_guild_strikes),
        )
        .route(
            "/guilds/:guild_id/strikes/decay",
            post(api_decay_guild_strikes),
        )
        .route(
            "/guilds/:guild_id/automod",
            get(api_get_automod_config).put(api_put_automod_config),
        )
        .route(
            "/guilds/:guild_id/logging",
            get(api_get_logging_config).put(api_put_logging_config),
        )
        .route(
            "/guilds/:guild_id/greetings",
            get(api_get_greetings_config).put(api_put_greetings_config),
        )
        .route(
            "/guilds/:guild_id/reaction-roles",
            get(api_list_reaction_roles).post(api_create_reaction_roles),
        )
        .route(
            "/guilds/:guild_id/reaction-roles/:group_id",
            delete(api_delete_reaction_role_group),
        )
        .route(
            "/guilds/:guild_id/triggers",
            get(api_list_custom_triggers).post(api_create_custom_trigger),
        )
        .route(
            "/guilds/:guild_id/triggers/:trigger_name",
            delete(api_delete_custom_trigger),
        )
        .route(
            "/tagscript/preview",
            post(api_tagscript_preview),
        )
        .route(
            "/guilds/:guild_id/activity/config",
            get(api_get_activity_config).put(api_put_activity_config),
        )
        .route(
            "/guilds/:guild_id/activity/leaderboard",
            get(api_get_activity_leaderboard),
        )
        .route(
            "/guilds/:guild_id/activity/rewards",
            get(api_list_activity_rewards).post(api_create_activity_reward),
        )
        .route(
            "/guilds/:guild_id/activity/rewards/:rule_id",
            delete(api_delete_activity_reward),
        )
        .route(
            "/guilds/:guild_id/activity/points/adjust",
            post(api_adjust_activity_points),
        )
        .route(
            "/guilds/:guild_id/command-channels",
            get(api_get_command_channels).put(api_put_command_channels),
        )
        .route("/reddit/generate-secret", post(api_generate_reddit_secret))
        .route("/update/check", get(api_update_check))
        .route("/update/apply", post(api_update_apply))
        .route("/update/snapshots", get(api_update_snapshots))
        .route("/update/snapshot", post(api_create_snapshot))
        .route("/backup/download", get(api_download_redacted_backup))
        .route("/ws/logs", get(ws_logs_handler))
        .route_layer(middleware::from_fn_with_state(
            Arc::clone(&state),
            require_auth_and_csrf,
        ));

    Router::new()
        .route("/auth/otp", get(auth_otp_handler))
        .route("/auth/login", get(auth_discord_login))
        .route("/auth/discord/login", get(auth_discord_login))
        .route("/auth/callback", get(auth_discord_callback))
        .route("/auth/discord/callback", get(auth_discord_callback))
        .route("/auth/logout", post(auth_logout))
        .route("/api/auth/session", get(api_auth_session))
        .route("/api/webhooks/reddit", post(api_webhook_reddit))
        .nest("/api", api_router)
        .fallback(serve_embedded_asset)
        .layer(RequestBodyLimitLayer::new(256 * 1024))
        .layer(middleware::from_fn(security_headers_middleware))
        .with_state(state)
}

/// Apply strict CSP (`default-src 'none'; script-src 'self'...`) and security headers to every HTTP response.
async fn security_headers_middleware(req: Request<Body>, next: Next) -> Response {
    let mut resp = next.run(req).await;
    let headers = resp.headers_mut();
    headers.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(
            "default-src 'none'; script-src 'self'; style-src 'self'; img-src 'self' https://cdn.discordapp.com data:; connect-src 'self' ws: wss:; font-src 'self'; manifest-src 'self'; frame-ancestors 'none'; base-uri 'self'; form-action 'self'",
        ),
    );
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(header::X_FRAME_OPTIONS, HeaderValue::from_static("DENY"));
    headers.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    resp
}

/// Middleware enforcing valid `silas_session` cookie and CSRF headers (`X-Requested-With: SilasBlue-SPA` + `Origin`) on `/api/*`.
async fn require_auth_and_csrf(
    State(state): State<Arc<WebState>>,
    req: Request<Body>,
    next: Next,
) -> Response {
    let settings = state.storage.get_app_settings();
    if state
        .auth
        .authenticate_request(req.headers(), &settings)
        .is_none()
    {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({ "error": "Authentication required." })),
        )
            .into_response();
    }

    if let Err((status, msg)) = verify_csrf(req.method(), req.headers()) {
        return (status, Json(json!({ "error": msg }))).into_response();
    }

    next.run(req).await
}

fn is_https_request(headers: &HeaderMap) -> bool {
    headers
        .get("x-forwarded-proto")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|s| s.eq_ignore_ascii_case("https"))
}

// ============================================================================
// Authentication Endpoints
// ============================================================================

#[derive(Deserialize)]
struct OtpQuery {
    token: Option<String>,
}

async fn auth_otp_handler(
    State(state): State<Arc<WebState>>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Query(q): Query<OtpQuery>,
) -> Response {
    let ip = addr.ip();
    if let Err(wait_secs) = state.auth.check_rate_limit(ip) {
        return (
            StatusCode::TOO_MANY_REQUESTS,
            format!("Too many failed login attempts. Try again in {wait_secs}s."),
        )
            .into_response();
    }

    let Some(raw_token) = q.token.as_deref() else {
        return Redirect::to("/login.html?error=missing_token").into_response();
    };

    if let Err(reason) = state.auth.verify_and_consume_otp(raw_token, ip) {
        state.auth.record_auth_failure(ip);
        state.telemetry.emit(TelemetryEvent::System {
            message: format!("Rejected OTP login attempt from {ip}: {reason}"),
        });
        return Redirect::to("/login.html?error=invalid_or_expired_otp").into_response();
    }

    state.auth.clear_rate_limit(ip);
    let settings = state.storage.get_app_settings();
    let is_owner = true; // Local loopback/LAN OTP is directly by the hosting bot administrator
    let Ok(session_tok) =
        state
            .auth
            .create_session_token("local-console", "Local Console Owner", is_owner, &settings)
    else {
        return (StatusCode::INTERNAL_SERVER_ERROR, "Session signing failed").into_response();
    };

    let cookie_str = build_session_cookie(&session_tok, is_https_request(&headers));
    let mut resp = Redirect::to("/").into_response();
    if let Ok(hv) = HeaderValue::from_str(&cookie_str) {
        resp.headers_mut().append(header::SET_COOKIE, hv);
    }
    resp
}

fn resolve_effective_client_id(state: &WebState, settings: &silas_core::config::AppSettings) -> String {
    let cid = settings.discord_oauth_client_id.trim();
    if !cid.is_empty() {
        return cid.to_string();
    }
    state.bot.bot_identity().id.unwrap_or_default()
}

async fn auth_discord_login(
    State(state): State<Arc<WebState>>,
    headers: HeaderMap,
) -> Response {
    let settings = state.storage.get_app_settings();
    let client_id = resolve_effective_client_id(&state, &settings);
    if client_id.is_empty() {
        return Redirect::to("/login.html?error=oauth_not_configured").into_response();
    }

    let is_https = is_https_request(&headers);
    let mut state_bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut state_bytes);
    let oauth_state = URL_SAFE_NO_PAD.encode(state_bytes);

    // Generate PKCE code verifier and code challenge (S256)
    let mut verifier_bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut verifier_bytes);
    let code_verifier = URL_SAFE_NO_PAD.encode(verifier_bytes);
    let challenge_digest = sha2::Sha256::digest(code_verifier.as_bytes());
    let code_challenge = URL_SAFE_NO_PAD.encode(challenge_digest);

    let redirect_uri = resolve_oauth_redirect_uri(&headers);
    let mut auth_url = match url::Url::parse("https://discord.com/oauth2/authorize") {
        Ok(u) => u,
        Err(_) => return Redirect::to("/login.html?error=url_error").into_response(),
    };
    auth_url
        .query_pairs_mut()
        .append_pair("client_id", &client_id)
        .append_pair("redirect_uri", &redirect_uri)
        .append_pair("response_type", "code")
        .append_pair("scope", "identify")
        .append_pair("state", &oauth_state)
        .append_pair("code_challenge", &code_challenge)
        .append_pair("code_challenge_method", "S256");

    let state_cookie = build_oauth_cookie(OAUTH_STATE_COOKIE_NAME, &oauth_state, is_https);
    let pkce_cookie = build_oauth_cookie(OAUTH_PKCE_COOKIE_NAME, &code_verifier, is_https);

    let mut resp = Redirect::to(auth_url.as_str()).into_response();
    if let Ok(hv) = HeaderValue::from_str(&state_cookie) {
        resp.headers_mut().append(header::SET_COOKIE, hv);
    }
    if let Ok(hv) = HeaderValue::from_str(&pkce_cookie) {
        resp.headers_mut().append(header::SET_COOKIE, hv);
    }
    resp
}

#[derive(Deserialize)]
struct OAuthCallbackQuery {
    code: Option<String>,
    state: Option<String>,
}

async fn auth_discord_callback(
    State(state): State<Arc<WebState>>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Query(q): Query<OAuthCallbackQuery>,
) -> Response {
    let ip = addr.ip();
    if let Err(wait_secs) = state.auth.check_rate_limit(ip) {
        return (
            StatusCode::TOO_MANY_REQUESTS,
            format!("Rate limited. Try again in {wait_secs}s."),
        )
            .into_response();
    }

    let Some(expected_state) = extract_cookie(&headers, OAUTH_STATE_COOKIE_NAME) else {
        state.auth.record_auth_failure(ip);
        return Redirect::to("/login.html?error=oauth_state_missing").into_response();
    };
    let Some(actual_state) = q.state.as_deref() else {
        state.auth.record_auth_failure(ip);
        return Redirect::to("/login.html?error=oauth_state_mismatch").into_response();
    };
    if expected_state != actual_state {
        state.auth.record_auth_failure(ip);
        return Redirect::to("/login.html?error=oauth_state_mismatch").into_response();
    }

    let Some(code) = q.code.as_deref() else {
        return Redirect::to("/login.html?error=oauth_cancelled").into_response();
    };

    let code_verifier = extract_cookie(&headers, OAUTH_PKCE_COOKIE_NAME);
    let settings = state.storage.get_app_settings();
    let effective_client_id = resolve_effective_client_id(&state, &settings);
    let redirect_uri = resolve_oauth_redirect_uri(&headers);

    match state
        .auth
        .verify_discord_oauth_callback(
            code,
            code_verifier.as_deref(),
            &redirect_uri,
            &effective_client_id,
            &settings,
        )
        .await
    {
        Ok((uid, uname, is_owner)) => {
            if !is_owner && !user_has_any_guild_access(&state, &uid, &uname).await {
                state.auth.record_auth_failure(ip);
                tracing::warn!(
                    "Discord OAuth2 login denied for {} ({}): no admin/mod/delegated access in any connected server",
                    uname,
                    uid
                );
                return Redirect::to("/login.html?error=unauthorized_discord_account")
                    .into_response();
            }
            state.auth.clear_rate_limit(ip);
            let Ok(session_tok) = state.auth.create_session_token(&uid, &uname, is_owner, &settings) else {
                return Redirect::to("/login.html?error=session_error").into_response();
            };
            let is_https = is_https_request(&headers);
            let sess_cookie = build_session_cookie(&session_tok, is_https);
            let clear_state = build_clear_cookie(OAUTH_STATE_COOKIE_NAME, is_https);
            let clear_pkce = build_clear_cookie(OAUTH_PKCE_COOKIE_NAME, is_https);

            let mut resp = Redirect::to("/").into_response();
            if let Ok(hv) = HeaderValue::from_str(&sess_cookie) {
                resp.headers_mut().append(header::SET_COOKIE, hv);
            }
            if let Ok(hv2) = HeaderValue::from_str(&clear_state) {
                resp.headers_mut().append(header::SET_COOKIE, hv2);
            }
            if let Ok(hv3) = HeaderValue::from_str(&clear_pkce) {
                resp.headers_mut().append(header::SET_COOKIE, hv3);
            }
            resp
        }
        Err(err) => {
            state.auth.record_auth_failure(ip);
            tracing::warn!("Discord OAuth2 callback rejected for {ip}: {err}");
            Redirect::to("/login.html?error=unauthorized_discord_account").into_response()
        }
    }
}

async fn auth_logout(headers: HeaderMap) -> Response {
    if verify_csrf(&Method::POST, &headers).is_err() {
        return (StatusCode::FORBIDDEN, "CSRF validation failed").into_response();
    }
    let clear = build_clear_cookie(crate::auth::SESSION_COOKIE_NAME, is_https_request(&headers));
    let mut resp = Json(json!({ "ok": true })).into_response();
    if let Ok(hv) = HeaderValue::from_str(&clear) {
        resp.headers_mut().append(header::SET_COOKIE, hv);
    }
    resp
}

async fn api_auth_session(
    State(state): State<Arc<WebState>>,
    headers: HeaderMap,
) -> Json<Value> {
    let settings = state.storage.get_app_settings();
    let oauth_enabled = !settings.discord_oauth_client_id.trim().is_empty()
        && !settings.discord_oauth_client_secret.trim().is_empty();

    if let Some(claims) = state.auth.authenticate_request(&headers, &settings) {
        let is_owner = claims.is_owner || state.auth.is_bot_owner(&claims.user_id, &settings);
        Json(json!({
            "authenticated": true,
            "user_id": claims.user_id,
            "username": claims.username,
            "is_owner": is_owner,
            "oauth_enabled": oauth_enabled,
            "version": CURRENT_VERSION,
        }))
    } else {
        Json(json!({
            "authenticated": false,
            "oauth_enabled": oauth_enabled,
            "version": CURRENT_VERSION,
        }))
    }
}

fn resolve_oauth_redirect_uri(headers: &HeaderMap) -> String {
    let scheme = if is_https_request(headers) {
        "https"
    } else {
        "http"
    };
    let raw_host = headers
        .get("x-forwarded-host")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.split(',').next())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .or_else(|| headers.get(header::HOST).and_then(|v| v.to_str().ok()))
        .unwrap_or("127.0.0.1:8080");

    let is_valid_host = !raw_host.is_empty()
        && raw_host
            .chars()
            .all(|c| c.is_alphanumeric() || c == '.' || c == ':' || c == '-');
    let host = if is_valid_host {
        raw_host
    } else {
        "127.0.0.1:8080"
    };
    format!("{scheme}://{host}/auth/discord/callback")
}

// ============================================================================
// Protected /api/* Endpoints
// ============================================================================

async fn api_status(State(state): State<Arc<WebState>>) -> Json<Value> {
    let settings = state.storage.get_app_settings();
    let bot_running = state.bot.is_running();
    let uptime = state.bot.uptime_seconds().await;
    let identity = state.bot.bot_identity();
    let guilds = state.bot.connected_guilds_list();
    let invite_url = state.bot.get_invite_url();
    let active_model = settings.effective_global_model();
    let mem_stats = state.bot.memory().stats();

    Json(json!({
        "bot_running": bot_running,
        "uptime_seconds": uptime,
        "version": CURRENT_VERSION,
        "connected_guilds": guilds.len(),
        "active_memory_channels": mem_stats.total_sessions,
        "backend_type": settings.backend_type,
        "active_model": active_model,
        "web_search_provider": settings.web_search_provider,
        "bot_identity": identity,
        "invite_url": invite_url,
        "has_discord_token": !settings.discord_token.trim().is_empty(),
    }))
}

fn is_request_owner(state: &WebState, headers: &HeaderMap) -> bool {
    let settings = state.storage.get_app_settings();
    match state.auth.authenticate_request(headers, &settings) {
        Some(c) => c.is_owner || state.auth.is_bot_owner(&c.user_id, &settings),
        None => false,
    }
}

async fn api_bot_start(State(state): State<Arc<WebState>>, headers: HeaderMap) -> Response {
    if !is_request_owner(&state, &headers) {
        return (
            StatusCode::FORBIDDEN,
            Json(json!({ "error": "Only the Bot Owner can start the bot." })),
        )
            .into_response();
    }
    match state.bot.start().await {
        Ok(()) => Json(json!({ "ok": true, "bot_running": true })).into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, Json(json!({ "error": e }))).into_response(),
    }
}

async fn api_bot_stop(State(state): State<Arc<WebState>>, headers: HeaderMap) -> Response {
    if !is_request_owner(&state, &headers) {
        return (
            StatusCode::FORBIDDEN,
            Json(json!({ "error": "Only the Bot Owner can stop the bot." })),
        )
            .into_response();
    }
    match state.bot.stop().await {
        Ok(()) => Json(json!({ "ok": true, "bot_running": false })).into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, Json(json!({ "error": e }))).into_response(),
    }
}

async fn api_bot_restart(State(state): State<Arc<WebState>>, headers: HeaderMap) -> Response {
    if !is_request_owner(&state, &headers) {
        return (
            StatusCode::FORBIDDEN,
            Json(json!({ "error": "Only the Bot Owner can restart the bot." })),
        )
            .into_response();
    }
    match state.bot.restart().await {
        Ok(()) => Json(json!({ "ok": true, "bot_running": true })).into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, Json(json!({ "error": e }))).into_response(),
    }
}

#[derive(Deserialize)]
struct UpdateProfilePayload {
    username: String,
}

async fn api_bot_profile(
    State(state): State<Arc<WebState>>,
    headers: HeaderMap,
    Json(payload): Json<UpdateProfilePayload>,
) -> Response {
    if !is_request_owner(&state, &headers) {
        return (
            StatusCode::FORBIDDEN,
            Json(json!({ "error": "Only the Bot Owner can change the global bot username." })),
        )
            .into_response();
    }
    match state.bot.update_global_username(&payload.username).await {
        Ok(new_name) => Json(json!({ "ok": true, "username": new_name })).into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, Json(json!({ "error": e }))).into_response(),
    }
}

async fn api_get_settings(State(state): State<Arc<WebState>>, headers: HeaderMap) -> Response {
    if !is_request_owner(&state, &headers) {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "Not Found" })),
        )
            .into_response();
    }
    let settings = state.storage.get_app_settings();
    Json(json!({
        "settings": settings.masked_for_web(),
    }))
    .into_response()
}

async fn api_put_settings(
    State(state): State<Arc<WebState>>,
    headers: HeaderMap,
    Json(patch): Json<AppSettingsPatch>,
) -> Response {
    let current = state.storage.get_app_settings();
    let old_epoch = current.session_epoch;
    let claims = state.auth.authenticate_request(&headers, &current);

    let is_owner = match &claims {
        Some(c) => c.is_owner || state.auth.is_bot_owner(&c.user_id, &current),
        None => false,
    };

    if !is_owner {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "Not Found" })),
        )
            .into_response();
    }

    // Validate custom URLs for SSRF before saving
    if let Some(ref url_str) = patch.ollama_url {
        if !url_str.trim().is_empty() {
            if let Err(e) = validate_endpoint_url(url_str, false, true) {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(json!({ "error": format!("Ollama URL: {e}") })),
                )
                    .into_response();
            }
        }
    }
    if let Some(ref url_str) = patch.llamacpp_url {
        if !url_str.trim().is_empty() {
            if let Err(e) = validate_endpoint_url(url_str, false, true) {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(json!({ "error": format!("llama.cpp URL: {e}") })),
                )
                    .into_response();
            }
        }
    }
    if let Some(ref url_str) = patch.remote_base_url {
        if !url_str.trim().is_empty() {
            if let Err(e) = validate_endpoint_url(url_str, true, false) {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(json!({ "error": format!("Remote Base URL: {e}") })),
                )
                    .into_response();
            }
        }
    }

    let updated = match state.storage.update_app_settings_async(patch).await {
        Ok(u) => u,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({ "error": e.to_string() })),
            )
                .into_response();
        }
    };

    state.ai.reload(&updated);
    let new_epoch = updated.session_epoch;
    let masked = updated.masked_for_web();

    // Audit log settings change
    if let Some(ref c) = claims {
        let before_json = serde_json::to_string(&current.masked_for_web()).unwrap_or_default();
        let after_json = serde_json::to_string(&masked).unwrap_or_default();
        let ip_hdr = headers
            .get("x-forwarded-for")
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.split(',').next())
            .map(str::trim)
            .unwrap_or("127.0.0.1");

        let uid_num = c.user_id.parse::<u64>().unwrap_or(0);
        let audit = NewWebAuditLog {
            guild_id: None,
            user_id: uid_num,
            user_name: &c.username,
            domain: ConsoleDomain::Settings.as_str(),
            action: "settings_update",
            before_json: Some(&before_json),
            after_json: Some(&after_json),
            ip_address: ip_hdr,
            status: "success",
        };
        let _ = state.storage.insert_web_audit_log(&audit);
    }

    let mut resp = Json(json!({
        "ok": true,
        "settings": masked,
    }))
    .into_response();

    // If session_epoch incremented (because credentials changed), automatically refresh the caller's cookie
    if new_epoch != old_epoch {
        if let Some(c) = claims {
            if let Ok(new_tok) = state
                .auth
                .create_session_token(&c.user_id, &c.username, c.is_owner, &updated)
            {
                let cookie_str = build_session_cookie(&new_tok, is_https_request(&headers));
                if let Ok(hv) = HeaderValue::from_str(&cookie_str) {
                    resp.headers_mut().append(header::SET_COOKIE, hv);
                }
            }
        }
    }

    resp
}

async fn api_ai_presets(State(state): State<Arc<WebState>>, headers: HeaderMap) -> Response {
    if !is_request_owner(&state, &headers) {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "Not Found" })),
        )
            .into_response();
    }
    Json(json!({ "presets": REMOTE_PRESETS })).into_response()
}

#[derive(Deserialize)]
struct ModelsQuery {
    refresh: Option<bool>,
}

async fn api_ai_models(
    State(state): State<Arc<WebState>>,
    headers: HeaderMap,
    Query(q): Query<ModelsQuery>,
) -> Json<Value> {
    let is_owner = is_request_owner(&state, &headers);
    let force_refresh = is_owner && q.refresh.unwrap_or(false);
    let models = state.ai.list_models_cached(force_refresh).await;
    Json(json!({ "ok": true, "models": models }))
}

async fn api_ai_detect(State(state): State<Arc<WebState>>, headers: HeaderMap) -> Response {
    if !is_request_owner(&state, &headers) {
        return (
            StatusCode::FORBIDDEN,
            Json(json!({ "error": "Only the Bot Owner can probe local AI backends." })),
        )
            .into_response();
    }
    let settings = state.storage.get_app_settings();
    let probe = probe_local_providers(&settings).await;
    Json(json!({ "ok": true, "probe": probe })).into_response()
}

#[derive(Deserialize)]
struct PullModelPayload {
    model: String,
}

async fn api_ai_pull(
    State(state): State<Arc<WebState>>,
    headers: HeaderMap,
    Json(payload): Json<PullModelPayload>,
) -> Response {
    if !is_request_owner(&state, &headers) {
        return (
            StatusCode::FORBIDDEN,
            Json(json!({ "error": "Only the Bot Owner can pull AI models." })),
        )
            .into_response();
    }
    let model = payload.model.trim().to_string();
    if model.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "Model name is required." })),
        )
            .into_response();
    }
    let telemetry = state.telemetry.clone();
    let model_for_cb = model.clone();

    let cb = Arc::new(move |pct: u8, status_txt: String| {
        telemetry.emit(TelemetryEvent::System {
            message: format!("Model pull [{model_for_cb}]: {status_txt} ({pct}%)"),
        });
    });

    match state.ai.get().pull_model(&model, Some(cb)).await {
        Ok(msg) => {
            let settings = state.storage.get_app_settings();
            state.ai.reload(&settings);
            Json(json!({ "ok": true, "model": model, "message": msg })).into_response()
        }
        Err(e) => (
            StatusCode::BAD_GATEWAY,
            Json(json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

#[derive(Deserialize)]
struct SearchTestPayload {
    query: String,
}

async fn api_search_test(
    State(state): State<Arc<WebState>>,
    headers: HeaderMap,
    Json(payload): Json<SearchTestPayload>,
) -> Response {
    if !is_request_owner(&state, &headers) {
        return (
            StatusCode::FORBIDDEN,
            Json(json!({ "error": "Only the Bot Owner can run global search diagnostics." })),
        )
            .into_response();
    }
    let settings = state.storage.get_app_settings();
    let results = state
        .search
        .search(&payload.query, Some(5), &settings)
        .await;
    Json(json!({
        "ok": true,
        "provider": settings.web_search_provider,
        "results": results,
    }))
    .into_response()
}

const GUILD_DOMAINS: [ConsoleDomain; 11] = [
    ConsoleDomain::Settings,
    ConsoleDomain::Moderation,
    ConsoleDomain::Automod,
    ConsoleDomain::Logging,
    ConsoleDomain::Greetings,
    ConsoleDomain::ReactionRoles,
    ConsoleDomain::Triggers,
    ConsoleDomain::Activity,
    ConsoleDomain::Reddit,
    ConsoleDomain::Permissions,
    ConsoleDomain::AuditLog,
];

/// Effective role and per-domain access profile for a caller in a specific guild.
#[derive(Debug, Clone, Serialize)]
pub struct GuildAccessProfile {
    pub role: &'static str,
    pub can_access: bool,
    pub domains: HashMap<&'static str, &'static str>,
}

impl GuildAccessProfile {
    pub fn none() -> Self {
        let mut domains = HashMap::with_capacity(GUILD_DOMAINS.len());
        for d in GUILD_DOMAINS {
            domains.insert(d.as_str(), AccessLevel::None.as_str());
        }
        Self {
            role: "none",
            can_access: false,
            domains,
        }
    }

    pub fn full(role: &'static str) -> Self {
        let mut domains = HashMap::with_capacity(GUILD_DOMAINS.len());
        for d in GUILD_DOMAINS {
            domains.insert(d.as_str(), AccessLevel::Edit.as_str());
        }
        Self {
            role,
            can_access: true,
            domains,
        }
    }

    pub fn domain_level(&self, domain: ConsoleDomain) -> AccessLevel {
        self.domains
            .get(domain.as_str())
            .and_then(|s| AccessLevel::from_str_relaxed(s))
            .unwrap_or(AccessLevel::None)
    }

    pub fn has_domain_access(&self, domain: ConsoleDomain, required: AccessLevel) -> bool {
        self.domain_level(domain) >= required
    }
}

/// Evaluates whether a newly authenticated Discord user has access to at least one connected or configured guild.
async fn user_has_any_guild_access(state: &WebState, user_id_str: &str, username: &str) -> bool {
    let Ok(user_id_num) = user_id_str.trim().parse::<u64>() else {
        return false;
    };
    let settings = state.storage.get_app_settings();
    if state.auth.is_bot_owner(user_id_str, &settings) {
        return true;
    }

    let temp_claims = Some(SessionClaims {
        user_id: user_id_str.to_string(),
        username: username.to_string(),
        is_owner: false,
        epoch: settings.session_epoch,
        exp: Utc::now().timestamp() + 60,
    });

    let mut checked = std::collections::HashSet::new();
    for g in state.bot.connected_guilds_list() {
        if let Ok(gid) = g.id.parse::<u64>() {
            checked.insert(gid);
            if resolve_guild_access(state, gid, user_id_num, &temp_claims)
                .await
                .can_access
            {
                return true;
            }
        }
    }

    for gid in state.storage.list_guild_ids() {
        if gid != 0
            && !checked.contains(&gid)
            && resolve_guild_access(state, gid, user_id_num, &temp_claims)
                .await
                .can_access
        {
            return true;
        }
    }

    false
}

async fn api_list_guilds(
    State(state): State<Arc<WebState>>,
    headers: HeaderMap,
) -> Json<Value> {
    let settings = state.storage.get_app_settings();
    let claims = state.auth.authenticate_request(&headers, &settings);
    let is_owner = match &claims {
        Some(c) => c.is_owner || state.auth.is_bot_owner(&c.user_id, &settings),
        None => false,
    };
    let user_id_num = claims
        .as_ref()
        .and_then(|c| c.user_id.parse::<u64>().ok())
        .unwrap_or(0);

    let connected = state.bot.connected_guilds_list();
    let mut seen_ids = std::collections::HashSet::new();
    let mut list = Vec::new();

    for g in connected {
        if let Ok(id_num) = g.id.parse::<u64>() {
            seen_ids.insert(id_num);
            let access = resolve_guild_access(&state, id_num, user_id_num, &claims).await;
            if !access.can_access {
                continue;
            }
            list.push(json!({
                "id": g.id,
                "name": g.name,
                "icon_url": g.icon_url,
                "member_count": g.member_count,
                "connected": true,
                "my_role": access.role,
                "permissions": access.domains,
            }));
        }
    }

    for gid in state.storage.list_guild_ids() {
        if gid != 0 && !seen_ids.contains(&gid) {
            let access = if is_owner {
                GuildAccessProfile::full("owner")
            } else {
                resolve_guild_access(&state, gid, user_id_num, &claims).await
            };
            if access.can_access {
                list.push(json!({
                    "id": gid.to_string(),
                    "name": format!("Server {gid}"),
                    "icon_url": null,
                    "member_count": 0,
                    "connected": false,
                    "my_role": access.role,
                    "permissions": access.domains,
                }));
            }
        }
    }

    Json(json!({ "guilds": list }))
}

/// Resolves the caller's full role (`owner`, `admin`, `moderator`, `delegated`, `none`)
/// and per-domain access levels (`none`, `view`, `edit`) for `guild_id`.
pub async fn resolve_guild_access(
    state: &WebState,
    guild_id: u64,
    user_id: u64,
    claims: &Option<SessionClaims>,
) -> GuildAccessProfile {
    let Some(c) = claims else {
        return GuildAccessProfile::none();
    };
    let settings = state.storage.get_app_settings();
    if c.is_owner || state.auth.is_bot_owner(&c.user_id, &settings) {
        return GuildAccessProfile::full("owner");
    }
    if user_id == 0 {
        return GuildAccessProfile::none();
    }

    let mut is_guild_owner = false;
    let mut is_discord_admin = false;
    let mut is_discord_mod = false;
    let mut member_role_ids: Option<Vec<u64>> = None;
    let mut member_role_names: Vec<String> = Vec::new();

    // 1. Check Serenity guild cache for owner_id and cached member roles
    if let Some(cache) = state.bot.cache() {
        if let Some(guild) = cache.guild(guild_id) {
            if guild.owner_id.get() == user_id {
                is_guild_owner = true;
            }
            if let Some(member) = guild.members.get(&serenity::model::id::UserId::new(user_id)) {
                let roles: Vec<u64> = member.roles.iter().map(|r| r.get()).collect();
                state.bot.member_role_cache().insert(guild_id, user_id, roles.clone());
                member_role_ids = Some(roles);
            }
        }
    }

    if is_guild_owner {
        return GuildAccessProfile::full("admin");
    }

    // 2. Check MemberRoleCache before falling back to Discord REST API
    if member_role_ids.is_none() {
        member_role_ids = state.bot.member_role_cache().get(guild_id, user_id);
    }

    // 3. Fallback to Discord REST API if member roles are not cached yet
    if member_role_ids.is_none() {
        if let Some(http) = state.bot.http() {
            if let Ok(member) = http
                .get_member(
                    serenity::model::id::GuildId::new(guild_id),
                    serenity::model::id::UserId::new(user_id),
                )
                .await
            {
                let roles: Vec<u64> = member.roles.iter().map(|r| r.get()).collect();
                state.bot.member_role_cache().insert(guild_id, user_id, roles.clone());
                if let Some(perms) = member.permissions {
                    if perms.contains(serenity::model::permissions::Permissions::ADMINISTRATOR)
                        || perms.contains(serenity::model::permissions::Permissions::MANAGE_GUILD)
                    {
                        is_discord_admin = true;
                    }
                    if perms.contains(serenity::model::permissions::Permissions::MODERATE_MEMBERS)
                        || perms.contains(serenity::model::permissions::Permissions::KICK_MEMBERS)
                        || perms.contains(serenity::model::permissions::Permissions::BAN_MEMBERS)
                        || perms.contains(serenity::model::permissions::Permissions::MANAGE_MESSAGES)
                    {
                        is_discord_mod = true;
                    }
                }
                member_role_ids = Some(roles);
            }
        }
    }

    let role_ids = member_role_ids.unwrap_or_default();

    // 4. Evaluate role permissions and role names from Serenity's guild roles cache
    if let Some(cache) = state.bot.cache() {
        if let Some(guild) = cache.guild(guild_id) {
            // Check @everyone role permissions as well as assigned roles
            if let Some(everyone_role) = guild.roles.get(&serenity::model::id::RoleId::new(guild_id)) {
                let p = everyone_role.permissions;
                if p.contains(serenity::model::permissions::Permissions::ADMINISTRATOR)
                    || p.contains(serenity::model::permissions::Permissions::MANAGE_GUILD)
                {
                    is_discord_admin = true;
                }
                if p.contains(serenity::model::permissions::Permissions::MODERATE_MEMBERS)
                    || p.contains(serenity::model::permissions::Permissions::KICK_MEMBERS)
                    || p.contains(serenity::model::permissions::Permissions::BAN_MEMBERS)
                    || p.contains(serenity::model::permissions::Permissions::MANAGE_MESSAGES)
                {
                    is_discord_mod = true;
                }
            }
            for rid in &role_ids {
                if let Some(role) = guild.roles.get(&serenity::model::id::RoleId::new(*rid)) {
                    member_role_names.push(role.name.clone());
                    let p = role.permissions;
                    if p.contains(serenity::model::permissions::Permissions::ADMINISTRATOR)
                        || p.contains(serenity::model::permissions::Permissions::MANAGE_GUILD)
                    {
                        is_discord_admin = true;
                    }
                    if p.contains(serenity::model::permissions::Permissions::MODERATE_MEMBERS)
                        || p.contains(serenity::model::permissions::Permissions::KICK_MEMBERS)
                        || p.contains(serenity::model::permissions::Permissions::BAN_MEMBERS)
                        || p.contains(serenity::model::permissions::Permissions::MANAGE_MESSAGES)
                    {
                        is_discord_mod = true;
                    }
                }
            }
        }
    }

    // 5. Check GuildConfig Bot Admin Roles / Users (`change_permission_roles` / `change_permission_users`)
    let guild_cfg = state.storage.get_guild_config(guild_id);
    let member_ctx = MemberContext {
        user_id,
        is_guild_owner,
        is_administrator: is_discord_admin,
        role_ids: &role_ids,
        role_names: &member_role_names,
    };
    let is_guild_bot_admin = has_guild_permission(&member_ctx, &guild_cfg, PermissionDomain::ManageBot);

    if is_discord_admin || is_guild_bot_admin {
        return GuildAccessProfile::full("admin");
    }

    // 6. Build domain permissions from Moderator defaults + Delegated Web Access Grants
    let mut domain_levels: HashMap<ConsoleDomain, AccessLevel> =
        GUILD_DOMAINS.into_iter().map(|d| (d, AccessLevel::None)).collect();

    if is_discord_mod {
        domain_levels.insert(ConsoleDomain::Moderation, AccessLevel::Edit);
        domain_levels.insert(ConsoleDomain::Automod, AccessLevel::View);
        domain_levels.insert(ConsoleDomain::Logging, AccessLevel::View);
        domain_levels.insert(ConsoleDomain::Activity, AccessLevel::View);
        domain_levels.insert(ConsoleDomain::AuditLog, AccessLevel::View);
    }

    if let Ok(grants) = state.storage.get_web_access_grants(guild_id) {
        for grant in grants {
            let matches_principal = (grant.target_type == "user" && grant.target_id == user_id)
                || (grant.target_type == "role"
                    && (role_ids.contains(&grant.target_id) || grant.target_id == guild_id));
            if !matches_principal {
                continue;
            }
            let level = AccessLevel::from_str_relaxed(&grant.access_level).unwrap_or(AccessLevel::None);
            if let Some(dom) = ConsoleDomain::from_str_relaxed(&grant.domain) {
                if dom == ConsoleDomain::All {
                    for d in GUILD_DOMAINS {
                        let cur = domain_levels.get(&d).copied().unwrap_or(AccessLevel::None);
                        if level > cur {
                            domain_levels.insert(d, level);
                        }
                    }
                } else if GUILD_DOMAINS.contains(&dom) {
                    let cur = domain_levels.get(&dom).copied().unwrap_or(AccessLevel::None);
                    if level > cur {
                        domain_levels.insert(dom, level);
                    }
                }
            }
        }
    }

    let can_access = domain_levels.values().any(|lvl| lvl.can_view());
    if !can_access {
        return GuildAccessProfile::none();
    }

    let is_delegated_admin = domain_levels
        .get(&ConsoleDomain::Settings)
        .copied()
        .unwrap_or(AccessLevel::None)
        .can_edit()
        && domain_levels
            .get(&ConsoleDomain::Permissions)
            .copied()
            .unwrap_or(AccessLevel::None)
            .can_edit();

    let role = if is_delegated_admin {
        "admin"
    } else if is_discord_mod {
        "moderator"
    } else {
        "delegated"
    };

    let mut domains_str = HashMap::with_capacity(GUILD_DOMAINS.len());
    for d in GUILD_DOMAINS {
        let lvl = domain_levels.get(&d).copied().unwrap_or(AccessLevel::None);
        domains_str.insert(d.as_str(), lvl.as_str());
    }

    GuildAccessProfile {
        role,
        can_access: true,
        domains: domains_str,
    }
}

/// Helper to evaluate if caller has any console access to a specific guild (used for shared entity lookups).
async fn user_has_guild_access(
    state: &WebState,
    guild_id: u64,
    user_id: u64,
    claims: &Option<SessionClaims>,
) -> bool {
    resolve_guild_access(state, guild_id, user_id, claims)
        .await
        .can_access
}

/// Enforces that the caller has at least `required` access level for `domain` in `guild_id`.
/// Returns `Ok((claims, user_id))` or an `Err(Response)` (`404` if no access to guild, `403` if insufficient domain role).
#[allow(clippy::result_large_err)]
async fn check_guild_domain_access(
    state: &WebState,
    headers: &HeaderMap,
    guild_id: u64,
    domain: ConsoleDomain,
    required: AccessLevel,
) -> Result<(Option<SessionClaims>, u64), Response> {
    check_guild_any_domain_access(state, headers, guild_id, &[domain], required).await
}

/// Enforces that the caller has at least `required` access level for any of `domains` in `guild_id`.
#[allow(clippy::result_large_err)]
async fn check_guild_any_domain_access(
    state: &WebState,
    headers: &HeaderMap,
    guild_id: u64,
    domains: &[ConsoleDomain],
    required: AccessLevel,
) -> Result<(Option<SessionClaims>, u64), Response> {
    let settings = state.storage.get_app_settings();
    let claims = state.auth.authenticate_request(headers, &settings);
    let user_id = claims
        .as_ref()
        .and_then(|c| c.user_id.parse::<u64>().ok())
        .unwrap_or(0);
    let profile = resolve_guild_access(state, guild_id, user_id, &claims).await;
    if !profile.can_access {
        return Err((StatusCode::NOT_FOUND, Json(json!({ "error": "Not Found" }))).into_response());
    }
    if !domains.iter().any(|&d| profile.has_domain_access(d, required)) {
        let req_str = if required.can_edit() { "edit" } else { "view" };
        return Err((
            StatusCode::FORBIDDEN,
            Json(json!({
                "error": format!(
                    "Insufficient permissions ({} role does not have '{}' access to this module).",
                    profile.role, req_str
                )
            })),
        )
            .into_response());
    }
    Ok((claims, user_id))
}

async fn api_get_guild_config(
    State(state): State<Arc<WebState>>,
    headers: HeaderMap,
    Path(guild_id_str): Path<String>,
) -> Response {
    let Ok(guild_id) = guild_id_str.parse::<u64>() else {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "Invalid guild ID." })),
        )
            .into_response();
    };

    if let Err(resp) = check_guild_domain_access(
        &state,
        &headers,
        guild_id,
        ConsoleDomain::Settings,
        AccessLevel::View,
    )
    .await
    {
        return resp;
    }

    let cfg = state.storage.get_guild_config(guild_id);
    let meta = state.bot.guild_metadata(guild_id);
    let default_avatar_url = state.bot.bot_identity().avatar_url;
    let tq = &cfg.token_quotas;
    let (win_start, win_end, reset_epoch) = calculate_window_period(
        &tq.reset_frequency,
        &tq.reset_time,
        tq.reset_timezone_offset,
        tq.reset_day_of_week,
        tq.reset_day_of_month,
        None,
    );

    Json(json!({
        "config": *cfg,
        "metadata": meta,
        "default_avatar_url": default_avatar_url,
        "quota_window": {
            "window_start": win_start.to_rfc3339(),
            "window_end": win_end.to_rfc3339(),
            "reset_epoch": reset_epoch,
            "frequency": tq.reset_frequency,
        }
    }))
    .into_response()
}

#[derive(Deserialize)]
struct GuildConfigWebUpdate {
    #[serde(default)]
    bot_nickname: Option<String>,
    #[serde(default)]
    bot_avatar: Option<String>,
    #[serde(flatten)]
    patch: GuildConfigPatch,
}

async fn api_put_guild_config(
    State(state): State<Arc<WebState>>,
    headers: HeaderMap,
    Path(guild_id_str): Path<String>,
    Json(update): Json<GuildConfigWebUpdate>,
) -> Response {
    let Ok(guild_id) = guild_id_str.parse::<u64>() else {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "Invalid guild ID." })),
        )
            .into_response();
    };

    let (claims, user_id) = match check_guild_domain_access(
        &state,
        &headers,
        guild_id,
        ConsoleDomain::Settings,
        AccessLevel::Edit,
    )
    .await
    {
        Ok(v) => v,
        Err(resp) => return resp,
    };

    let current = state.storage.get_guild_config(guild_id);
    let before_json = serde_json::to_string(&*current).unwrap_or_default();

    if let Some(nick) = update.bot_nickname.as_deref() {
        let _ = state.bot.update_guild_nickname(guild_id, Some(nick)).await;
    }

    if let Some(avatar) = update.bot_avatar.as_deref() {
        let clean = avatar.trim();
        let target_avatar = if clean.is_empty()
            || clean.eq_ignore_ascii_case("reset")
            || clean.eq_ignore_ascii_case("clear")
            || clean.eq_ignore_ascii_case("default")
        {
            None
        } else if clean.starts_with("data:image/") {
            Some(clean.to_string())
        } else if clean.starts_with("http://") || clean.starts_with("https://") {
            match silas_discord::bot::download_image_as_data_uri(clean).await {
                Ok(uri) => Some(uri),
                Err(e) => {
                    return (
                        StatusCode::BAD_REQUEST,
                        Json(json!({ "error": format!("Failed to download avatar image: {e}") })),
                    )
                        .into_response();
                }
            }
        } else {
            None
        };

        if target_avatar.is_some()
            || clean.eq_ignore_ascii_case("reset")
            || clean.eq_ignore_ascii_case("clear")
            || clean.eq_ignore_ascii_case("default")
        {
            let _ = state
                .bot
                .update_guild_avatar(guild_id, target_avatar.as_deref())
                .await;
        }
    }

    let updated = match state.storage.patch_guild_config(guild_id, update.patch) {
        Ok(u) => u,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": format!("{e}") })),
            )
                .into_response();
        }
    };
    let after_json = serde_json::to_string(&*updated).unwrap_or_default();

    // Audit log
    if let Some(ref c) = claims {
        let ip_hdr = headers
            .get("x-forwarded-for")
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.split(',').next())
            .map(str::trim)
            .unwrap_or("127.0.0.1");

        let audit = NewWebAuditLog {
            guild_id: Some(guild_id),
            user_id,
            user_name: &c.username,
            domain: ConsoleDomain::Settings.as_str(),
            action: "guild_config_update",
            before_json: Some(&before_json),
            after_json: Some(&after_json),
            ip_address: ip_hdr,
            status: "success",
        };
        let _ = state.storage.insert_web_audit_log(&audit);
    }

    Json(json!({
        "ok": true,
        "config": *updated,
    }))
    .into_response()
}

#[derive(Deserialize)]
struct SetGuildIconPayload {
    #[serde(default)]
    icon: Option<String>,
}

async fn api_set_guild_icon(
    State(state): State<Arc<WebState>>,
    headers: HeaderMap,
    Path(guild_id_str): Path<String>,
    Json(payload): Json<SetGuildIconPayload>,
) -> Response {
    let Ok(guild_id) = guild_id_str.parse::<u64>() else {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "Invalid guild ID." })),
        )
            .into_response();
    };

    let (claims, user_id) = match check_guild_domain_access(
        &state,
        &headers,
        guild_id,
        ConsoleDomain::Settings,
        AccessLevel::Edit,
    )
    .await
    {
        Ok(v) => v,
        Err(resp) => return resp,
    };

    let target_data_uri = match payload.icon.as_deref().map(str::trim) {
        None => None,
        Some(s)
            if s.is_empty()
                || s.eq_ignore_ascii_case("reset")
                || s.eq_ignore_ascii_case("clear")
                || s.eq_ignore_ascii_case("default") =>
        {
            None
        }
        Some(s) if s.starts_with("data:image/") => Some(s.to_string()),
        Some(s) if s.starts_with("http://") || s.starts_with("https://") => {
            match silas_discord::bot::download_image_as_data_uri(s).await {
                Ok(uri) => Some(uri),
                Err(e) => {
                    return (
                        StatusCode::BAD_REQUEST,
                        Json(json!({ "error": format!("Failed to download image: {e}") })),
                    )
                        .into_response();
                }
            }
        }
        Some(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({
                    "error": "Invalid image format. Provide an image URL, base64 data URI, or 'reset'."
                })),
            )
                .into_response();
        }
    };

    match state.bot.update_guild_avatar(guild_id, target_data_uri.as_deref()).await {
        Ok(new_avatar_url) => {
            if let Some(ref c) = claims {
                let ip_hdr = headers
                    .get("x-forwarded-for")
                    .and_then(|v| v.to_str().ok())
                    .and_then(|s| s.split(',').next())
                    .map(str::trim)
                    .unwrap_or("127.0.0.1");

                let audit = NewWebAuditLog {
                    guild_id: Some(guild_id),
                    user_id,
                    user_name: &c.username,
                    domain: ConsoleDomain::Settings.as_str(),
                    action: "guild_bot_avatar_update",
                    before_json: None,
                    after_json: Some(&json!({ "avatar_url": new_avatar_url }).to_string()),
                    ip_address: ip_hdr,
                    status: "success",
                };
                let _ = state.storage.insert_web_audit_log(&audit);
            }

            Json(json!({
                "ok": true,
                "avatar_url": new_avatar_url,
            }))
            .into_response()
        }
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": e })),
        )
            .into_response(),
    }
}


// ============================================================================
// Guild Entity Lookups (Roles, Channels, Emojis, Members) via Serenity Cache/HTTP
// ============================================================================

async fn api_get_guild_roles(
    State(state): State<Arc<WebState>>,
    headers: HeaderMap,
    Path(guild_id_str): Path<String>,
) -> Response {
    let Ok(guild_id) = guild_id_str.parse::<u64>() else {
        return (StatusCode::BAD_REQUEST, Json(json!({ "error": "Invalid guild ID." }))).into_response();
    };

    let settings = state.storage.get_app_settings();
    let claims = state.auth.authenticate_request(&headers, &settings);
    let user_id = claims.as_ref().and_then(|c| c.user_id.parse::<u64>().ok()).unwrap_or(0);
    if !user_has_guild_access(&state, guild_id, user_id, &claims).await {
        return (StatusCode::NOT_FOUND, Json(json!({ "error": "Not Found" }))).into_response();
    }

    let mut roles_list = Vec::new();
    if let Some(cache) = state.bot.cache() {
        if let Some(guild) = cache.guild(guild_id) {
            for role in guild.roles.values() {
                roles_list.push(json!({
                    "id": role.id.to_string(),
                    "name": role.name,
                    "color": role.colour.hex(),
                    "position": role.position,
                    "managed": role.managed,
                }));
            }
        }
    }

    roles_list.sort_by(|a, b| {
        let pos_b = b.get("position").and_then(Value::as_u64).unwrap_or(0);
        let pos_a = a.get("position").and_then(Value::as_u64).unwrap_or(0);
        pos_b.cmp(&pos_a)
    });

    Json(json!({ "roles": roles_list })).into_response()
}

async fn api_get_guild_channels(
    State(state): State<Arc<WebState>>,
    headers: HeaderMap,
    Path(guild_id_str): Path<String>,
) -> Response {
    let Ok(guild_id) = guild_id_str.parse::<u64>() else {
        return (StatusCode::BAD_REQUEST, Json(json!({ "error": "Invalid guild ID." }))).into_response();
    };

    let settings = state.storage.get_app_settings();
    let claims = state.auth.authenticate_request(&headers, &settings);
    let user_id = claims.as_ref().and_then(|c| c.user_id.parse::<u64>().ok()).unwrap_or(0);
    if !user_has_guild_access(&state, guild_id, user_id, &claims).await {
        return (StatusCode::NOT_FOUND, Json(json!({ "error": "Not Found" }))).into_response();
    }

    let mut channels_list = Vec::new();
    if let Some(cache) = state.bot.cache() {
        if let Some(guild) = cache.guild(guild_id) {
            for ch in guild.channels.values() {
                let kind_str = match ch.kind {
                    serenity::model::channel::ChannelType::Text => "text",
                    serenity::model::channel::ChannelType::Voice => "voice",
                    serenity::model::channel::ChannelType::Category => "category",
                    serenity::model::channel::ChannelType::News => "announcement",
                    serenity::model::channel::ChannelType::Stage => "stage",
                    serenity::model::channel::ChannelType::Forum => "forum",
                    serenity::model::channel::ChannelType::PublicThread
                    | serenity::model::channel::ChannelType::PrivateThread
                    | serenity::model::channel::ChannelType::NewsThread => "thread",
                    _ => "other",
                };
                channels_list.push(json!({
                    "id": ch.id.to_string(),
                    "name": ch.name,
                    "kind": kind_str,
                    "parent_id": ch.parent_id.map(|p| p.to_string()),
                    "position": ch.position,
                }));
            }
        }
    }

    channels_list.sort_by(|a, b| {
        let pos_a = a.get("position").and_then(Value::as_u64).unwrap_or(0);
        let pos_b = b.get("position").and_then(Value::as_u64).unwrap_or(0);
        pos_a.cmp(&pos_b)
    });

    Json(json!({ "channels": channels_list })).into_response()
}

async fn api_get_guild_emojis(
    State(state): State<Arc<WebState>>,
    headers: HeaderMap,
    Path(guild_id_str): Path<String>,
) -> Response {
    let Ok(guild_id) = guild_id_str.parse::<u64>() else {
        return (StatusCode::BAD_REQUEST, Json(json!({ "error": "Invalid guild ID." }))).into_response();
    };

    let settings = state.storage.get_app_settings();
    let claims = state.auth.authenticate_request(&headers, &settings);
    let user_id = claims.as_ref().and_then(|c| c.user_id.parse::<u64>().ok()).unwrap_or(0);
    if !user_has_guild_access(&state, guild_id, user_id, &claims).await {
        return (StatusCode::NOT_FOUND, Json(json!({ "error": "Not Found" }))).into_response();
    }

    let mut emojis_list = Vec::new();
    if let Some(cache) = state.bot.cache() {
        if let Some(guild) = cache.guild(guild_id) {
            for em in guild.emojis.values() {
                emojis_list.push(json!({
                    "id": em.id.to_string(),
                    "name": em.name,
                    "animated": em.animated,
                    "available": em.available,
                    "url": em.url(),
                }));
            }
        }
    }

    emojis_list.sort_by(|a, b| {
        let name_a = a.get("name").and_then(Value::as_str).unwrap_or("");
        let name_b = b.get("name").and_then(Value::as_str).unwrap_or("");
        name_a.cmp(name_b)
    });

    Json(json!({ "emojis": emojis_list })).into_response()
}

#[derive(Deserialize)]
struct MemberSearchQuery {
    q: Option<String>,
}

async fn api_search_guild_members(
    State(state): State<Arc<WebState>>,
    headers: HeaderMap,
    Path(guild_id_str): Path<String>,
    Query(q): Query<MemberSearchQuery>,
) -> Response {
    let Ok(guild_id) = guild_id_str.parse::<u64>() else {
        return (StatusCode::BAD_REQUEST, Json(json!({ "error": "Invalid guild ID." }))).into_response();
    };

    let settings = state.storage.get_app_settings();
    let claims = state.auth.authenticate_request(&headers, &settings);
    let user_id = claims.as_ref().and_then(|c| c.user_id.parse::<u64>().ok()).unwrap_or(0);
    if !user_has_guild_access(&state, guild_id, user_id, &claims).await {
        return (StatusCode::NOT_FOUND, Json(json!({ "error": "Not Found" }))).into_response();
    }

    let query_str = q.q.as_deref().unwrap_or("").trim();
    if query_str.len() < 2 {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "Search query must be at least 2 characters." })),
        )
            .into_response();
    }

    let Some(http) = state.bot.http() else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({ "error": "Discord client is not connected." })),
        )
            .into_response();
    };

    match http
        .search_guild_members(serenity::model::id::GuildId::new(guild_id), query_str, Some(25))
        .await
    {
        Ok(members) => {
            let list: Vec<Value> = members
                .into_iter()
                .map(|m| {
                    json!({
                        "id": m.user.id.to_string(),
                        "username": m.user.name,
                        "global_name": m.user.global_name,
                        "nick": m.nick,
                        "avatar_url": m.avatar_url().or_else(|| Some(m.user.face())),
                    })
                })
                .collect();
            Json(json!({ "members": list })).into_response()
        }
        Err(e) => (
            StatusCode::BAD_GATEWAY,
            Json(json!({ "error": format!("Discord search failed: {e}") })),
        )
            .into_response(),
    }
}

// ============================================================================
// Access & Grants Endpoints (Feature F1)
// ============================================================================

async fn api_list_guild_grants(
    State(state): State<Arc<WebState>>,
    headers: HeaderMap,
    Path(guild_id_str): Path<String>,
) -> Response {
    let Ok(guild_id) = guild_id_str.parse::<u64>() else {
        return (StatusCode::BAD_REQUEST, Json(json!({ "error": "Invalid guild ID." }))).into_response();
    };

    if let Err(resp) = check_guild_domain_access(
        &state,
        &headers,
        guild_id,
        ConsoleDomain::Permissions,
        AccessLevel::View,
    )
    .await
    {
        return resp;
    }

    match state.storage.get_web_access_grants(guild_id) {
        Ok(grants) => Json(json!({ "grants": grants })).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e.to_string() }))).into_response(),
    }
}

#[derive(Deserialize)]
struct UpsertGrantRequest {
    #[serde(alias = "principal_type")]
    target_type: String,
    #[serde(alias = "principal_id", with = "silas_core::snowflake")]
    target_id: u64,
    #[serde(default, alias = "principal_name")]
    target_name: Option<String>,
    domain: String,
    access_level: String,
}

async fn api_upsert_guild_grant(
    State(state): State<Arc<WebState>>,
    headers: HeaderMap,
    Path(guild_id_str): Path<String>,
    Json(payload): Json<UpsertGrantRequest>,
) -> Response {
    let Ok(guild_id) = guild_id_str.parse::<u64>() else {
        return (StatusCode::BAD_REQUEST, Json(json!({ "error": "Invalid guild ID." }))).into_response();
    };

    let (claims, user_id) = match check_guild_domain_access(
        &state,
        &headers,
        guild_id,
        ConsoleDomain::Permissions,
        AccessLevel::Edit,
    )
    .await
    {
        Ok(v) => v,
        Err(resp) => return resp,
    };

    if payload.target_type != "role" && payload.target_type != "user" {
        return (StatusCode::BAD_REQUEST, Json(json!({ "error": "target_type must be 'role' or 'user'." }))).into_response();
    }

    let target_name = payload.target_name.as_deref().unwrap_or("Unknown Target");

    match state.storage.create_or_update_web_access_grant(
        guild_id,
        &payload.target_type,
        payload.target_id,
        target_name,
        &payload.domain,
        &payload.access_level,
    ) {
        Ok(saved) => {
            // Audit log
            if let Some(ref c) = claims {
                let ip_hdr = headers
                    .get("x-forwarded-for")
                    .and_then(|v| v.to_str().ok())
                    .and_then(|s| s.split(',').next())
                    .map(str::trim)
                    .unwrap_or("127.0.0.1");

                let audit = NewWebAuditLog {
                    guild_id: Some(guild_id),
                    user_id,
                    user_name: &c.username,
                    domain: ConsoleDomain::Permissions.as_str(),
                    action: "grant_upsert",
                    before_json: None,
                    after_json: Some(&serde_json::to_string(&saved).unwrap_or_default()),
                    ip_address: ip_hdr,
                    status: "success",
                };
                let _ = state.storage.insert_web_audit_log(&audit);
            }
            (StatusCode::CREATED, Json(json!({ "grant": saved }))).into_response()
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e.to_string() }))).into_response(),
    }
}

async fn api_delete_guild_grant(
    State(state): State<Arc<WebState>>,
    headers: HeaderMap,
    Path((guild_id_str, grant_id_str)): Path<(String, String)>,
) -> Response {
    let Ok(guild_id) = guild_id_str.parse::<u64>() else {
        return (StatusCode::BAD_REQUEST, Json(json!({ "error": "Invalid guild ID." }))).into_response();
    };
    let Ok(grant_id) = grant_id_str.parse::<i64>() else {
        return (StatusCode::BAD_REQUEST, Json(json!({ "error": "Invalid grant ID." }))).into_response();
    };

    let (claims, user_id) = match check_guild_domain_access(
        &state,
        &headers,
        guild_id,
        ConsoleDomain::Permissions,
        AccessLevel::Edit,
    )
    .await
    {
        Ok(v) => v,
        Err(resp) => return resp,
    };

    match state.storage.delete_web_access_grant(guild_id, grant_id) {
        Ok(true) => {
            // Audit log
            if let Some(ref c) = claims {
                let ip_hdr = headers
                    .get("x-forwarded-for")
                    .and_then(|v| v.to_str().ok())
                    .and_then(|s| s.split(',').next())
                    .map(str::trim)
                    .unwrap_or("127.0.0.1");

                let audit = NewWebAuditLog {
                    guild_id: Some(guild_id),
                    user_id,
                    user_name: &c.username,
                    domain: ConsoleDomain::Permissions.as_str(),
                    action: "grant_delete",
                    before_json: Some(&format!("{{\"grant_id\": {grant_id}}}")),
                    after_json: None,
                    ip_address: ip_hdr,
                    status: "success",
                };
                let _ = state.storage.insert_web_audit_log(&audit);
            }
            Json(json!({ "ok": true })).into_response()
        }
        Ok(false) => (StatusCode::NOT_FOUND, Json(json!({ "error": "Grant not found." }))).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e.to_string() }))).into_response(),
    }
}

// ============================================================================
// Audit Log Query Endpoints (Feature F2)
// ============================================================================

#[derive(Deserialize)]
struct AuditLogQuery {
    domain: Option<String>,
    user_id: Option<u64>,
    limit: Option<i64>,
    offset: Option<i64>,
}

async fn api_get_guild_audit_log(
    State(state): State<Arc<WebState>>,
    headers: HeaderMap,
    Path(guild_id_str): Path<String>,
    Query(q): Query<AuditLogQuery>,
) -> Response {
    let Ok(guild_id) = guild_id_str.parse::<u64>() else {
        return (StatusCode::BAD_REQUEST, Json(json!({ "error": "Invalid guild ID." }))).into_response();
    };

    if let Err(resp) = check_guild_domain_access(
        &state,
        &headers,
        guild_id,
        ConsoleDomain::AuditLog,
        AccessLevel::View,
    )
    .await
    {
        return resp;
    }

    let limit = q.limit.unwrap_or(50).clamp(1, 100) as usize;
    let offset = q.offset.unwrap_or(0).max(0) as usize;

    match state.storage.list_web_audit_logs(
        Some(guild_id),
        q.user_id,
        q.domain.as_deref(),
        limit,
        offset,
    ) {
        Ok(logs) => Json(json!({ "logs": logs, "limit": limit, "offset": offset })).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e.to_string() }))).into_response(),
    }
}

async fn api_get_global_audit_log(
    State(state): State<Arc<WebState>>,
    headers: HeaderMap,
    Query(q): Query<AuditLogQuery>,
) -> Response {
    let settings = state.storage.get_app_settings();
    let claims = state.auth.authenticate_request(&headers, &settings);
    let is_owner = match &claims {
        Some(c) => c.is_owner || state.auth.is_bot_owner(&c.user_id, &settings),
        None => false,
    };

    if !is_owner {
        return (StatusCode::NOT_FOUND, Json(json!({ "error": "Not Found" }))).into_response();
    }

    let limit = q.limit.unwrap_or(50).clamp(1, 100) as usize;
    let offset = q.offset.unwrap_or(0).max(0) as usize;

    match state.storage.list_web_audit_logs(
        None,
        q.user_id,
        q.domain.as_deref(),
        limit,
        offset,
    ) {
        Ok(logs) => Json(json!({ "logs": logs, "limit": limit, "offset": offset })).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e.to_string() }))).into_response(),
    }
}

async fn api_reset_guild_quota(
    State(state): State<Arc<WebState>>,
    headers: HeaderMap,
    Path(guild_id_str): Path<String>,
) -> Response {
    let Ok(guild_id) = guild_id_str.parse::<u64>() else {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "Invalid guild ID." })),
        )
            .into_response();
    };

    let (claims, user_id) = match check_guild_domain_access(
        &state,
        &headers,
        guild_id,
        ConsoleDomain::Settings,
        AccessLevel::Edit,
    )
    .await
    {
        Ok(v) => v,
        Err(resp) => return resp,
    };

    let cfg = state.storage.get_guild_config(guild_id);
    let tq = &cfg.token_quotas;
    let (win_start, _, _) = calculate_window_period(
        &tq.reset_frequency,
        &tq.reset_time,
        tq.reset_timezone_offset,
        tq.reset_day_of_week,
        tq.reset_day_of_month,
        None,
    );

    match state.storage.reset_guild_token_usage(guild_id, win_start) {
        Ok(deleted) => {
            if let Some(ref c) = claims {
                let ip_hdr = headers
                    .get("x-forwarded-for")
                    .and_then(|v| v.to_str().ok())
                    .and_then(|s| s.split(',').next())
                    .map(str::trim)
                    .unwrap_or("127.0.0.1");

                let audit = NewWebAuditLog {
                    guild_id: Some(guild_id),
                    user_id,
                    user_name: &c.username,
                    domain: ConsoleDomain::Settings.as_str(),
                    action: "quota_reset",
                    before_json: None,
                    after_json: Some(&format!("{{\"deleted_entries\": {deleted}}}")),
                    ip_address: ip_hdr,
                    status: "success",
                };
                let _ = state.storage.insert_web_audit_log(&audit);
            }
            Json(json!({ "ok": true, "deleted_entries": deleted })).into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

async fn api_clear_guild_memory(
    State(state): State<Arc<WebState>>,
    headers: HeaderMap,
    Path(guild_id_str): Path<String>,
) -> Response {
    let Ok(guild_id) = guild_id_str.parse::<u64>() else {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "Invalid guild ID." })),
        )
            .into_response();
    };

    let (claims, user_id) = match check_guild_domain_access(
        &state,
        &headers,
        guild_id,
        ConsoleDomain::Settings,
        AccessLevel::Edit,
    )
    .await
    {
        Ok(v) => v,
        Err(resp) => return resp,
    };

    state.bot.memory().clear_guild(guild_id);

    if let Some(ref c) = claims {
        let ip_hdr = headers
            .get("x-forwarded-for")
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.split(',').next())
            .map(str::trim)
            .unwrap_or("127.0.0.1");

        let audit = NewWebAuditLog {
            guild_id: Some(guild_id),
            user_id,
            user_name: &c.username,
            domain: ConsoleDomain::Ai.as_str(),
            action: "memory_clear",
            before_json: None,
            after_json: None,
            ip_address: ip_hdr,
            status: "success",
        };
        let _ = state.storage.insert_web_audit_log(&audit);
    }

    Json(json!({ "ok": true })).into_response()
}

async fn api_update_check(
    State(state): State<Arc<WebState>>,
    headers: HeaderMap,
    Query(params): Query<HashMap<String, String>>,
) -> Response {
    let settings = state.storage.get_app_settings();
    let claims = state.auth.authenticate_request(&headers, &settings);
    let is_owner = match &claims {
        Some(c) => c.is_owner || state.auth.is_bot_owner(&c.user_id, &settings),
        None => false,
    };
    if !is_owner {
        return (StatusCode::NOT_FOUND, Json(json!({ "error": "Not Found" }))).into_response();
    }

    let channel = params.get("channel").map(|s| s.as_str());
    let res = state.updater.check_for_updates(channel).await;
    Json(json!(res)).into_response()
}

#[derive(Deserialize)]
struct UpdateApplyPayload {
    channel: Option<String>,
}

async fn api_update_apply(
    State(state): State<Arc<WebState>>,
    headers: HeaderMap,
    Json(payload): Json<UpdateApplyPayload>,
) -> Response {
    let settings = state.storage.get_app_settings();
    let claims = state.auth.authenticate_request(&headers, &settings);
    let is_owner = match &claims {
        Some(c) => c.is_owner || state.auth.is_bot_owner(&c.user_id, &settings),
        None => false,
    };
    if !is_owner {
        return (StatusCode::NOT_FOUND, Json(json!({ "error": "Not Found" }))).into_response();
    }

    match state
        .updater
        .apply_update(payload.channel.as_deref())
        .await
    {
        Ok(msg) => Json(json!({ "ok": true, "message": msg })).into_response(),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(json!({ "ok": false, "error": e.to_string() })),
        )
            .into_response(),
    }
}

async fn api_update_snapshots(
    State(state): State<Arc<WebState>>,
    headers: HeaderMap,
) -> Response {
    let settings = state.storage.get_app_settings();
    let claims = state.auth.authenticate_request(&headers, &settings);
    let is_owner = match &claims {
        Some(c) => c.is_owner || state.auth.is_bot_owner(&c.user_id, &settings),
        None => false,
    };
    if !is_owner {
        return (StatusCode::NOT_FOUND, Json(json!({ "error": "Not Found" }))).into_response();
    }

    let snapshots = state.updater.list_snapshots().unwrap_or_default();
    Json(json!({ "snapshots": snapshots })).into_response()
}

async fn api_create_snapshot(
    State(state): State<Arc<WebState>>,
    headers: HeaderMap,
) -> Response {
    let settings = state.storage.get_app_settings();
    let claims = state.auth.authenticate_request(&headers, &settings);
    let is_owner = match &claims {
        Some(c) => c.is_owner || state.auth.is_bot_owner(&c.user_id, &settings),
        None => false,
    };
    if !is_owner {
        return (StatusCode::NOT_FOUND, Json(json!({ "error": "Not Found" }))).into_response();
    }

    match state.updater.create_snapshot("manual").await {
        Ok(snap) => Json(json!({ "ok": true, "snapshot": snap })).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

/// Generate and download a secret-redacted SQLite database backup (`silas-blue-redacted-backup.db`).
async fn api_download_redacted_backup(
    State(state): State<Arc<WebState>>,
    headers: HeaderMap,
) -> Response {
    let settings = state.storage.get_app_settings();
    let claims = state.auth.authenticate_request(&headers, &settings);
    let is_owner = match &claims {
        Some(c) => c.is_owner || state.auth.is_bot_owner(&c.user_id, &settings),
        None => false,
    };
    if !is_owner {
        return (StatusCode::NOT_FOUND, Json(json!({ "error": "Not Found" }))).into_response();
    }

    let tmp_dir = state.updater.backups_dir().to_path_buf();
    let _ = std::fs::create_dir_all(&tmp_dir);
    let dest = tmp_dir.join(format!("redacted_export_{}.db", Utc::now().timestamp_millis()));

    let storage = state.storage.clone();
    let dest_for_task = dest.clone();
    let backup_res =
        tokio::task::spawn_blocking(move || storage.create_redacted_backup(&dest_for_task)).await;

    match backup_res {
        Ok(Ok(_)) => {}
        Ok(Err(e)) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": format!("Failed to create redacted backup: {e}") })),
            )
                .into_response();
        }
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": format!("Backup task error: {e}") })),
            )
                .into_response();
        }
    }

    let bytes = match std::fs::read(&dest) {
        Ok(b) => b,
        Err(e) => {
            let _ = std::fs::remove_file(&dest);
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": format!("Failed to read redacted backup: {e}") })),
            )
                .into_response();
        }
    };
    let _ = std::fs::remove_file(&dest);

    let mut resp = Response::new(Body::from(bytes));
    resp.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/vnd.sqlite3"),
    );
    resp.headers_mut().insert(
        header::CONTENT_DISPOSITION,
        HeaderValue::from_static("attachment; filename=\"silas-blue-redacted-backup.db\""),
    );
    resp
}

// ============================================================================
// Reddit Feeds & Webhook Endpoints
// ============================================================================

async fn api_webhook_reddit(
    State(state): State<Arc<WebState>>,
    headers: HeaderMap,
    Json(post): Json<silas_core::reddit::RedditPost>,
) -> Response {
    let settings = state.storage.get_app_settings();
    let expected_secret = settings.reddit_webhook_secret.trim();
    if expected_secret.is_empty() {
        return (
            StatusCode::FORBIDDEN,
            Json(json!({ "error": "Reddit webhook secret is not configured in Silas Blue settings." })),
        )
            .into_response();
    }

    let provided = headers
        .get("x-silas-reddit-secret")
        .and_then(|h| h.to_str().ok())
        .or_else(|| {
            headers
                .get(header::AUTHORIZATION)
                .and_then(|h| h.to_str().ok())
                .and_then(|h| h.strip_prefix("Bearer "))
        })
        .unwrap_or("")
        .trim();

    use sha2::{Digest, Sha256};
    use subtle::ConstantTimeEq;
    let hash_expected = Sha256::digest(expected_secret.as_bytes());
    let hash_provided = Sha256::digest(provided.as_bytes());
    let valid: bool = hash_expected.ct_eq(&hash_provided).into();

    if !valid {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({ "error": "Invalid webhook authentication secret." })),
        )
            .into_response();
    }

    let Some(http) = state.bot.http() else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({ "error": "Discord gateway client is not connected." })),
        )
            .into_response();
    };

    let delivered =
        silas_discord::dispatch_post_to_matching_subscriptions(&http, &state.storage, &post).await;

    Json(json!({
        "status": "ok",
        "delivered_count": delivered
    }))
    .into_response()
}

async fn api_list_guild_reddit_subs(
    State(state): State<Arc<WebState>>,
    headers: HeaderMap,
    Path(guild_id_str): Path<String>,
) -> Response {
    let Ok(guild_id) = guild_id_str.parse::<u64>() else {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "Invalid guild ID." })),
        )
            .into_response();
    };

    if let Err(resp) = check_guild_any_domain_access(
        &state,
        &headers,
        guild_id,
        &[ConsoleDomain::Reddit, ConsoleDomain::Settings],
        AccessLevel::View,
    )
    .await
    {
        return resp;
    }

    match state.storage.list_reddit_subscriptions_for_guild(guild_id) {
        Ok(subs) => Json(json!({ "subscriptions": subs })).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": format!("Database error: {e}") })),
        )
            .into_response(),
    }
}

async fn api_create_guild_reddit_sub(
    State(state): State<Arc<WebState>>,
    headers: HeaderMap,
    Path(guild_id_str): Path<String>,
    Json(mut new_sub): Json<silas_core::reddit::NewRedditSubscription>,
) -> Response {
    let Ok(guild_id) = guild_id_str.parse::<u64>() else {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "Invalid guild ID." })),
        )
            .into_response();
    };

    let (claims, user_id) = match check_guild_any_domain_access(
        &state,
        &headers,
        guild_id,
        &[ConsoleDomain::Reddit, ConsoleDomain::Settings],
        AccessLevel::Edit,
    )
    .await
    {
        Ok(v) => v,
        Err(resp) => return resp,
    };

    new_sub.guild_id = guild_id;
    let clean_sub = silas_core::reddit::clean_subreddit_name(&new_sub.subreddit);
    if clean_sub.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "Subreddit name cannot be empty." })),
        )
            .into_response();
    }
    new_sub.subreddit = clean_sub.clone();

    let client = silas_discord::RedditClient::new();
    let settings = state.storage.get_app_settings();
    let recent_posts = client
        .fetch_new_posts(&clean_sub, 10, &settings)
        .await
        .unwrap_or_default();

    let newest_id = recent_posts.first().map(|p| p.normalized_id().to_string());

    let created_sub = match state
        .storage
        .create_reddit_subscription(&new_sub, newest_id.as_deref())
    {
        Ok(s) => s,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": format!("Failed to create subscription: {e}") })),
            )
                .into_response();
        }
    };

    let backfill = new_sub.backfill_count.min(5);
    if backfill > 0 {
        if let Some(http) = state.bot.http() {
            let mut matching: Vec<_> = recent_posts
                .into_iter()
                .filter(|p| p.matches_subscription(&created_sub))
                .take(backfill as usize)
                .collect();
            matching.reverse();
            for p in matching {
                let _ =
                    silas_discord::deliver_reddit_post(&http, &state.storage, &p, &created_sub)
                        .await;
            }
        }
    }

    // Audit log
    if let Some(ref c) = claims {
        let ip_hdr = headers
            .get("x-forwarded-for")
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.split(',').next())
            .map(str::trim)
            .unwrap_or("127.0.0.1");

        let audit = NewWebAuditLog {
            guild_id: Some(guild_id),
            user_id,
            user_name: &c.username,
            domain: ConsoleDomain::Settings.as_str(),
            action: "reddit_feed_create",
            before_json: None,
            after_json: Some(&serde_json::to_string(&created_sub).unwrap_or_default()),
            ip_address: ip_hdr,
            status: "success",
        };
        let _ = state.storage.insert_web_audit_log(&audit);
    }

    (
        StatusCode::CREATED,
        Json(json!({ "subscription": created_sub })),
    )
        .into_response()
}

async fn api_delete_guild_reddit_sub(
    State(state): State<Arc<WebState>>,
    headers: HeaderMap,
    Path((guild_id_str, sub_id_str)): Path<(String, String)>,
) -> Response {
    let Ok(guild_id) = guild_id_str.parse::<u64>() else {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "Invalid guild ID." })),
        )
            .into_response();
    };

    let Ok(sub_id) = sub_id_str.parse::<i64>() else {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "Invalid subscription ID." })),
        )
            .into_response();
    };

    let (claims, user_id) = match check_guild_any_domain_access(
        &state,
        &headers,
        guild_id,
        &[ConsoleDomain::Reddit, ConsoleDomain::Settings],
        AccessLevel::Edit,
    )
    .await
    {
        Ok(v) => v,
        Err(resp) => return resp,
    };

    match state.storage.delete_reddit_subscription(sub_id, guild_id) {
        Ok(true) => {
            if let Some(ref c) = claims {
                let ip_hdr = headers
                    .get("x-forwarded-for")
                    .and_then(|v| v.to_str().ok())
                    .and_then(|s| s.split(',').next())
                    .map(str::trim)
                    .unwrap_or("127.0.0.1");

                let audit = NewWebAuditLog {
                    guild_id: Some(guild_id),
                    user_id,
                    user_name: &c.username,
                    domain: ConsoleDomain::Settings.as_str(),
                    action: "reddit_feed_delete",
                    before_json: Some(&format!("{{\"sub_id\": {sub_id}}}")),
                    after_json: None,
                    ip_address: ip_hdr,
                    status: "success",
                };
                let _ = state.storage.insert_web_audit_log(&audit);
            }
            Json(json!({ "success": true })).into_response()
        }
        Ok(false) => (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "Subscription not found." })),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": format!("Database error: {e}") })),
        )
            .into_response(),
    }
}

async fn api_generate_reddit_secret(
    State(state): State<Arc<WebState>>,
    headers: HeaderMap,
) -> Response {
    if !is_request_owner(&state, &headers) {
        return (
            StatusCode::FORBIDDEN,
            Json(json!({ "error": "Only the Bot Owner can generate global webhook secrets." })),
        )
            .into_response();
    }
    let mut bytes = [0u8; 24];
    rand::thread_rng().fill_bytes(&mut bytes);
    let secret = hex::encode(bytes);
    Json(json!({ "secret": secret })).into_response()
}

// ============================================================================
// Moderation, Automod, Logging, Greetings, Reaction Roles & Triggers Endpoints
// ============================================================================

#[derive(Debug, Deserialize)]
pub struct ModCasesQuery {
    pub user_id: Option<u64>,
    pub limit: Option<usize>,
}

async fn api_list_mod_cases(
    State(state): State<Arc<WebState>>,
    headers: HeaderMap,
    Path(guild_id_str): Path<String>,
    Query(query): Query<ModCasesQuery>,
) -> Response {
    let Ok(guild_id) = guild_id_str.parse::<u64>() else {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "Invalid guild ID." })),
        )
            .into_response();
    };

    if let Err(resp) = check_guild_domain_access(
        &state,
        &headers,
        guild_id,
        ConsoleDomain::Moderation,
        AccessLevel::View,
    )
    .await
    {
        return resp;
    }

    let cases = match query.user_id {
        Some(user_id) => state.storage.get_mod_cases_for_user(guild_id, user_id),
        None => state.storage.get_mod_cases_for_guild(guild_id, query.limit),
    };

    match cases {
        Ok(c) => Json(json!({ "cases": c })).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": format!("{e}") })),
        )
            .into_response(),
    }
}

#[derive(Debug, Deserialize)]
pub struct CreateModCaseRequest {
    #[serde(with = "silas_core::snowflake")]
    pub user_id: u64,
    #[serde(default, with = "silas_core::snowflake::opt")]
    pub moderator_id: Option<u64>,
    pub action_type: String,
    pub reason: Option<String>,
    pub duration_seconds: Option<i64>,
    pub expires_at: Option<i64>,
}

async fn api_create_mod_case(
    State(state): State<Arc<WebState>>,
    headers: HeaderMap,
    Path(guild_id_str): Path<String>,
    Json(payload): Json<CreateModCaseRequest>,
) -> Response {
    let Ok(guild_id) = guild_id_str.parse::<u64>() else {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "Invalid guild ID." })),
        )
            .into_response();
    };

    let (claims, user_id) = match check_guild_domain_access(
        &state,
        &headers,
        guild_id,
        ConsoleDomain::Moderation,
        AccessLevel::Edit,
    )
    .await
    {
        Ok(v) => v,
        Err(resp) => return resp,
    };

    let new_case = NewModCase {
        guild_id,
        user_id: payload.user_id,
        moderator_id: payload.moderator_id.unwrap_or(user_id),
        action_type: payload.action_type,
        reason: payload
            .reason
            .unwrap_or_else(|| "No reason provided".to_string()),
        duration_seconds: payload.duration_seconds,
        created_at: None,
        expires_at: payload.expires_at,
    };

    match state.storage.create_mod_case(&new_case) {
        Ok(created) => {
            if let Some(ref c) = claims {
                let ip_hdr = headers
                    .get("x-forwarded-for")
                    .and_then(|v| v.to_str().ok())
                    .and_then(|s| s.split(',').next())
                    .map(str::trim)
                    .unwrap_or("127.0.0.1");

                let audit = NewWebAuditLog {
                    guild_id: Some(guild_id),
                    user_id,
                    user_name: &c.username,
                    domain: ConsoleDomain::Moderation.as_str(),
                    action: "mod_case_create",
                    before_json: None,
                    after_json: Some(&serde_json::to_string(&created).unwrap_or_default()),
                    ip_address: ip_hdr,
                    status: "success",
                };
                let _ = state.storage.insert_web_audit_log(&audit);
            }
            (StatusCode::CREATED, Json(json!({ "case": created }))).into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": format!("{e}") })),
        )
            .into_response(),
    }
}

#[derive(Debug, Deserialize)]
pub struct UpdateCaseReasonRequest {
    pub reason: String,
}

async fn api_update_mod_case_reason(
    State(state): State<Arc<WebState>>,
    headers: HeaderMap,
    Path((guild_id_str, case_id_str)): Path<(String, String)>,
    Json(payload): Json<UpdateCaseReasonRequest>,
) -> Response {
    let Ok(guild_id) = guild_id_str.parse::<u64>() else {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "Invalid guild ID." })),
        )
            .into_response();
    };
    let Ok(case_id) = case_id_str.parse::<i64>() else {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "Invalid case ID." })),
        )
            .into_response();
    };

    let (claims, user_id) = match check_guild_domain_access(
        &state,
        &headers,
        guild_id,
        ConsoleDomain::Moderation,
        AccessLevel::Edit,
    )
    .await
    {
        Ok(v) => v,
        Err(resp) => return resp,
    };

    // Verify case belongs to guild
    if let Ok(Some(existing_case)) = state.storage.get_mod_case(guild_id, case_id) {
        if existing_case.guild_id != guild_id {
            return (StatusCode::NOT_FOUND, Json(json!({ "error": "Case not found." }))).into_response();
        }
    } else {
        return (StatusCode::NOT_FOUND, Json(json!({ "error": "Case not found." }))).into_response();
    }

    let success = state
        .storage
        .update_case_reason(guild_id, case_id, &payload.reason)
        .unwrap_or(false);

    if success {
        if let Some(ref c) = claims {
            let ip_hdr = headers
                .get("x-forwarded-for")
                .and_then(|v| v.to_str().ok())
                .and_then(|s| s.split(',').next())
                .map(str::trim)
                .unwrap_or("127.0.0.1");

            let audit = NewWebAuditLog {
                guild_id: Some(guild_id),
                user_id,
                user_name: &c.username,
                domain: ConsoleDomain::Moderation.as_str(),
                action: "mod_case_reason_update",
                before_json: None,
                after_json: Some(&format!("{{\"case_id\": {case_id}, \"reason\": {:?}}}", payload.reason)),
                ip_address: ip_hdr,
                status: "success",
            };
            let _ = state.storage.insert_web_audit_log(&audit);
        }
        Json(json!({ "success": true, "reason": payload.reason })).into_response()
    } else {
        (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "Case not found." })),
        )
            .into_response()
    }
}

async fn api_deactivate_mod_case(
    State(state): State<Arc<WebState>>,
    headers: HeaderMap,
    Path((guild_id_str, case_id_str)): Path<(String, String)>,
) -> Response {
    let Ok(guild_id) = guild_id_str.parse::<u64>() else {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "Invalid guild ID." })),
        )
            .into_response();
    };
    let Ok(case_id) = case_id_str.parse::<i64>() else {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "Invalid case ID." })),
        )
            .into_response();
    };

    let (claims, user_id) = match check_guild_domain_access(
        &state,
        &headers,
        guild_id,
        ConsoleDomain::Moderation,
        AccessLevel::Edit,
    )
    .await
    {
        Ok(v) => v,
        Err(resp) => return resp,
    };

    // Verify case belongs to guild
    if let Ok(Some(existing_case)) = state.storage.get_mod_case(guild_id, case_id) {
        if existing_case.guild_id != guild_id {
            return (StatusCode::NOT_FOUND, Json(json!({ "error": "Case not found." }))).into_response();
        }
    } else {
        return (StatusCode::NOT_FOUND, Json(json!({ "error": "Case not found." }))).into_response();
    }

    match state.storage.deactivate_case(case_id) {
        Ok(true) => {
            if let Some(ref c) = claims {
                let ip_hdr = headers
                    .get("x-forwarded-for")
                    .and_then(|v| v.to_str().ok())
                    .and_then(|s| s.split(',').next())
                    .map(str::trim)
                    .unwrap_or("127.0.0.1");

                let audit = NewWebAuditLog {
                    guild_id: Some(guild_id),
                    user_id,
                    user_name: &c.username,
                    domain: ConsoleDomain::Moderation.as_str(),
                    action: "mod_case_deactivate",
                    before_json: Some(&format!("{{\"case_id\": {case_id}}}")),
                    after_json: None,
                    ip_address: ip_hdr,
                    status: "success",
                };
                let _ = state.storage.insert_web_audit_log(&audit);
            }
            Json(json!({ "success": true, "case_id": case_id })).into_response()
        }
        Ok(false) => (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "Case not found or already inactive." })),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": format!("{e}") })),
        )
            .into_response(),
    }
}

#[derive(Debug, Deserialize)]
pub struct StrikesQuery {
    pub user_id: Option<u64>,
    pub limit: Option<usize>,
}

async fn api_get_guild_strikes(
    State(state): State<Arc<WebState>>,
    headers: HeaderMap,
    Path(guild_id_str): Path<String>,
    Query(query): Query<StrikesQuery>,
) -> Response {
    let Ok(guild_id) = guild_id_str.parse::<u64>() else {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "Invalid guild ID." })),
        )
            .into_response();
    };

    if let Err(resp) = check_guild_any_domain_access(
        &state,
        &headers,
        guild_id,
        &[ConsoleDomain::Moderation, ConsoleDomain::Automod],
        AccessLevel::View,
    )
    .await
    {
        return resp;
    }

    if let Some(target_user_id) = query.user_id {
        let strikes = state
            .storage
            .get_user_strikes(guild_id, target_user_id)
            .unwrap_or_default();
        let active_count = state
            .storage
            .get_active_strikes_count(guild_id, target_user_id)
            .unwrap_or(0);
        Json(json!({ "user_id": target_user_id, "active_count": active_count, "strikes": strikes }))
            .into_response()
    } else {
        match state.storage.get_guild_strikes(guild_id, query.limit) {
            Ok(strikes) => Json(json!({ "strikes": strikes })).into_response(),
            Err(e) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": format!("{e}") })),
            )
                .into_response(),
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct DecayStrikesRequest {
    pub cutoff_epoch: Option<i64>,
}

async fn api_decay_guild_strikes(
    State(state): State<Arc<WebState>>,
    headers: HeaderMap,
    Path(guild_id_str): Path<String>,
    Json(payload): Json<DecayStrikesRequest>,
) -> Response {
    let Ok(guild_id) = guild_id_str.parse::<u64>() else {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "Invalid guild ID." })),
        )
            .into_response();
    };

    let (claims, user_id) = match check_guild_any_domain_access(
        &state,
        &headers,
        guild_id,
        &[ConsoleDomain::Moderation, ConsoleDomain::Automod],
        AccessLevel::Edit,
    )
    .await
    {
        Ok(v) => v,
        Err(resp) => return resp,
    };

    let cutoff = match payload.cutoff_epoch {
        Some(ts) => ts,
        None => {
            let automod_cfg = state
                .storage
                .get_automod_config(guild_id)
                .unwrap_or_else(|_| AutomodConfig::default_for_guild(guild_id));
            let hours = automod_cfg.strike_decay_hours.max(1);
            Utc::now()
                .timestamp()
                .saturating_sub(hours.saturating_mul(3600))
        }
    };

    match state.storage.decay_strikes(guild_id, cutoff) {
        Ok(count) => {
            if let Some(ref c) = claims {
                let ip_hdr = headers
                    .get("x-forwarded-for")
                    .and_then(|v| v.to_str().ok())
                    .and_then(|s| s.split(',').next())
                    .map(str::trim)
                    .unwrap_or("127.0.0.1");

                let audit = NewWebAuditLog {
                    guild_id: Some(guild_id),
                    user_id,
                    user_name: &c.username,
                    domain: ConsoleDomain::Automod.as_str(),
                    action: "strike_decay",
                    before_json: None,
                    after_json: Some(&format!("{{\"decayed_count\": {count}, \"cutoff_epoch\": {cutoff}}}")),
                    ip_address: ip_hdr,
                    status: "success",
                };
                let _ = state.storage.insert_web_audit_log(&audit);
            }
            Json(json!({
                "success": true,
                "decayed_count": count,
                "cutoff_epoch": cutoff
            }))
            .into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": format!("{e}") })),
        )
            .into_response(),
    }
}

async fn api_get_automod_config(
    State(state): State<Arc<WebState>>,
    headers: HeaderMap,
    Path(guild_id_str): Path<String>,
) -> Response {
    let Ok(guild_id) = guild_id_str.parse::<u64>() else {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "Invalid guild ID." })),
        )
            .into_response();
    };

    if let Err(resp) = check_guild_domain_access(
        &state,
        &headers,
        guild_id,
        ConsoleDomain::Automod,
        AccessLevel::View,
    )
    .await
    {
        return resp;
    }

    match state.storage.get_automod_config(guild_id) {
        Ok(cfg) => Json(json!({ "config": cfg })).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": format!("{e}") })),
        )
            .into_response(),
    }
}

async fn api_put_automod_config(
    State(state): State<Arc<WebState>>,
    headers: HeaderMap,
    Path(guild_id_str): Path<String>,
    Json(mut config): Json<AutomodConfig>,
) -> Response {
    let Ok(guild_id) = guild_id_str.parse::<u64>() else {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "Invalid guild ID." })),
        )
            .into_response();
    };
    config.guild_id = guild_id;

    let (claims, user_id) = match check_guild_domain_access(
        &state,
        &headers,
        guild_id,
        ConsoleDomain::Automod,
        AccessLevel::Edit,
    )
    .await
    {
        Ok(v) => v,
        Err(resp) => return resp,
    };

    let before_json = state
        .storage
        .get_automod_config(guild_id)
        .ok()
        .and_then(|c| serde_json::to_string(&c).ok());

    match state.storage.save_automod_config(&config) {
        Ok(()) => {
            if let Some(ref c) = claims {
                let ip_hdr = headers
                    .get("x-forwarded-for")
                    .and_then(|v| v.to_str().ok())
                    .and_then(|s| s.split(',').next())
                    .map(str::trim)
                    .unwrap_or("127.0.0.1");

                let after_json = serde_json::to_string(&config).ok();
                let audit = NewWebAuditLog {
                    guild_id: Some(guild_id),
                    user_id,
                    user_name: &c.username,
                    domain: ConsoleDomain::Automod.as_str(),
                    action: "automod_config_update",
                    before_json: before_json.as_deref(),
                    after_json: after_json.as_deref(),
                    ip_address: ip_hdr,
                    status: "success",
                };
                let _ = state.storage.insert_web_audit_log(&audit);
            }
            Json(json!({ "success": true, "config": config })).into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": format!("{e}") })),
        )
            .into_response(),
    }
}

async fn api_get_logging_config(
    State(state): State<Arc<WebState>>,
    headers: HeaderMap,
    Path(guild_id_str): Path<String>,
) -> Response {
    let Ok(guild_id) = guild_id_str.parse::<u64>() else {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "Invalid guild ID." })),
        )
            .into_response();
    };

    if let Err(resp) = check_guild_domain_access(
        &state,
        &headers,
        guild_id,
        ConsoleDomain::Logging,
        AccessLevel::View,
    )
    .await
    {
        return resp;
    }

    match state.storage.get_logging_config(guild_id) {
        Ok(cfg) => Json(json!({ "config": cfg })).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": format!("{e}") })),
        )
            .into_response(),
    }
}

async fn api_put_logging_config(
    State(state): State<Arc<WebState>>,
    headers: HeaderMap,
    Path(guild_id_str): Path<String>,
    Json(mut config): Json<LoggingConfig>,
) -> Response {
    let Ok(guild_id) = guild_id_str.parse::<u64>() else {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "Invalid guild ID." })),
        )
            .into_response();
    };
    config.guild_id = guild_id;

    let (claims, user_id) = match check_guild_domain_access(
        &state,
        &headers,
        guild_id,
        ConsoleDomain::Logging,
        AccessLevel::Edit,
    )
    .await
    {
        Ok(v) => v,
        Err(resp) => return resp,
    };

    let before_json = state
        .storage
        .get_logging_config(guild_id)
        .ok()
        .and_then(|c| serde_json::to_string(&c).ok());

    match state.storage.save_logging_config(&config) {
        Ok(()) => {
            if let Some(ref c) = claims {
                let ip_hdr = headers
                    .get("x-forwarded-for")
                    .and_then(|v| v.to_str().ok())
                    .and_then(|s| s.split(',').next())
                    .map(str::trim)
                    .unwrap_or("127.0.0.1");

                let after_json = serde_json::to_string(&config).ok();
                let audit = NewWebAuditLog {
                    guild_id: Some(guild_id),
                    user_id,
                    user_name: &c.username,
                    domain: ConsoleDomain::Logging.as_str(),
                    action: "logging_config_update",
                    before_json: before_json.as_deref(),
                    after_json: after_json.as_deref(),
                    ip_address: ip_hdr,
                    status: "success",
                };
                let _ = state.storage.insert_web_audit_log(&audit);
            }
            Json(json!({ "success": true, "config": config })).into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": format!("{e}") })),
        )
            .into_response(),
    }
}

async fn api_get_greetings_config(
    State(state): State<Arc<WebState>>,
    headers: HeaderMap,
    Path(guild_id_str): Path<String>,
) -> Response {
    let Ok(guild_id) = guild_id_str.parse::<u64>() else {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "Invalid guild ID." })),
        )
            .into_response();
    };

    if let Err(resp) = check_guild_domain_access(
        &state,
        &headers,
        guild_id,
        ConsoleDomain::Greetings,
        AccessLevel::View,
    )
    .await
    {
        return resp;
    }

    match state.storage.get_greetings_config(guild_id) {
        Ok(cfg) => Json(json!({ "config": cfg })).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": format!("{e}") })),
        )
            .into_response(),
    }
}

async fn api_put_greetings_config(
    State(state): State<Arc<WebState>>,
    headers: HeaderMap,
    Path(guild_id_str): Path<String>,
    Json(mut config): Json<GreetingsConfig>,
) -> Response {
    let Ok(guild_id) = guild_id_str.parse::<u64>() else {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "Invalid guild ID." })),
        )
            .into_response();
    };
    config.guild_id = guild_id;

    let (claims, user_id) = match check_guild_domain_access(
        &state,
        &headers,
        guild_id,
        ConsoleDomain::Greetings,
        AccessLevel::Edit,
    )
    .await
    {
        Ok(v) => v,
        Err(resp) => return resp,
    };

    let before_json = state
        .storage
        .get_greetings_config(guild_id)
        .ok()
        .and_then(|c| serde_json::to_string(&c).ok());

    match state.storage.save_greetings_config(&config) {
        Ok(()) => {
            if let Some(ref c) = claims {
                let ip_hdr = headers
                    .get("x-forwarded-for")
                    .and_then(|v| v.to_str().ok())
                    .and_then(|s| s.split(',').next())
                    .map(str::trim)
                    .unwrap_or("127.0.0.1");

                let after_json = serde_json::to_string(&config).ok();
                let audit = NewWebAuditLog {
                    guild_id: Some(guild_id),
                    user_id,
                    user_name: &c.username,
                    domain: ConsoleDomain::Settings.as_str(),
                    action: "greetings_config_update",
                    before_json: before_json.as_deref(),
                    after_json: after_json.as_deref(),
                    ip_address: ip_hdr,
                    status: "success",
                };
                let _ = state.storage.insert_web_audit_log(&audit);
            }
            Json(json!({ "success": true, "config": config })).into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": format!("{e}") })),
        )
            .into_response(),
    }
}

#[derive(Debug, Deserialize)]
pub struct CreateReactionRoleGroupRequest {
    #[serde(with = "silas_core::snowflake")]
    pub channel_id: u64,
    #[serde(with = "silas_core::snowflake")]
    pub message_id: u64,
    pub group_type: Option<String>,
    #[serde(default, with = "silas_core::snowflake::opt")]
    pub required_role_id: Option<u64>,
    pub component_type: Option<String>,
    #[serde(default)]
    pub entries: Vec<CreateReactionRoleEntryItem>,
}

#[derive(Debug, Deserialize)]
pub struct CreateReactionRoleEntryItem {
    pub emoji_or_custom_id: String,
    #[serde(with = "silas_core::snowflake")]
    pub role_id: u64,
    pub button_label: Option<String>,
    pub button_style: Option<String>,
}

async fn api_list_reaction_roles(
    State(state): State<Arc<WebState>>,
    headers: HeaderMap,
    Path(guild_id_str): Path<String>,
) -> Response {
    let Ok(guild_id) = guild_id_str.parse::<u64>() else {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "Invalid guild ID." })),
        )
            .into_response();
    };

    if let Err(resp) = check_guild_domain_access(
        &state,
        &headers,
        guild_id,
        ConsoleDomain::ReactionRoles,
        AccessLevel::View,
    )
    .await
    {
        return resp;
    }

    match state.storage.get_guild_reaction_roles(guild_id) {
        Ok(groups) => Json(json!({ "groups": groups })).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": format!("{e}") })),
        )
            .into_response(),
    }
}

async fn api_create_reaction_roles(
    State(state): State<Arc<WebState>>,
    headers: HeaderMap,
    Path(guild_id_str): Path<String>,
    Json(payload): Json<CreateReactionRoleGroupRequest>,
) -> Response {
    let Ok(guild_id) = guild_id_str.parse::<u64>() else {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "Invalid guild ID." })),
        )
            .into_response();
    };

    let (claims, user_id) = match check_guild_domain_access(
        &state,
        &headers,
        guild_id,
        ConsoleDomain::ReactionRoles,
        AccessLevel::Edit,
    )
    .await
    {
        Ok(v) => v,
        Err(resp) => return resp,
    };

    let new_group = NewReactionRoleGroup {
        guild_id,
        channel_id: payload.channel_id,
        message_id: payload.message_id,
        group_type: payload
            .group_type
            .unwrap_or_else(|| "normal".to_string()),
        required_role_id: payload.required_role_id,
        component_type: payload
            .component_type
            .unwrap_or_else(|| "reaction".to_string()),
    };

    let mut created_group = match state.storage.create_reaction_role_group(&new_group) {
        Ok(g) => g,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": format!("{e}") })),
            )
                .into_response();
        }
    };

    for entry in payload.entries {
        let new_entry = NewReactionRoleEntry {
            group_id: created_group.id,
            emoji_or_custom_id: entry.emoji_or_custom_id,
            role_id: entry.role_id,
            button_label: entry.button_label,
            button_style: entry.button_style,
        };
        if let Ok(saved_entry) = state.storage.add_reaction_role_entry(&new_entry) {
            created_group.entries.push(saved_entry);
        }
    }

    if let Some(ref c) = claims {
        let ip_hdr = headers
            .get("x-forwarded-for")
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.split(',').next())
            .map(str::trim)
            .unwrap_or("127.0.0.1");

        let after_json = serde_json::to_string(&created_group).ok();
        let audit = NewWebAuditLog {
            guild_id: Some(guild_id),
            user_id,
            user_name: &c.username,
            domain: ConsoleDomain::Settings.as_str(),
            action: "reaction_role_group_create",
            before_json: None,
            after_json: after_json.as_deref(),
            ip_address: ip_hdr,
            status: "success",
        };
        let _ = state.storage.insert_web_audit_log(&audit);
    }

    (StatusCode::CREATED, Json(json!({ "group": created_group }))).into_response()
}

async fn api_delete_reaction_role_group(
    State(state): State<Arc<WebState>>,
    headers: HeaderMap,
    Path((guild_id_str, group_id_str)): Path<(String, String)>,
) -> Response {
    let Ok(guild_id) = guild_id_str.parse::<u64>() else {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "Invalid guild ID." })),
        )
            .into_response();
    };
    let Ok(group_id) = group_id_str.parse::<i64>() else {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "Invalid group ID." })),
        )
            .into_response();
    };

    let (claims, user_id) = match check_guild_domain_access(
        &state,
        &headers,
        guild_id,
        ConsoleDomain::ReactionRoles,
        AccessLevel::Edit,
    )
    .await
    {
        Ok(v) => v,
        Err(resp) => return resp,
    };

    // Verify group belongs to this guild to prevent cross-guild IDOR
    let groups = state.storage.get_guild_reaction_roles(guild_id).unwrap_or_default();
    if !groups.iter().any(|g| g.id == group_id) {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "Reaction role group not found." })),
        )
            .into_response();
    }

    match state.storage.delete_reaction_role_group(group_id) {
        Ok(true) => {
            if let Some(ref c) = claims {
                let ip_hdr = headers
                    .get("x-forwarded-for")
                    .and_then(|v| v.to_str().ok())
                    .and_then(|s| s.split(',').next())
                    .map(str::trim)
                    .unwrap_or("127.0.0.1");

                let audit = NewWebAuditLog {
                    guild_id: Some(guild_id),
                    user_id,
                    user_name: &c.username,
                    domain: ConsoleDomain::Settings.as_str(),
                    action: "reaction_role_group_delete",
                    before_json: Some(&format!("{{\"group_id\": {group_id}}}")),
                    after_json: None,
                    ip_address: ip_hdr,
                    status: "success",
                };
                let _ = state.storage.insert_web_audit_log(&audit);
            }
            Json(json!({ "success": true, "group_id": group_id })).into_response()
        }
        Ok(false) => (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "Reaction role group not found." })),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": format!("{e}") })),
        )
            .into_response(),
    }
}

#[derive(Debug, Deserialize)]
pub struct CreateCustomTriggerRequest {
    pub trigger_name: String,
    pub match_mode: Option<String>,
    pub response_template: String,
    pub cooldown_seconds: Option<i64>,
    #[serde(default, with = "silas_core::snowflake::vec")]
    pub allowed_roles: Vec<u64>,
    #[serde(default, with = "silas_core::snowflake::vec")]
    pub ignored_channels: Vec<u64>,
    pub delete_trigger_message: Option<bool>,
    pub reply_in_dm: Option<bool>,
    pub enabled: Option<bool>,
}

async fn api_list_custom_triggers(
    State(state): State<Arc<WebState>>,
    headers: HeaderMap,
    Path(guild_id_str): Path<String>,
) -> Response {
    let Ok(guild_id) = guild_id_str.parse::<u64>() else {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "Invalid guild ID." })),
        )
            .into_response();
    };

    if let Err(resp) = check_guild_domain_access(
        &state,
        &headers,
        guild_id,
        ConsoleDomain::Triggers,
        AccessLevel::View,
    )
    .await
    {
        return resp;
    }

    match state.storage.list_custom_triggers(guild_id) {
        Ok(triggers) => Json(json!({ "triggers": triggers })).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": format!("{e}") })),
        )
            .into_response(),
    }
}

async fn api_create_custom_trigger(
    State(state): State<Arc<WebState>>,
    headers: HeaderMap,
    Path(guild_id_str): Path<String>,
    Json(payload): Json<CreateCustomTriggerRequest>,
) -> Response {
    let Ok(guild_id) = guild_id_str.parse::<u64>() else {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "Invalid guild ID." })),
        )
            .into_response();
    };

    let (claims, user_id) = match check_guild_domain_access(
        &state,
        &headers,
        guild_id,
        ConsoleDomain::Triggers,
        AccessLevel::Edit,
    )
    .await
    {
        Ok(v) => v,
        Err(resp) => return resp,
    };

    let new_trigger = NewCustomTrigger {
        guild_id,
        trigger_name: payload.trigger_name,
        match_mode: payload.match_mode.unwrap_or_else(|| "prefix".to_string()),
        response_template: payload.response_template,
        cooldown_seconds: payload.cooldown_seconds.unwrap_or(0),
        allowed_roles: payload.allowed_roles,
        ignored_channels: payload.ignored_channels,
        delete_trigger_message: payload.delete_trigger_message.unwrap_or(false),
        reply_in_dm: payload.reply_in_dm.unwrap_or(false),
        enabled: payload.enabled.unwrap_or(true),
    };

    match state.storage.create_custom_trigger(&new_trigger) {
        Ok(trigger) => {
            if let Some(ref c) = claims {
                let ip_hdr = headers
                    .get("x-forwarded-for")
                    .and_then(|v| v.to_str().ok())
                    .and_then(|s| s.split(',').next())
                    .map(str::trim)
                    .unwrap_or("127.0.0.1");

                let after_json = serde_json::to_string(&trigger).ok();
                let audit = NewWebAuditLog {
                    guild_id: Some(guild_id),
                    user_id,
                    user_name: &c.username,
                    domain: ConsoleDomain::Settings.as_str(),
                    action: "custom_trigger_create",
                    before_json: None,
                    after_json: after_json.as_deref(),
                    ip_address: ip_hdr,
                    status: "success",
                };
                let _ = state.storage.insert_web_audit_log(&audit);
            }
            Json(json!({ "success": true, "trigger": trigger })).into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": format!("{e}") })),
        )
            .into_response(),
    }
}

async fn api_delete_custom_trigger(
    State(state): State<Arc<WebState>>,
    headers: HeaderMap,
    Path((guild_id_str, trigger_name)): Path<(String, String)>,
) -> Response {
    let Ok(guild_id) = guild_id_str.parse::<u64>() else {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "Invalid guild ID." })),
        )
            .into_response();
    };

    let (claims, user_id) = match check_guild_domain_access(
        &state,
        &headers,
        guild_id,
        ConsoleDomain::Triggers,
        AccessLevel::Edit,
    )
    .await
    {
        Ok(v) => v,
        Err(resp) => return resp,
    };

    match state.storage.delete_custom_trigger(guild_id, &trigger_name) {
        Ok(true) => {
            if let Some(ref c) = claims {
                let ip_hdr = headers
                    .get("x-forwarded-for")
                    .and_then(|v| v.to_str().ok())
                    .and_then(|s| s.split(',').next())
                    .map(str::trim)
                    .unwrap_or("127.0.0.1");

                let audit = NewWebAuditLog {
                    guild_id: Some(guild_id),
                    user_id,
                    user_name: &c.username,
                    domain: ConsoleDomain::Settings.as_str(),
                    action: "custom_trigger_delete",
                    before_json: Some(&format!("{{\"trigger_name\": {:?}}}", trigger_name)),
                    after_json: None,
                    ip_address: ip_hdr,
                    status: "success",
                };
                let _ = state.storage.insert_web_audit_log(&audit);
            }
            Json(json!({ "success": true, "trigger_name": trigger_name })).into_response()
        }
        Ok(false) => (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "Trigger not found." })),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": format!("{e}") })),
        )
            .into_response(),
    }
}

#[derive(Debug, Deserialize)]
pub struct TagScriptPreviewRequest {
    pub template: String,
    #[serde(default, with = "silas_core::snowflake::opt")]
    pub author_id: Option<u64>,
    pub author_name: Option<String>,
    pub author_avatar: Option<String>,
    #[serde(default, with = "silas_core::snowflake::opt")]
    pub target_id: Option<u64>,
    pub target_name: Option<String>,
    #[serde(default, with = "silas_core::snowflake::opt")]
    pub guild_id: Option<u64>,
    pub guild_name: Option<String>,
    #[serde(default, with = "silas_core::snowflake::opt")]
    pub guild_members: Option<u64>,
    #[serde(default, with = "silas_core::snowflake::opt")]
    pub channel_id: Option<u64>,
    pub channel_name: Option<String>,
    pub custom_vars: Option<HashMap<String, String>>,
}

async fn api_tagscript_preview(
    Json(payload): Json<TagScriptPreviewRequest>,
) -> Response {
    let ctx = TagScriptContext {
        author_id: payload.author_id.unwrap_or(123456789012345678),
        author_name: payload
            .author_name
            .unwrap_or_else(|| "CyberUser#0001".to_string()),
        author_avatar: payload.author_avatar,
        author_created_at: Some("2026-01-01T00:00:00Z".to_string()),
        target_id: payload.target_id.or(Some(987654321098765432)),
        target_name: payload.target_name.or_else(|| Some("TargetMember".to_string())),
        guild_id: payload.guild_id.or(Some(112233445566778899)),
        guild_name: payload
            .guild_name
            .or_else(|| Some("Neon City Sanctuary".to_string())),
        guild_members: payload.guild_members.or(Some(1337)),
        channel_id: payload.channel_id.or(Some(100200300400500600)),
        channel_name: payload.channel_name.or_else(|| Some("cyber-hub".to_string())),
        custom_vars: payload.custom_vars.unwrap_or_default(),
    };

    let result = process_tagscript(&payload.template, &ctx);
    Json(json!({ "result": result })).into_response()
}

// ============================================================================
// Activity, Rewards & Command Channels API
// ============================================================================

async fn api_get_activity_config(
    State(state): State<Arc<WebState>>,
    headers: HeaderMap,
    Path(guild_id_str): Path<String>,
) -> Response {
    let Ok(guild_id) = guild_id_str.parse::<u64>() else {
        return (StatusCode::BAD_REQUEST, Json(json!({ "error": "Invalid guild ID." }))).into_response();
    };

    if let Err(resp) = check_guild_domain_access(
        &state,
        &headers,
        guild_id,
        ConsoleDomain::Activity,
        AccessLevel::View,
    )
    .await
    {
        return resp;
    }

    match state.storage.get_activity_config(guild_id) {
        Ok(cfg) => Json(json!({ "config": cfg })).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": format!("{e}") }))).into_response(),
    }
}

async fn api_put_activity_config(
    State(state): State<Arc<WebState>>,
    headers: HeaderMap,
    Path(guild_id_str): Path<String>,
    Json(mut config): Json<ActivityConfig>,
) -> Response {
    let Ok(guild_id) = guild_id_str.parse::<u64>() else {
        return (StatusCode::BAD_REQUEST, Json(json!({ "error": "Invalid guild ID." }))).into_response();
    };
    config.guild_id = guild_id;

    let (claims, user_id) = match check_guild_domain_access(
        &state,
        &headers,
        guild_id,
        ConsoleDomain::Activity,
        AccessLevel::Edit,
    )
    .await
    {
        Ok(v) => v,
        Err(resp) => return resp,
    };

    let before_json = state
        .storage
        .get_activity_config(guild_id)
        .ok()
        .and_then(|c| serde_json::to_string(&c).ok());

    match state.storage.set_activity_config(&config) {
        Ok(()) => {
            if let Some(ref c) = claims {
                let ip_hdr = headers
                    .get("x-forwarded-for")
                    .and_then(|v| v.to_str().ok())
                    .and_then(|s| s.split(',').next())
                    .map(str::trim)
                    .unwrap_or("127.0.0.1");

                let after_json = serde_json::to_string(&config).ok();
                let audit = NewWebAuditLog {
                    guild_id: Some(guild_id),
                    user_id,
                    user_name: &c.username,
                    domain: "activity",
                    action: "update_config",
                    before_json: before_json.as_deref(),
                    after_json: after_json.as_deref(),
                    ip_address: ip_hdr,
                    status: "success",
                };
                let _ = state.storage.insert_web_audit_log(&audit);
            }
            Json(json!({ "success": true, "config": config })).into_response()
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": format!("{e}") }))).into_response(),
    }
}

#[derive(Deserialize)]
struct LeaderboardQuery {
    timeframe: Option<String>,
    page: Option<usize>,
    limit: Option<usize>,
}

async fn api_get_activity_leaderboard(
    State(state): State<Arc<WebState>>,
    headers: HeaderMap,
    Path(guild_id_str): Path<String>,
    Query(query): Query<LeaderboardQuery>,
) -> Response {
    let Ok(guild_id) = guild_id_str.parse::<u64>() else {
        return (StatusCode::BAD_REQUEST, Json(json!({ "error": "Invalid guild ID." }))).into_response();
    };

    if let Err(resp) = check_guild_domain_access(
        &state,
        &headers,
        guild_id,
        ConsoleDomain::Activity,
        AccessLevel::View,
    )
    .await
    {
        return resp;
    }

    let tf = query.timeframe.as_deref().unwrap_or("lifetime");
    let limit = query.limit.unwrap_or(20).clamp(1, 100);
    let page = query.page.unwrap_or(1).max(1);
    let offset = (page - 1) * limit;

    match state.storage.get_leaderboard(guild_id, tf, limit, offset) {
        Ok((entries, total)) => {
            let cache_opt = state.bot.cache();
            let http_opt = state.bot.http();
            let gid = serenity::model::id::GuildId::new(guild_id);

            let mut list = Vec::with_capacity(entries.len());
            for e in entries {
                let uid = serenity::model::id::UserId::new(e.user_id);
                let cached_info = cache_opt.as_ref().and_then(|c| {
                    c.guild(guild_id).and_then(|g| {
                        g.members.get(&uid).map(|m| {
                            (
                                m.user.name.clone(),
                                m.nick.clone().unwrap_or_else(|| m.user.name.clone()),
                                m.user.avatar_url(),
                            )
                        })
                    })
                });

                let (username, display_name, avatar_url) = if let Some(info) = cached_info {
                    (Some(info.0), Some(info.1), info.2)
                } else if let Some(ref http) = http_opt {
                    if let Ok(member) = http.get_member(gid, uid).await {
                        let u = member.user.name.clone();
                        let d = member.nick.clone().unwrap_or_else(|| member.user.name.clone());
                        let a = member.user.avatar_url();
                        (Some(u), Some(d), a)
                    } else if let Ok(user) = http.get_user(uid).await {
                        let u = user.name.clone();
                        let d = user.global_name.clone().unwrap_or_else(|| user.name.clone());
                        let a = user.avatar_url();
                        (Some(u), Some(d), a)
                    } else {
                        (None, None, None)
                    }
                } else {
                    (None, None, None)
                };

                list.push(json!({
                    "user_id": e.user_id.to_string(),
                    "rank": e.rank,
                    "points": e.points,
                    "text_points": e.text_points,
                    "voice_points": e.voice_points,
                    "reaction_points": e.reaction_points,
                    "username": username,
                    "display_name": display_name,
                    "avatar_url": avatar_url,
                }));
            }

            Json(json!({ "entries": list, "total": total, "page": page, "limit": limit })).into_response()
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": format!("{e}") }))).into_response(),
    }
}

async fn api_list_activity_rewards(
    State(state): State<Arc<WebState>>,
    headers: HeaderMap,
    Path(guild_id_str): Path<String>,
) -> Response {
    let Ok(guild_id) = guild_id_str.parse::<u64>() else {
        return (StatusCode::BAD_REQUEST, Json(json!({ "error": "Invalid guild ID." }))).into_response();
    };

    if let Err(resp) = check_guild_domain_access(
        &state,
        &headers,
        guild_id,
        ConsoleDomain::Activity,
        AccessLevel::View,
    )
    .await
    {
        return resp;
    }

    match state.storage.list_activity_reward_rules(guild_id) {
        Ok(rules) => {
            let cache_opt = state.bot.cache();
            let list: Vec<Value> = rules.into_iter().map(|r| {
                let role_name = cache_opt.as_ref().and_then(|c| {
                    c.guild(guild_id).and_then(|g| {
                        g.roles.get(&serenity::model::id::RoleId::new(r.role_id)).map(|role| role.name.clone())
                    })
                });
                json!({
                    "id": r.id,
                    "guild_id": r.guild_id.to_string(),
                    "role_id": r.role_id.to_string(),
                    "role_name": role_name,
                    "rule_type": r.rule_type,
                    "threshold_points": r.threshold_points,
                    "top_rank": r.top_rank,
                    "timeframe": r.timeframe,
                    "remove_on_drop": r.remove_on_drop,
                    "created_at": r.created_at,
                })
            }).collect();
            Json(json!({ "rules": list })).into_response()
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": format!("{e}") }))).into_response(),
    }
}

async fn api_create_activity_reward(
    State(state): State<Arc<WebState>>,
    headers: HeaderMap,
    Path(guild_id_str): Path<String>,
    Json(mut rule): Json<NewActivityRewardRule>,
) -> Response {
    let Ok(guild_id) = guild_id_str.parse::<u64>() else {
        return (StatusCode::BAD_REQUEST, Json(json!({ "error": "Invalid guild ID." }))).into_response();
    };
    rule.guild_id = guild_id;

    let (claims, user_id) = match check_guild_domain_access(
        &state,
        &headers,
        guild_id,
        ConsoleDomain::Activity,
        AccessLevel::Edit,
    )
    .await
    {
        Ok(v) => v,
        Err(resp) => return resp,
    };

    match state.storage.create_activity_reward_rule(rule) {
        Ok(created) => {
            if let Some(ref c) = claims {
                let ip_hdr = headers
                    .get("x-forwarded-for")
                    .and_then(|v| v.to_str().ok())
                    .and_then(|s| s.split(',').next())
                    .map(str::trim)
                    .unwrap_or("127.0.0.1");

                let after_json = serde_json::to_string(&created).ok();
                let audit = NewWebAuditLog {
                    guild_id: Some(guild_id),
                    user_id,
                    user_name: &c.username,
                    domain: "activity",
                    action: "create_reward_rule",
                    before_json: None,
                    after_json: after_json.as_deref(),
                    ip_address: ip_hdr,
                    status: "success",
                };
                let _ = state.storage.insert_web_audit_log(&audit);
            }
            Json(json!({ "success": true, "rule": created })).into_response()
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": format!("{e}") }))).into_response(),
    }
}

async fn api_delete_activity_reward(
    State(state): State<Arc<WebState>>,
    headers: HeaderMap,
    Path((guild_id_str, rule_id)): Path<(String, String)>,
) -> Response {
    let Ok(guild_id) = guild_id_str.parse::<u64>() else {
        return (StatusCode::BAD_REQUEST, Json(json!({ "error": "Invalid guild ID." }))).into_response();
    };

    let (claims, user_id) = match check_guild_domain_access(
        &state,
        &headers,
        guild_id,
        ConsoleDomain::Activity,
        AccessLevel::Edit,
    )
    .await
    {
        Ok(v) => v,
        Err(resp) => return resp,
    };

    match state.storage.delete_activity_reward_rule(guild_id, &rule_id) {
        Ok(deleted) => {
            if deleted {
                if let Some(ref c) = claims {
                    let ip_hdr = headers
                        .get("x-forwarded-for")
                        .and_then(|v| v.to_str().ok())
                        .and_then(|s| s.split(',').next())
                        .map(str::trim)
                        .unwrap_or("127.0.0.1");

                    let audit = NewWebAuditLog {
                        guild_id: Some(guild_id),
                        user_id,
                        user_name: &c.username,
                        domain: "activity",
                        action: "delete_reward_rule",
                        before_json: Some(&rule_id),
                        after_json: None,
                        ip_address: ip_hdr,
                        status: "success",
                    };
                    let _ = state.storage.insert_web_audit_log(&audit);
                }
                Json(json!({ "success": true })).into_response()
            } else {
                (StatusCode::NOT_FOUND, Json(json!({ "error": "Rule not found" }))).into_response()
            }
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": format!("{e}") }))).into_response(),
    }
}

#[derive(Deserialize)]
struct AdjustPointsPayload {
    user_id: u64,
    amount: i64,
}

async fn api_adjust_activity_points(
    State(state): State<Arc<WebState>>,
    headers: HeaderMap,
    Path(guild_id_str): Path<String>,
    Json(payload): Json<AdjustPointsPayload>,
) -> Response {
    let Ok(guild_id) = guild_id_str.parse::<u64>() else {
        return (StatusCode::BAD_REQUEST, Json(json!({ "error": "Invalid guild ID." }))).into_response();
    };

    let (claims, caller_user_id) = match check_guild_any_domain_access(
        &state,
        &headers,
        guild_id,
        &[ConsoleDomain::Activity, ConsoleDomain::Moderation],
        AccessLevel::Edit,
    )
    .await
    {
        Ok(v) => v,
        Err(resp) => return resp,
    };

    match state.storage.adjust_user_points(guild_id, payload.user_id, payload.amount) {
        Ok(new_total) => {
            if let Some(ref c) = claims {
                let ip_hdr = headers
                    .get("x-forwarded-for")
                    .and_then(|v| v.to_str().ok())
                    .and_then(|s| s.split(',').next())
                    .map(str::trim)
                    .unwrap_or("127.0.0.1");

                let details = format!("user: {}, amount: {}, new_total: {}", payload.user_id, payload.amount, new_total);
                let audit = NewWebAuditLog {
                    guild_id: Some(guild_id),
                    user_id: caller_user_id,
                    user_name: &c.username,
                    domain: "activity",
                    action: "adjust_points",
                    before_json: None,
                    after_json: Some(&details),
                    ip_address: ip_hdr,
                    status: "success",
                };
                let _ = state.storage.insert_web_audit_log(&audit);
            }
            Json(json!({ "success": true, "new_total": new_total })).into_response()
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": format!("{e}") }))).into_response(),
    }
}

#[derive(Serialize, Deserialize)]
struct CommandChannelsPayload {
    restrictions: GuildCommandRestrictions,
    rules: Vec<CommandChannelRule>,
}

async fn api_get_command_channels(
    State(state): State<Arc<WebState>>,
    headers: HeaderMap,
    Path(guild_id_str): Path<String>,
) -> Response {
    let Ok(guild_id) = guild_id_str.parse::<u64>() else {
        return (StatusCode::BAD_REQUEST, Json(json!({ "error": "Invalid guild ID." }))).into_response();
    };

    if let Err(resp) = check_guild_any_domain_access(
        &state,
        &headers,
        guild_id,
        &[ConsoleDomain::Settings, ConsoleDomain::Activity],
        AccessLevel::View,
    )
    .await
    {
        return resp;
    }

    let restr = state.storage.get_guild_command_restrictions(guild_id).unwrap_or_else(|_| GuildCommandRestrictions::default_for_guild(guild_id));
    let rules = state.storage.list_command_channel_rules(guild_id).unwrap_or_default();

    Json(json!({ "restrictions": restr, "rules": rules })).into_response()
}

async fn api_put_command_channels(
    State(state): State<Arc<WebState>>,
    headers: HeaderMap,
    Path(guild_id_str): Path<String>,
    Json(mut payload): Json<CommandChannelsPayload>,
) -> Response {
    let Ok(guild_id) = guild_id_str.parse::<u64>() else {
        return (StatusCode::BAD_REQUEST, Json(json!({ "error": "Invalid guild ID." }))).into_response();
    };
    payload.restrictions.guild_id = guild_id;

    if let Err(resp) = check_guild_any_domain_access(
        &state,
        &headers,
        guild_id,
        &[ConsoleDomain::Settings, ConsoleDomain::Activity],
        AccessLevel::Edit,
    )
    .await
    {
        return resp;
    }

    if let Err(e) = state.storage.set_guild_command_restrictions(&payload.restrictions) {
        return (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": format!("{e}") }))).into_response();
    }

    for rule in &payload.rules {
        let mut r = rule.clone();
        r.guild_id = guild_id;
        let _ = state.storage.set_command_channel_rule(&r);
    }

    Json(json!({ "success": true })).into_response()
}

// ============================================================================
// Embedded Static Asset Serving
// ============================================================================

async fn serve_embedded_asset(
    State(state): State<Arc<WebState>>,
    headers: HeaderMap,
    uri: Uri,
) -> Response {
    let mut path = uri.path().trim_start_matches('/');
    if path.is_empty() {
        path = "index.html";
    } else if path == "login" {
        path = "login.html";
    }

    // Redirect unauthenticated requests for index.html to login.html
    if path == "index.html" {
        let settings = state.storage.get_app_settings();
        if state.auth.authenticate_request(&headers, &settings).is_none() {
            return Redirect::to("/login.html").into_response();
        }
    }

    if let Some(content) = EmbeddedAssets::get(path) {
        let mime = mime_guess::from_path(path).first_or_octet_stream();
        let mut resp = Response::new(Body::from(content.data.into_owned()));
        if let Ok(hv) = HeaderValue::from_str(mime.as_ref()) {
            resp.headers_mut().insert(header::CONTENT_TYPE, hv);
        }
        resp.headers_mut().insert(
            header::CACHE_CONTROL,
            HeaderValue::from_static("no-cache, must-revalidate"),
        );
        return resp;
    }

    (StatusCode::NOT_FOUND, "404 Not Found").into_response()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use silas_storage::{
        AutomodConfig, GreetingsConfig, LoggingConfig, NewCustomTrigger, NewModCase,
        NewModStrike, NewReactionRoleEntry, NewReactionRoleGroup, PunishmentStep, Storage,
    };

    #[tokio::test]
    async fn test_tagscript_preview_endpoint() {
        let req = TagScriptPreviewRequest {
            template: "Hello {user.name}! {delete} {title: Server Rules} {color: #38bdf8} {ai: Cyberpunk status update}".to_string(),
            author_id: Some(111222333),
            author_name: Some("NeonAgent".to_string()),
            author_avatar: None,
            target_id: None,
            target_name: None,
            guild_id: Some(999888777),
            guild_name: Some("Neo-Tokyo".to_string()),
            guild_members: Some(42),
            channel_id: Some(555666),
            channel_name: Some("matrix".to_string()),
            custom_vars: None,
        };

        let response = api_tagscript_preview(Json(req)).await;
        let (parts, body) = response.into_parts();
        assert_eq!(parts.status, StatusCode::OK);

        let bytes = axum::body::to_bytes(body, usize::MAX).await.unwrap();
        let val: Value = serde_json::from_slice(&bytes).unwrap();

        let result = val.get("result").unwrap();
        let rendered_body = result.get("body").unwrap().as_str().unwrap();
        assert!(rendered_body.contains("Hello NeonAgent!"));

        let embed = result.get("embed").unwrap();
        assert_eq!(embed.get("title").unwrap().as_str().unwrap(), "Server Rules");
        assert_eq!(embed.get("color").unwrap().as_u64().unwrap(), 0x38bdf8);

        let actions = result.get("actions").unwrap().as_array().unwrap();
        assert!(actions.contains(&json!("DeleteMessage")));

        assert_eq!(
            result.get("ai_prompt").unwrap().as_str().unwrap(),
            "Cyberpunk status update"
        );
    }

    #[tokio::test]
    async fn test_moderation_cases_storage_and_decay() {
        let storage = Storage::open(":memory:").unwrap();
        let guild_id = 123456789;

        // 1. Create a moderation case
        let new_case = NewModCase {
            guild_id,
            user_id: 987654321,
            moderator_id: 111111111,
            action_type: "warn".to_string(),
            reason: "Excessive caps".to_string(),
            duration_seconds: None,
            created_at: Some(1700000000),
            expires_at: None,
        };
        let case = storage.create_mod_case(&new_case).unwrap();
        assert_eq!(case.case_number, 1);
        assert_eq!(case.reason, "Excessive caps");
        assert!(case.active);

        // 2. Fetch guild cases
        let cases = storage.get_mod_cases_for_guild(guild_id, None).unwrap();
        assert_eq!(cases.len(), 1);
        assert_eq!(cases.first().unwrap().case_number, 1);

        // 3. Update reason
        let updated = storage
            .update_case_reason_by_id(case.id, "Updated: spamming caps lock")
            .unwrap();
        assert!(updated);
        let fetched = storage.get_mod_case(guild_id, 1).unwrap().unwrap();
        assert_eq!(fetched.reason, "Updated: spamming caps lock");

        // 4. Add strikes and decay
        let strike = NewModStrike {
            guild_id,
            user_id: 987654321,
            case_id: Some(case.id),
            strike_count: 2,
            reason: "Caps spam".to_string(),
            created_at: Some(1700000000),
            expires_at: Some(1700000100),
        };
        storage.add_strike(&strike).unwrap();

        let count = storage
            .get_active_strikes_count(guild_id, 987654321)
            .unwrap();
        assert_eq!(count, 2);

        let decayed = storage.decay_strikes(guild_id, 1700000500).unwrap();
        assert_eq!(decayed, 1);

        let count_after = storage
            .get_active_strikes_count(guild_id, 987654321)
            .unwrap();
        assert_eq!(count_after, 0);

        // 5. Deactivate case
        let deact = storage.deactivate_case(case.id).unwrap();
        assert!(deact);
        let fetched_deact = storage.get_mod_case(guild_id, 1).unwrap().unwrap();
        assert!(!fetched_deact.active);
    }

    #[tokio::test]
    async fn test_automod_config_roundtrip() {
        let storage = Storage::open(":memory:").unwrap();
        let guild_id = 99887766;

        let mut config = AutomodConfig::default_for_guild(guild_id);
        config.anti_spam_max_messages = 8;
        config.blocked_words = vec!["toxic".to_string(), "griefer".to_string()];
        config.whitelisted_domains = vec!["trusted.org".to_string()];
        config.punishment_ladder.push(PunishmentStep {
            strikes: 15,
            action: "ban".to_string(),
            duration_seconds: None,
        });

        storage.save_automod_config(&config).unwrap();

        let loaded = storage.get_automod_config(guild_id).unwrap();
        assert_eq!(loaded.anti_spam_max_messages, 8);
        assert_eq!(loaded.blocked_words, vec!["toxic", "griefer"]);
        assert_eq!(loaded.whitelisted_domains, vec!["trusted.org"]);
        assert_eq!(loaded.punishment_ladder.len(), 5);
    }

    #[tokio::test]
    async fn test_logging_and_greetings_config_roundtrip() {
        let storage = Storage::open(":memory:").unwrap();
        let guild_id = 77665544;

        // Logging config
        let mut log_cfg = LoggingConfig::default_for_guild(guild_id);
        log_cfg.unified_channel_id = Some(111222);
        log_cfg.mod_channel_id = Some(333444);
        log_cfg.embed_color = 0x38bdf8;
        log_cfg.log_bots = false;
        storage.save_logging_config(&log_cfg).unwrap();

        let loaded_log = storage.get_logging_config(guild_id).unwrap();
        assert_eq!(loaded_log.unified_channel_id, Some(111222));
        assert_eq!(loaded_log.mod_channel_id, Some(333444));
        assert_eq!(loaded_log.embed_color, 0x38bdf8);
        assert!(!loaded_log.log_bots);

        // Greetings config
        let mut greet_cfg = GreetingsConfig::default_for_guild(guild_id);
        greet_cfg.welcome_channel_id = Some(555777);
        greet_cfg.welcome_message = "Welcome {user} to the cyber hub!".to_string();
        greet_cfg.member_autorole_ids = vec![999111, 999222];
        greet_cfg.autorole_delay_seconds = 30;
        storage.save_greetings_config(&greet_cfg).unwrap();

        let loaded_greet = storage.get_greetings_config(guild_id).unwrap();
        assert_eq!(loaded_greet.welcome_channel_id, Some(555777));
        assert_eq!(
            loaded_greet.welcome_message,
            "Welcome {user} to the cyber hub!"
        );
        assert_eq!(loaded_greet.member_autorole_ids, vec![999111, 999222]);
        assert_eq!(loaded_greet.autorole_delay_seconds, 30);
    }

    #[tokio::test]
    async fn test_reaction_roles_and_triggers_roundtrip() {
        let storage = Storage::open(":memory:").unwrap();
        let guild_id = 44556677;

        // Reaction roles
        let new_group = NewReactionRoleGroup {
            guild_id,
            channel_id: 101010,
            message_id: 202020,
            group_type: "unique".to_string(),
            required_role_id: None,
            component_type: "button".to_string(),
        };
        let group = storage.create_reaction_role_group(&new_group).unwrap();

        let entry = NewReactionRoleEntry {
            group_id: group.id,
            emoji_or_custom_id: "🚀".to_string(),
            role_id: 888999,
            button_label: Some("Explorer".to_string()),
            button_style: Some("primary".to_string()),
        };
        storage.add_reaction_role_entry(&entry).unwrap();

        let groups = storage.get_guild_reaction_roles(guild_id).unwrap();
        assert_eq!(groups.len(), 1);
        assert_eq!(groups.first().unwrap().entries.len(), 1);
        assert_eq!(groups.first().unwrap().entries.first().unwrap().role_id, 888999);

        let deleted_rr = storage.delete_reaction_role_group(group.id).unwrap();
        assert!(deleted_rr);
        assert!(storage
            .get_guild_reaction_roles(guild_id)
            .unwrap()
            .is_empty());

        // Custom triggers
        let new_trigger = NewCustomTrigger {
            guild_id,
            trigger_name: "!ping".to_string(),
            match_mode: "prefix".to_string(),
            response_template: "Pong! Gateway latency is low.".to_string(),
            cooldown_seconds: 5,
            allowed_roles: vec![],
            ignored_channels: vec![],
            delete_trigger_message: true,
            reply_in_dm: false,
            enabled: true,
        };
        storage.create_custom_trigger(&new_trigger).unwrap();

        let triggers = storage.list_custom_triggers(guild_id).unwrap();
        assert_eq!(triggers.len(), 1);
        assert_eq!(triggers.first().unwrap().trigger_name, "!ping");
        assert!(triggers.first().unwrap().delete_trigger_message);

        let deleted_trig = storage.delete_custom_trigger(guild_id, "!ping").unwrap();
        assert!(deleted_trig);
        assert!(storage.list_custom_triggers(guild_id).unwrap().is_empty());
    }

    #[test]
    fn test_logging_and_greetings_config_json_with_string_snowflakes() -> Result<(), Box<dyn std::error::Error>> {
        let json_payload = r#"{
            "guild_id": "295767175426473984",
            "unified_channel_id": "1341857943265509426",
            "mod_channel_id": null,
            "message_channel_id": "",
            "member_channel_id": null,
            "server_channel_id": null,
            "voice_channel_id": null,
            "ignored_channels": ["295767175426473984", "497607738227097600"],
            "ignored_roles": ["803771637493137458"],
            "log_bots": true,
            "embed_color": 3553599
        }"#;

        let parsed: LoggingConfig = serde_json::from_str(json_payload)?;
        assert_eq!(parsed.guild_id, 295767175426473984);
        assert_eq!(parsed.unified_channel_id, Some(1341857943265509426));
        assert_eq!(parsed.message_channel_id, None);
        assert_eq!(parsed.ignored_channels, vec![295767175426473984, 497607738227097600]);
        assert_eq!(parsed.ignored_roles, vec![803771637493137458]);

        let greet_payload = r#"{
            "guild_id": "295767175426473984",
            "welcome_channel_id": "295767175426473984",
            "welcome_message": "Welcome {user}!",
            "welcome_embed_json": "",
            "welcome_dm_enabled": false,
            "welcome_dm_message": "",
            "farewell_channel_id": null,
            "farewell_message": "",
            "farewell_embed_json": "",
            "member_autorole_ids": ["803771637493137458"],
            "bot_autorole_ids": [],
            "autorole_delay_seconds": 0
        }"#;

        let parsed_greet: GreetingsConfig = serde_json::from_str(greet_payload)?;
        assert_eq!(parsed_greet.welcome_channel_id, Some(295767175426473984));
        assert_eq!(parsed_greet.member_autorole_ids, vec![803771637493137458]);
        Ok(())
    }

    #[test]
    fn test_set_guild_icon_payload_deserialization() -> Result<(), Box<dyn std::error::Error>> {
        let json_data = r#"{"icon": "data:image/png;base64,iVBORw0KGgo="}"#;
        let payload: SetGuildIconPayload = serde_json::from_str(json_data)?;
        assert_eq!(payload.icon, Some("data:image/png;base64,iVBORw0KGgo=".to_string()));

        let json_reset = r#"{"icon": "reset"}"#;
        let payload_reset: SetGuildIconPayload = serde_json::from_str(json_reset)?;
        assert_eq!(payload_reset.icon, Some("reset".to_string()));

        let json_null = r#"{"icon": null}"#;
        let payload_null: SetGuildIconPayload = serde_json::from_str(json_null)?;
        assert_eq!(payload_null.icon, None);

        let json_empty = r#"{}"#;
        let payload_empty: SetGuildIconPayload = serde_json::from_str(json_empty)?;
        assert_eq!(payload_empty.icon, None);
        Ok(())
    }

    #[test]
    fn test_guild_access_profile_roles_and_domains() -> Result<(), Box<dyn std::error::Error>> {
        let admin = GuildAccessProfile::full("admin");
        assert!(admin.can_access);
        assert_eq!(admin.role, "admin");
        assert!(admin.has_domain_access(ConsoleDomain::Settings, AccessLevel::Edit));
        assert!(admin.has_domain_access(ConsoleDomain::Permissions, AccessLevel::Edit));
        assert!(admin.has_domain_access(ConsoleDomain::Moderation, AccessLevel::Edit));

        let storage = Storage::open(":memory:")?;
        let guild_id = 555666777;
        // Grant 'view' on 'logging' and 'edit' on 'moderation' to a role
        let g1 = storage.create_or_update_web_access_grant(
            guild_id,
            "role",
            1001,
            "@ModRole",
            "logging",
            "read",
        )?;
        assert_eq!(g1.access_level, "view");

        let g2 = storage.create_or_update_web_access_grant(
            guild_id,
            "role",
            1001,
            "@ModRole",
            "moderation",
            "write",
        )?;
        assert_eq!(g2.access_level, "edit");

        let grants = storage.get_web_access_grants(guild_id)?;
        assert_eq!(grants.len(), 2);
        Ok(())
    }

}
