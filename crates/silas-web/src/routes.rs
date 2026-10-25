//! REST API routes, OAuth2/OTP handlers, and embedded SPA static asset serving (`rust-embed`).

use crate::auth::{
    build_clear_cookie, build_session_cookie, extract_cookie, verify_csrf, AuthManager,
    OAUTH_STATE_COOKIE_NAME,
};
use crate::ws::ws_logs_handler;
use axum::body::Body;
use axum::extract::{ConnectInfo, Path, Query, State};
use axum::http::{header, HeaderMap, HeaderValue, Method, Request, StatusCode, Uri};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use chrono::Utc;
use cookie::{Cookie, SameSite};
use rand::RngCore;
use rust_embed::RustEmbed;
use serde::Deserialize;
use serde_json::{json, Value};
use silas_ai::detector::probe_local_providers;
use silas_ai::{validate_endpoint_url, ProviderManager, REMOTE_PRESETS};
use silas_core::config::{AppSettingsPatch, GuildConfigPatch};
use silas_core::events::{TelemetryBus, TelemetryEvent};
use silas_core::quota::calculate_window_period;
use silas_discord::BotController;
use silas_search::SearchCoordinator;
use silas_storage::Storage;
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
    let Ok(session_tok) =
        state
            .auth
            .create_session_token("local-console", "Local Console Owner", &settings)
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

    let mut state_bytes = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut state_bytes);
    let oauth_state = URL_SAFE_NO_PAD.encode(state_bytes);

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
        .append_pair("state", &oauth_state);

    let state_cookie = Cookie::build((OAUTH_STATE_COOKIE_NAME, oauth_state))
        .path("/")
        .http_only(true)
        .same_site(SameSite::Lax)
        .max_age(cookie::time::Duration::minutes(10))
        .build()
        .to_string();

    let mut resp = Redirect::to(auth_url.as_str()).into_response();
    if let Ok(hv) = HeaderValue::from_str(&state_cookie) {
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

    let settings = state.storage.get_app_settings();
    let effective_client_id = resolve_effective_client_id(&state, &settings);
    let redirect_uri = resolve_oauth_redirect_uri(&headers);

    match state
        .auth
        .verify_discord_oauth_callback(code, &redirect_uri, &effective_client_id, &settings)
        .await
    {
        Ok((uid, uname)) => {
            state.auth.clear_rate_limit(ip);
            let Ok(session_tok) = state.auth.create_session_token(&uid, &uname, &settings) else {
                return Redirect::to("/login.html?error=session_error").into_response();
            };
            let sess_cookie = build_session_cookie(&session_tok, is_https_request(&headers));
            let clear_state = build_clear_cookie(OAUTH_STATE_COOKIE_NAME);

            let mut resp = Redirect::to("/").into_response();
            if let Ok(hv) = HeaderValue::from_str(&sess_cookie) {
                resp.headers_mut().append(header::SET_COOKIE, hv);
            }
            if let Ok(hv2) = HeaderValue::from_str(&clear_state) {
                resp.headers_mut().append(header::SET_COOKIE, hv2);
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
    let clear = build_clear_cookie(crate::auth::SESSION_COOKIE_NAME);
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
        Json(json!({
            "authenticated": true,
            "user_id": claims.user_id,
            "username": claims.username,
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
    let host = headers
        .get("x-forwarded-host")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.split(',').next())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .or_else(|| headers.get(header::HOST).and_then(|v| v.to_str().ok()))
        .unwrap_or("127.0.0.1:8080");
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

async fn api_bot_start(State(state): State<Arc<WebState>>) -> Response {
    match state.bot.start().await {
        Ok(()) => Json(json!({ "ok": true, "bot_running": true })).into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, Json(json!({ "error": e }))).into_response(),
    }
}

async fn api_bot_stop(State(state): State<Arc<WebState>>) -> Response {
    match state.bot.stop().await {
        Ok(()) => Json(json!({ "ok": true, "bot_running": false })).into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, Json(json!({ "error": e }))).into_response(),
    }
}

async fn api_bot_restart(State(state): State<Arc<WebState>>) -> Response {
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
    Json(payload): Json<UpdateProfilePayload>,
) -> Response {
    match state.bot.update_global_username(&payload.username).await {
        Ok(new_name) => Json(json!({ "ok": true, "username": new_name })).into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, Json(json!({ "error": e }))).into_response(),
    }
}

async fn api_get_settings(State(state): State<Arc<WebState>>) -> Json<Value> {
    let settings = state.storage.get_app_settings();
    Json(json!({
        "settings": settings.masked_for_web(),
    }))
}

async fn api_put_settings(
    State(state): State<Arc<WebState>>,
    headers: HeaderMap,
    Json(patch): Json<AppSettingsPatch>,
) -> Response {
    let current = state.storage.get_app_settings();
    let old_epoch = current.session_epoch;
    let claims = state.auth.authenticate_request(&headers, &current);

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
                .create_session_token(&c.user_id, &c.username, &updated)
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

async fn api_ai_presets() -> Json<Value> {
    Json(json!({ "presets": REMOTE_PRESETS }))
}

#[derive(Deserialize)]
struct ModelsQuery {
    refresh: Option<bool>,
}

async fn api_ai_models(
    State(state): State<Arc<WebState>>,
    Query(q): Query<ModelsQuery>,
) -> Json<Value> {
    let force_refresh = q.refresh.unwrap_or(false);
    let models = state.ai.list_models_cached(force_refresh).await;
    Json(json!({ "ok": true, "models": models }))
}

async fn api_ai_detect(State(state): State<Arc<WebState>>) -> Json<Value> {
    let settings = state.storage.get_app_settings();
    let probe = probe_local_providers(&settings).await;
    Json(json!({ "ok": true, "probe": probe }))
}

#[derive(Deserialize)]
struct PullModelPayload {
    model: String,
}

async fn api_ai_pull(
    State(state): State<Arc<WebState>>,
    Json(payload): Json<PullModelPayload>,
) -> Response {
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
    Json(payload): Json<SearchTestPayload>,
) -> Response {
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

async fn api_list_guilds(State(state): State<Arc<WebState>>) -> Json<Value> {
    let connected = state.bot.connected_guilds_list();
    let mut seen_ids = std::collections::HashSet::new();
    let mut list = Vec::new();

    for g in connected {
        if let Ok(id_num) = g.id.parse::<u64>() {
            seen_ids.insert(id_num);
        }
        list.push(json!({
            "id": g.id,
            "name": g.name,
            "icon_url": g.icon_url,
            "member_count": g.member_count,
            "connected": true,
        }));
    }

    for gid in state.storage.list_guild_ids() {
        if gid != 0 && !seen_ids.contains(&gid) {
            list.push(json!({
                "id": gid.to_string(),
                "name": format!("Server {gid}"),
                "icon_url": null,
                "member_count": 0,
                "connected": false,
            }));
        }
    }

    Json(json!({ "guilds": list }))
}

async fn api_get_guild_config(
    State(state): State<Arc<WebState>>,
    Path(guild_id_str): Path<String>,
) -> Response {
    let Ok(guild_id) = guild_id_str.parse::<u64>() else {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "Invalid guild ID." })),
        )
            .into_response();
    };

    let cfg = state.storage.get_guild_config(guild_id);
    let meta = state.bot.guild_metadata(guild_id);
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
    #[serde(flatten)]
    patch: GuildConfigPatch,
}

async fn api_put_guild_config(
    State(state): State<Arc<WebState>>,
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

    let updated_cfg = match state
        .storage
        .patch_guild_config_async(guild_id, update.patch)
        .await
    {
        Ok(c) => c,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": e.to_string() })),
            )
                .into_response();
        }
    };

    if let Some(ref nick) = update.bot_nickname {
        if state.bot.is_running() && guild_id != 0 {
            let _ = state.bot.update_guild_nickname(guild_id, Some(nick)).await;
        }
    }

    Json(json!({
        "ok": true,
        "config": *updated_cfg,
    }))
    .into_response()
}

async fn api_reset_guild_quota(
    State(state): State<Arc<WebState>>,
    Path(guild_id_str): Path<String>,
) -> Response {
    let Ok(guild_id) = guild_id_str.parse::<u64>() else {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "Invalid guild ID." })),
        )
            .into_response();
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
        Ok(deleted) => Json(json!({ "ok": true, "deleted_entries": deleted })).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

async fn api_clear_guild_memory(
    State(state): State<Arc<WebState>>,
    Path(guild_id_str): Path<String>,
) -> Response {
    let Ok(guild_id) = guild_id_str.parse::<u64>() else {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "Invalid guild ID." })),
        )
            .into_response();
    };
    state.bot.memory().clear_guild(guild_id);
    Json(json!({ "ok": true })).into_response()
}

async fn api_update_check(
    State(state): State<Arc<WebState>>,
    Query(params): Query<HashMap<String, String>>,
) -> Json<Value> {
    let channel = params.get("channel").map(|s| s.as_str());
    let res = state.updater.check_for_updates(channel).await;
    Json(json!(res))
}

#[derive(Deserialize)]
struct UpdateApplyPayload {
    channel: Option<String>,
}

async fn api_update_apply(
    State(state): State<Arc<WebState>>,
    Json(payload): Json<UpdateApplyPayload>,
) -> Response {
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

async fn api_update_snapshots(State(state): State<Arc<WebState>>) -> Json<Value> {
    let snapshots = state.updater.list_snapshots().unwrap_or_default();
    Json(json!({ "snapshots": snapshots }))
}

async fn api_create_snapshot(State(state): State<Arc<WebState>>) -> Response {
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
async fn api_download_redacted_backup(State(state): State<Arc<WebState>>) -> Response {
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
    Path(guild_id_str): Path<String>,
) -> Response {
    let Ok(guild_id) = guild_id_str.parse::<u64>() else {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "Invalid guild ID." })),
        )
            .into_response();
    };

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

    (
        StatusCode::CREATED,
        Json(json!({ "subscription": created_sub })),
    )
        .into_response()
}

async fn api_delete_guild_reddit_sub(
    State(state): State<Arc<WebState>>,
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

    match state.storage.delete_reddit_subscription(sub_id, guild_id) {
        Ok(true) => Json(json!({ "success": true })).into_response(),
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

async fn api_generate_reddit_secret() -> Response {
    let mut bytes = [0u8; 24];
    rand::thread_rng().fill_bytes(&mut bytes);
    let secret = hex::encode(bytes);
    Json(json!({ "secret": secret })).into_response()
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
        return resp;
    }

    (StatusCode::NOT_FOUND, "404 Not Found").into_response()
}
