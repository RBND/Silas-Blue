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
use axum::routing::{delete, get, post, put};
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
use silas_core::tagscript::{process_tagscript, TagScriptContext};
use silas_storage::{
    AutomodConfig, GreetingsConfig, LoggingConfig, NewCustomTrigger, NewModCase,
    NewReactionRoleEntry, NewReactionRoleGroup, Storage,
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
// Moderation, Automod, Logging, Greetings, Reaction Roles & Triggers Endpoints
// ============================================================================

#[derive(Debug, Deserialize)]
pub struct ModCasesQuery {
    pub user_id: Option<u64>,
    pub limit: Option<usize>,
}

async fn api_list_mod_cases(
    State(state): State<Arc<WebState>>,
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
    pub user_id: u64,
    pub moderator_id: Option<u64>,
    pub action_type: String,
    pub reason: Option<String>,
    pub duration_seconds: Option<i64>,
    pub expires_at: Option<i64>,
}

async fn api_create_mod_case(
    State(state): State<Arc<WebState>>,
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

    let new_case = NewModCase {
        guild_id,
        user_id: payload.user_id,
        moderator_id: payload.moderator_id.unwrap_or(0),
        action_type: payload.action_type,
        reason: payload
            .reason
            .unwrap_or_else(|| "No reason provided".to_string()),
        duration_seconds: payload.duration_seconds,
        created_at: None,
        expires_at: payload.expires_at,
    };

    match state.storage.create_mod_case(&new_case) {
        Ok(created) => (StatusCode::CREATED, Json(json!({ "case": created }))).into_response(),
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

    let updated_by_id = state
        .storage
        .update_case_reason_by_id(case_id, &payload.reason)
        .unwrap_or(false);
    let success = if updated_by_id {
        true
    } else {
        state
            .storage
            .update_case_reason(guild_id, case_id, &payload.reason)
            .unwrap_or(false)
    };

    if success {
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
    Path((_guild_id_str, case_id_str)): Path<(String, String)>,
) -> Response {
    let Ok(case_id) = case_id_str.parse::<i64>() else {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "Invalid case ID." })),
        )
            .into_response();
    };

    match state.storage.deactivate_case(case_id) {
        Ok(true) => Json(json!({ "success": true, "case_id": case_id })).into_response(),
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

    if let Some(user_id) = query.user_id {
        let strikes = state
            .storage
            .get_user_strikes(guild_id, user_id)
            .unwrap_or_default();
        let active_count = state
            .storage
            .get_active_strikes_count(guild_id, user_id)
            .unwrap_or(0);
        Json(json!({ "user_id": user_id, "active_count": active_count, "strikes": strikes }))
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
        Ok(count) => Json(json!({
            "success": true,
            "decayed_count": count,
            "cutoff_epoch": cutoff
        }))
        .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": format!("{e}") })),
        )
            .into_response(),
    }
}

async fn api_get_automod_config(
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

    match state.storage.save_automod_config(&config) {
        Ok(()) => Json(json!({ "success": true, "config": config })).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": format!("{e}") })),
        )
            .into_response(),
    }
}

async fn api_get_logging_config(
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

    match state.storage.save_logging_config(&config) {
        Ok(()) => Json(json!({ "success": true, "config": config })).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": format!("{e}") })),
        )
            .into_response(),
    }
}

async fn api_get_greetings_config(
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

    match state.storage.save_greetings_config(&config) {
        Ok(()) => Json(json!({ "success": true, "config": config })).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": format!("{e}") })),
        )
            .into_response(),
    }
}

#[derive(Debug, Deserialize)]
pub struct CreateReactionRoleGroupRequest {
    pub channel_id: u64,
    pub message_id: u64,
    pub group_type: Option<String>,
    pub required_role_id: Option<u64>,
    pub component_type: Option<String>,
    #[serde(default)]
    pub entries: Vec<CreateReactionRoleEntryItem>,
}

#[derive(Debug, Deserialize)]
pub struct CreateReactionRoleEntryItem {
    pub emoji_or_custom_id: String,
    pub role_id: u64,
    pub button_label: Option<String>,
    pub button_style: Option<String>,
}

async fn api_list_reaction_roles(
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

    (StatusCode::CREATED, Json(json!({ "group": created_group }))).into_response()
}

async fn api_delete_reaction_role_group(
    State(state): State<Arc<WebState>>,
    Path((_guild_id_str, group_id_str)): Path<(String, String)>,
) -> Response {
    let Ok(group_id) = group_id_str.parse::<i64>() else {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "Invalid group ID." })),
        )
            .into_response();
    };

    match state.storage.delete_reaction_role_group(group_id) {
        Ok(true) => Json(json!({ "success": true, "group_id": group_id })).into_response(),
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
    #[serde(default)]
    pub allowed_roles: Vec<u64>,
    #[serde(default)]
    pub ignored_channels: Vec<u64>,
    pub delete_trigger_message: Option<bool>,
    pub reply_in_dm: Option<bool>,
    pub enabled: Option<bool>,
}

async fn api_list_custom_triggers(
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
        Ok(trigger) => Json(json!({ "success": true, "trigger": trigger })).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": format!("{e}") })),
        )
            .into_response(),
    }
}

async fn api_delete_custom_trigger(
    State(state): State<Arc<WebState>>,
    Path((guild_id_str, trigger_name)): Path<(String, String)>,
) -> Response {
    let Ok(guild_id) = guild_id_str.parse::<u64>() else {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "Invalid guild ID." })),
        )
            .into_response();
    };

    match state.storage.delete_custom_trigger(guild_id, &trigger_name) {
        Ok(true) => Json(json!({ "success": true, "trigger_name": trigger_name })).into_response(),
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
    pub author_id: Option<u64>,
    pub author_name: Option<String>,
    pub author_avatar: Option<String>,
    pub target_id: Option<u64>,
    pub target_name: Option<String>,
    pub guild_id: Option<u64>,
    pub guild_name: Option<String>,
    pub guild_members: Option<u64>,
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
}
