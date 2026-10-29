//! Hardened authentication, loopback OTP, Discord OAuth2 owner verification, session cookies,
//! CSRF verification, and bounded IP rate-limiting for `silas-web`.

use axum::http::{HeaderMap, Method, StatusCode};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use chrono::Utc;
use cookie::{Cookie, SameSite};
use dashmap::DashMap;
use hmac::{Hmac, Mac};
use parking_lot::Mutex;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use silas_core::config::AppSettings;
use std::net::IpAddr;
use std::time::{Duration, Instant};
use subtle::ConstantTimeEq;

type HmacSha256 = Hmac<Sha256>;

pub const SESSION_COOKIE_NAME: &str = "silas_session";
pub const OAUTH_STATE_COOKIE_NAME: &str = "silas_oauth_state";
pub const OAUTH_PKCE_COOKIE_NAME: &str = "silas_pkce";
pub const SESSION_TTL_SECS: i64 = 7 * 24 * 3600;
pub const OTP_TTL: Duration = Duration::from_secs(2 * 3600);
pub const MAX_TRACKED_IPS: usize = 4_096;

/// Returns true if `ip` is loopback, RFC1918 private LAN, link-local, or Tailscale CGNAT (`100.64.0.0/10`).
pub fn is_local_or_tailscale_ip(ip: IpAddr) -> bool {
    if ip.is_loopback() {
        return true;
    }
    match ip {
        IpAddr::V4(v4) => {
            let octets = v4.octets();
            v4.is_private()
                || v4.is_link_local()
                || (octets[0] == 100 && (octets[1] & 0b1100_0000) == 64) // 100.64.0.0/10 Tailscale CGNAT
        }
        IpAddr::V6(v6) => {
            if let Some(mapped) = v6.to_ipv4_mapped() {
                return is_local_or_tailscale_ip(IpAddr::V4(mapped));
            }
            let seg0 = v6.segments()[0];
            (seg0 & 0xfe00) == 0xfc00 || (seg0 & 0xffc0) == 0xfe80 // ULA fc00::/7 (incl. Tailscale fd7a:115c:a1e0::/48) or link-local
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionClaims {
    pub user_id: String,
    pub username: String,
    pub exp: i64,
    pub epoch: i64,
    #[serde(default)]
    pub is_owner: bool,
}

#[derive(Debug, Clone)]
struct OtpEntry {
    token_sha256: [u8; 32],
    expires_at: Instant,
}

#[derive(Debug, Clone)]
struct RateLimitEntry {
    failures: u32,
    window_start: Instant,
    locked_until: Option<Instant>,
}

/// Authentication and rate-limiting coordinator for the Web Console.
pub struct AuthManager {
    active_otp: Mutex<Option<OtpEntry>>,
    allow_lan_otp: bool,
    rate_limits: DashMap<IpAddr, RateLimitEntry>,
    http: reqwest::Client,
}

impl AuthManager {
    pub fn new(allow_lan_otp: bool) -> Result<Self, reqwest::Error> {
        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(8))
            .timeout(Duration::from_secs(15))
            .redirect(reqwest::redirect::Policy::none())
            .build()?;

        Ok(Self {
            active_otp: Mutex::new(None),
            allow_lan_otp,
            rate_limits: DashMap::new(),
            http,
        })
    }

    /// Generate a fresh 256-bit (32-byte) One-Time Login token, storing only its SHA-256 digest in memory.
    /// Returns the URL-safe raw token string to be printed to the terminal / opened in the local browser.
    pub fn generate_otp_token(&self) -> String {
        let mut raw_bytes = [0u8; 32];
        rand::thread_rng().fill_bytes(&mut raw_bytes);
        let token_str = URL_SAFE_NO_PAD.encode(raw_bytes);

        let mut hasher = Sha256::new();
        hasher.update(token_str.as_bytes());
        let digest_arr: [u8; 32] = hasher.finalize().into();

        let mut guard = self.active_otp.lock();
        *guard = Some(OtpEntry {
            token_sha256: digest_arr,
            expires_at: Instant::now() + OTP_TTL,
        });

        token_str
    }

    /// Verify and atomically consume the single-use OTP token.
    pub fn verify_and_consume_otp(
        &self,
        candidate_token: &str,
        client_ip: IpAddr,
    ) -> Result<(), &'static str> {
        if !self.allow_lan_otp && !is_local_or_tailscale_ip(client_ip) {
            return Err(
                "One-Time Login links are restricted to loopback/LAN/Tailscale unless --allow-lan-otp is set.",
            );
        }

        let mut hasher = Sha256::new();
        hasher.update(candidate_token.trim().as_bytes());
        let candidate_hash: [u8; 32] = hasher.finalize().into();

        let mut guard = self.active_otp.lock();
        let Some(entry) = guard.as_ref() else {
            return Err("One-Time Login token has already been used or was not generated.");
        };

        if Instant::now() > entry.expires_at {
            *guard = None;
            return Err(
                "One-Time Login token has expired. Restart Silas Blue or sign in with Discord OAuth2.",
            );
        }

        let matches: bool = entry.token_sha256.ct_eq(&candidate_hash).into();
        if !matches {
            return Err("Invalid One-Time Login token.");
        }

        // Single-use: consume immediately
        *guard = None;
        Ok(())
    }

    /// Check whether `client_ip` is currently rate-limited on `/auth/*` endpoints.
    pub fn check_rate_limit(&self, client_ip: IpAddr) -> Result<(), u64> {
        let now = Instant::now();
        if let Some(entry) = self.rate_limits.get(&client_ip) {
            if let Some(locked_until) = entry.locked_until {
                if now < locked_until {
                    return Err(locked_until.duration_since(now).as_secs().max(1));
                }
            }
        }
        Ok(())
    }

    /// Record a failed authentication attempt from `client_ip`.
    pub fn record_auth_failure(&self, client_ip: IpAddr) {
        let now = Instant::now();
        if self.rate_limits.len() >= MAX_TRACKED_IPS {
            self.rate_limits
                .retain(|_, v| now.duration_since(v.window_start) < Duration::from_secs(900));
            if self.rate_limits.len() >= MAX_TRACKED_IPS {
                if let Some(oldest_key) = self
                    .rate_limits
                    .iter()
                    .min_by_key(|e| e.value().window_start)
                    .map(|e| *e.key())
                {
                    self.rate_limits.remove(&oldest_key);
                }
            }
        }

        let mut entry = self.rate_limits.entry(client_ip).or_insert(RateLimitEntry {
            failures: 0,
            window_start: now,
            locked_until: None,
        });

        if now.duration_since(entry.window_start) > Duration::from_secs(300) {
            entry.failures = 0;
            entry.window_start = now;
            entry.locked_until = None;
        }

        entry.failures = entry.failures.saturating_add(1);
        if entry.failures >= 10 {
            entry.locked_until = Some(now + Duration::from_secs(900));
        }
    }

    /// Clear rate-limit failures for `client_ip` after a successful login.
    pub fn clear_rate_limit(&self, client_ip: IpAddr) {
        self.rate_limits.remove(&client_ip);
    }

    /// Create a signed HMAC-SHA256 session token string.
    pub fn create_session_token(
        &self,
        user_id: &str,
        username: &str,
        is_owner: bool,
        settings: &AppSettings,
    ) -> Result<String, String> {
        let claims = SessionClaims {
            user_id: user_id.to_string(),
            username: username.to_string(),
            exp: Utc::now().timestamp().saturating_add(SESSION_TTL_SECS),
            epoch: settings.session_epoch,
            is_owner,
        };
        let json_bytes = serde_json::to_vec(&claims).map_err(|e| e.to_string())?;
        let payload_b64 = URL_SAFE_NO_PAD.encode(&json_bytes);

        let mut mac =
            HmacSha256::new_from_slice(&settings.web_secret_key).map_err(|e| e.to_string())?;
        mac.update(payload_b64.as_bytes());
        let sig_hex = hex::encode(mac.finalize().into_bytes());

        Ok(format!("{payload_b64}.{sig_hex}"))
    }

    /// Verify a session cookie token in constant time and validate `exp` and `session_epoch`.
    pub fn verify_session_token(
        &self,
        token: &str,
        settings: &AppSettings,
    ) -> Option<SessionClaims> {
        let (payload_b64, sig_hex) = token.split_once('.')?;
        let sig_bytes = hex::decode(sig_hex).ok()?;

        let mut mac = HmacSha256::new_from_slice(&settings.web_secret_key).ok()?;
        mac.update(payload_b64.as_bytes());
        mac.verify_slice(&sig_bytes).ok()?;

        let decoded = URL_SAFE_NO_PAD.decode(payload_b64).ok()?;
        let claims: SessionClaims = serde_json::from_slice(&decoded).ok()?;

        if claims.exp <= Utc::now().timestamp() {
            return None;
        }
        if claims.epoch != settings.session_epoch {
            return None;
        }
        Some(claims)
    }

    /// Extract and verify `SessionClaims` from request headers.
    pub fn authenticate_request(
        &self,
        headers: &HeaderMap,
        settings: &AppSettings,
    ) -> Option<SessionClaims> {
        let token = extract_cookie(headers, SESSION_COOKIE_NAME)?;
        self.verify_session_token(&token, settings)
    }

    /// Check if a user ID is the configured bot owner or local console admin.
    pub fn is_bot_owner(&self, user_id: &str, settings: &AppSettings) -> bool {
        if user_id == "local-console" {
            return true;
        }
        settings.is_owner_id(user_id)
    }

    /// Exchange a Discord OAuth2 authorization code with PKCE verification and authenticate the user.
    /// Returns `(user_id_str, username, is_bot_owner)`.
    pub async fn verify_discord_oauth_callback(
        &self,
        code: &str,
        code_verifier: Option<&str>,
        redirect_uri: &str,
        effective_client_id: &str,
        settings: &AppSettings,
    ) -> Result<(String, String, bool), String> {
        if effective_client_id.trim().is_empty()
            || settings.discord_oauth_client_secret.trim().is_empty()
        {
            return Err("Discord OAuth2 Client ID / Secret are not configured.".to_string());
        }

        // 1. Exchange code for OAuth2 user access token (with optional PKCE code_verifier)
        let mut params = vec![
            ("client_id", effective_client_id.trim()),
            ("client_secret", settings.discord_oauth_client_secret.trim()),
            ("grant_type", "authorization_code"),
            ("code", code.trim()),
            ("redirect_uri", redirect_uri),
        ];
        if let Some(cv) = code_verifier {
            if !cv.trim().is_empty() {
                params.push(("code_verifier", cv.trim()));
            }
        }

        let token_resp = self
            .http
            .post("https://discord.com/api/v10/oauth2/token")
            .form(&params)
            .send()
            .await
            .map_err(|e| format!("Failed to reach Discord OAuth2 token endpoint: {e}"))?;

        if !token_resp.status().is_success() {
            return Err(format!(
                "Discord OAuth2 token exchange failed with HTTP {}",
                token_resp.status()
            ));
        }

        let token_json: serde_json::Value = token_resp
            .json()
            .await
            .map_err(|e| format!("Invalid OAuth2 token JSON: {e}"))?;
        let access_token = token_json
            .get("access_token")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "Missing access_token in Discord OAuth2 response".to_string())?;

        // 2. Fetch user identity from /users/@me
        let user_resp = self
            .http
            .get("https://discord.com/api/v10/users/@me")
            .header("Authorization", format!("Bearer {access_token}"))
            .send()
            .await
            .map_err(|e| format!("Failed to fetch Discord user profile: {e}"))?;

        if !user_resp.status().is_success() {
            return Err("Failed to authenticate Discord user profile.".to_string());
        }

        let user_json: serde_json::Value = user_resp
            .json()
            .await
            .map_err(|e| format!("Invalid user JSON: {e}"))?;
        let user_id_str = user_json
            .get("id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "Missing user id".to_string())?
            .to_string();
        let username = user_json
            .get("username")
            .and_then(|v| v.as_str())
            .unwrap_or("user")
            .to_string();

        // 3. Check if user is Bot Owner (either directly via bot_owner_id or application team owner/admin)
        let mut is_owner = false;
        if !settings.bot_owner_id.trim().is_empty() && settings.bot_owner_id.trim() == user_id_str {
            is_owner = true;
        } else if !settings.discord_token.trim().is_empty() {
            if let Ok(app_resp) = self
                .http
                .get("https://discord.com/api/v10/oauth2/applications/@me")
                .header(
                    "Authorization",
                    format!("Bot {}", settings.discord_token.trim()),
                )
                .send()
                .await
            {
                if app_resp.status().is_success() {
                    if let Ok(app_json) = app_resp.json::<serde_json::Value>().await {
                        if is_authorized_application_owner_or_admin(&app_json, &user_id_str) {
                            is_owner = true;
                        }
                    }
                }
            }
        }

        Ok((user_id_str, username, is_owner))
    }
}

/// Check if `user_id_str` is the application owner or an accepted Team Owner/Admin (`membership_state == 2`).
pub fn is_authorized_application_owner_or_admin(
    app_json: &serde_json::Value,
    user_id_str: &str,
) -> bool {
    if let Some(owner_id) = app_json
        .get("owner")
        .and_then(|o| o.get("id"))
        .and_then(|v| v.as_str())
    {
        if owner_id == user_id_str {
            return true;
        }
    }

    if let Some(members) = app_json
        .get("team")
        .and_then(|t| t.get("members"))
        .and_then(|m| m.as_array())
    {
        for member in members {
            let member_uid = member
                .get("user")
                .and_then(|u| u.get("id"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let state = member
                .get("membership_state")
                .and_then(|v| v.as_i64())
                .unwrap_or(0);
            let role = member
                .get("role")
                .and_then(|v| v.as_str())
                .unwrap_or("owner")
                .to_lowercase();

            if member_uid == user_id_str && state == 2 && (role == "owner" || role == "admin") {
                return true;
            }
        }
    }

    false
}

/// Build an `HttpOnly; SameSite=Lax; Path=/` Set-Cookie header string.
pub fn build_session_cookie(token: &str, is_https: bool) -> String {
    let mut cookie = Cookie::build((SESSION_COOKIE_NAME, token.to_string()))
        .path("/")
        .http_only(true)
        .same_site(SameSite::Lax)
        .max_age(cookie::time::Duration::seconds(SESSION_TTL_SECS));
    if is_https {
        cookie = cookie.secure(true);
    }
    cookie.build().to_string()
}

pub fn build_oauth_cookie(name: &str, value: &str, is_https: bool) -> String {
    let mut cookie = Cookie::build((name.to_string(), value.to_string()))
        .path("/")
        .http_only(true)
        .same_site(SameSite::Lax)
        .max_age(cookie::time::Duration::minutes(10));
    if is_https {
        cookie = cookie.secure(true);
    }
    cookie.build().to_string()
}

pub fn build_clear_cookie(name: &str, is_https: bool) -> String {
    let mut cookie = Cookie::build((name.to_string(), String::new()))
        .path("/")
        .http_only(true)
        .same_site(SameSite::Lax)
        .max_age(cookie::time::Duration::ZERO);
    if is_https {
        cookie = cookie.secure(true);
    }
    cookie.build().to_string()
}

/// Extract a named cookie from an HTTP `HeaderMap`.
pub fn extract_cookie(headers: &HeaderMap, name: &str) -> Option<String> {
    for val in headers.get_all(axum::http::header::COOKIE) {
        let Ok(raw) = val.to_str() else {
            continue;
        };
        for part in raw.split(';') {
            if let Ok(c) = Cookie::parse(part.trim()) {
                if c.name() == name {
                    return Some(c.value().to_string());
                }
            }
        }
    }
    None
}

/// Verify CSRF protection on state-mutating requests (`POST`, `PUT`, `PATCH`, `DELETE`):
/// 1. Requires `X-Requested-With: SilasBlue-SPA`.
/// 2. If `Origin` is present, its host must match the `Host` header.
pub fn verify_csrf(method: &Method, headers: &HeaderMap) -> Result<(), (StatusCode, &'static str)> {
    if matches!(method, &Method::GET | &Method::HEAD | &Method::OPTIONS) {
        return Ok(());
    }

    let xrw = headers
        .get("x-requested-with")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if xrw != "SilasBlue-SPA" {
        return Err((
            StatusCode::FORBIDDEN,
            "CSRF validation failed: missing X-Requested-With: SilasBlue-SPA header.",
        ));
    }

    if let Some(origin_val) = headers
        .get(axum::http::header::ORIGIN)
        .and_then(|v| v.to_str().ok())
    {
        if let Some(host_val) = headers
            .get(axum::http::header::HOST)
            .and_then(|v| v.to_str().ok())
        {
            if let Ok(parsed_origin) = url::Url::parse(origin_val) {
                let origin_host_port = match (parsed_origin.host_str(), parsed_origin.port()) {
                    (Some(h), Some(p)) => format!("{h}:{p}"),
                    (Some(h), None) => h.to_string(),
                    _ => String::new(),
                };
                if !origin_host_port.eq_ignore_ascii_case(host_val.trim()) {
                    return Err((
                        StatusCode::FORBIDDEN,
                        "CSRF validation failed: Origin header does not match Host.",
                    ));
                }
            } else {
                return Err((
                    StatusCode::FORBIDDEN,
                    "CSRF validation failed: malformed Origin.",
                ));
            }
        }
    }

    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    #[test]
    fn test_otp_single_use_and_loopback_enforcement() {
        let auth = AuthManager::new(false).unwrap();
        let token = auth.generate_otp_token();

        // Public internet IP rejected when allow_lan_otp is false
        let public_ip = IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8));
        assert!(auth.verify_and_consume_otp(&token, public_ip).is_err());

        // LAN and Tailscale IPs are recognized
        assert!(is_local_or_tailscale_ip(IpAddr::V4(Ipv4Addr::new(
            192, 168, 4, 1
        ))));
        assert!(is_local_or_tailscale_ip(IpAddr::V4(Ipv4Addr::new(
            100, 115, 39, 47
        ))));

        // Loopback IP succeeds once, then second attempt fails (single-use)
        let loopback = IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1));
        assert!(auth.verify_and_consume_otp(&token, loopback).is_ok());
        assert!(auth.verify_and_consume_otp(&token, loopback).is_err());
    }

    #[test]
    fn test_session_token_roundtrip_and_epoch_invalidation() {
        let auth = AuthManager::new(false).unwrap();
        let mut settings = AppSettings::default();
        settings.web_secret_key = b"0123456789abcdef0123456789abcdef".to_vec();
        settings.session_epoch = 1;

        let token = auth
            .create_session_token("12345", "local-admin", true, &settings)
            .unwrap();
        let claims = auth.verify_session_token(&token, &settings).unwrap();
        assert_eq!(claims.user_id, "12345");
        assert!(claims.is_owner);
        assert_eq!(claims.epoch, 1);

        // Incrementing session_epoch immediately invalidates existing cookie
        settings.session_epoch = 2;
        assert!(auth.verify_session_token(&token, &settings).is_none());
    }

    #[test]
    fn test_team_membership_role_check() {
        let app_json = serde_json::json!({
            "id": "999",
            "team": {
                "members": [
                    {
                        "membership_state": 2,
                        "role": "read_only",
                        "user": { "id": "111" }
                    },
                    {
                        "membership_state": 1,
                        "role": "owner",
                        "user": { "id": "222" }
                    },
                    {
                        "membership_state": 2,
                        "role": "admin",
                        "user": { "id": "333" }
                    }
                ]
            }
        });
        assert!(!is_authorized_application_owner_or_admin(&app_json, "111")); // read_only rejected
        assert!(!is_authorized_application_owner_or_admin(&app_json, "222")); // unaccepted invite rejected
        assert!(is_authorized_application_owner_or_admin(&app_json, "333")); // accepted admin allowed
    }
}
