//! Prebuilt binary release self-updater (`silas-updater`) with Ed25519 signature verification,
//! streaming SHA-256 verification, SQLite WAL-safe snapshot backups, and pre-SQLite boot health rollback.

use chrono::Utc;
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use futures_util::StreamExt;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use silas_core::events::{TelemetryBus, TelemetryEvent};
use silas_storage::Storage;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use thiserror::Error;

/// Current compiled semantic version of Silas Blue.
pub const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");

/// GitHub repository slug for release checks.
pub const GITHUB_REPO: &str = "RBND/Silas-Blue";

/// Maximum permitted binary asset size (100 MB) to prevent disk exhaustion.
pub const MAX_BINARY_SIZE: u64 = 100 * 1024 * 1024;

/// Compiled-in 32-byte Ed25519 public key for verifying `SHA256SUMS.txt.sig` release manifests.
pub const RELEASE_SIGNING_PUBLIC_KEY: [u8; 32] = [
    0xd7, 0x5a, 0x98, 0x01, 0x82, 0xb1, 0x0a, 0xb7,
    0xd5, 0x4b, 0xfe, 0xd3, 0xc9, 0x64, 0x07, 0x3a,
    0x0e, 0xe1, 0x72, 0xf3, 0xda, 0xa6, 0x23, 0x25,
    0xaf, 0x02, 0x1a, 0x68, 0xf7, 0x07, 0x51, 0x1a,
];

#[derive(Debug, Error)]
pub enum UpdaterError {
    #[error("HTTP request failed: {0}")]
    Http(#[from] reqwest::Error),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Storage error: {0}")]
    Storage(#[from] silas_storage::StorageError),
    #[error("JSON serialization error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("Cryptographic verification failed: {0}")]
    Verification(String),
    #[error("Update error: {0}")]
    Message(String),
}

/// Out-of-band pre-SQLite boot health state persisted at `data/boot_state.json`.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct BootState {
    pub pending_update: bool,
    pub target_version: String,
    pub previous_version: String,
    pub previous_binary_path: String,
    pub previous_db_backup_path: String,
    pub boot_attempts: u32,
    pub updated_at: i64,
}

impl BootState {
    /// Load `boot_state.json` from the given path if it exists.
    pub fn load(path: &Path) -> Result<Self, UpdaterError> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let raw = std::fs::read_to_string(path)?;
        let state: Self = serde_json::from_str(&raw)?;
        Ok(state)
    }

    /// Atomically write `boot_state.json` to disk.
    pub fn save(&self, path: &Path) -> Result<(), UpdaterError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let tmp_path = path.with_extension("json.tmp");
        let serialized = serde_json::to_string_pretty(self)?;
        std::fs::write(&tmp_path, serialized)?;
        std::fs::rename(&tmp_path, path)?;
        Ok(())
    }

    /// Execute line-1 pre-SQLite boot check.
    ///
    /// If `pending_update == true`:
    /// - When `boot_attempts >= 2`, rolls back the SQLite database (removing `-wal` and `-shm` first)
    ///   and restores the previous binary via `self_replace::self_replace`, then clears `pending_update`.
    /// - Otherwise, increments `boot_attempts` and persists `boot_state.json` before SQLite opens.
    pub fn check_or_rollback_pre_sqlite(
        boot_state_path: &Path,
        db_path: &Path,
    ) -> Result<Option<String>, UpdaterError> {
        let mut state = match Self::load(boot_state_path) {
            Ok(s) => s,
            Err(_) => return Ok(None),
        };

        if !state.pending_update {
            return Ok(None);
        }

        if state.boot_attempts >= 2 {
            // Crash-loop detected! Execute out-of-band rollback before opening SQLite.
            let wal_path = PathBuf::from(format!("{}-wal", db_path.display()));
            let shm_path = PathBuf::from(format!("{}-shm", db_path.display()));
            let _ = std::fs::remove_file(&wal_path);
            let _ = std::fs::remove_file(&shm_path);

            let prev_db = PathBuf::from(&state.previous_db_backup_path);
            if prev_db.exists() {
                std::fs::copy(&prev_db, db_path)?;
            }

            let prev_bin = PathBuf::from(&state.previous_binary_path);
            if prev_bin.exists() {
                let _ = self_replace::self_replace(&prev_bin);
            }

            let rolled_back_from = state.target_version.clone();
            let restored_to = state.previous_version.clone();
            state.pending_update = false;
            state.boot_attempts = 0;
            state.save(boot_state_path)?;

            return Ok(Some(format!(
                "Crash loop detected on v{rolled_back_from}; automatically rolled back to v{restored_to}."
            )));
        }

        state.boot_attempts = state.boot_attempts.saturating_add(1);
        state.save(boot_state_path)?;
        Ok(None)
    }

    /// Mark the current boot as healthy after SQLite migrations and startup succeed.
    pub fn mark_boot_healthy(boot_state_path: &Path) -> Result<(), UpdaterError> {
        if !boot_state_path.exists() {
            return Ok(());
        }
        let mut state = Self::load(boot_state_path)?;
        if state.pending_update || state.boot_attempts > 0 {
            state.pending_update = false;
            state.boot_attempts = 0;
            state.save(boot_state_path)?;
        }
        Ok(())
    }
}

/// Information about a release check against GitHub Releases.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReleaseCheckResult {
    pub update_available: bool,
    pub current_version: String,
    pub latest_version: String,
    pub channel: String,
    pub release_url: String,
    pub release_notes: String,
    pub published_at: String,
    pub binary_asset_name: Option<String>,
    pub binary_download_url: Option<String>,
    pub checksums_url: Option<String>,
    pub signature_url: Option<String>,
    pub error: Option<String>,
}

/// Metadata for a local pre-update or manual snapshot in `backups/`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotInfo {
    pub id: String,
    pub version: String,
    pub created_at: String,
    pub has_db_backup: bool,
    pub has_binary_backup: bool,
    pub size_bytes: u64,
}

#[derive(Debug, Deserialize)]
struct GithubRelease {
    tag_name: String,
    html_url: String,
    body: Option<String>,
    prerelease: bool,
    draft: bool,
    published_at: Option<String>,
    assets: Vec<GithubAsset>,
}

#[derive(Debug, Deserialize)]
struct GithubAsset {
    name: String,
    browser_download_url: String,
    size: u64,
}

/// Self-updater and snapshot manager for Silas Blue.
#[derive(Clone)]
pub struct BotUpdater {
    storage: Storage,
    telemetry: TelemetryBus,
    backups_dir: PathBuf,
    boot_state_path: PathBuf,
    http: Client,
}

impl BotUpdater {
    pub fn new(
        storage: Storage,
        telemetry: TelemetryBus,
        backups_dir: impl Into<PathBuf>,
        boot_state_path: impl Into<PathBuf>,
    ) -> Result<Self, UpdaterError> {
        let http = Client::builder()
            .user_agent(format!("SilasBlue/{CURRENT_VERSION} (+https://github.com/{GITHUB_REPO})"))
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(120))
            .build()?;

        Ok(Self {
            storage,
            telemetry,
            backups_dir: backups_dir.into(),
            boot_state_path: boot_state_path.into(),
            http,
        })
    }

    pub fn current_version(&self) -> &'static str {
        CURRENT_VERSION
    }

    pub fn backups_dir(&self) -> &Path {
        &self.backups_dir
    }

    /// Determine the expected release binary asset name for the current OS and CPU architecture.
    pub fn platform_asset_name() -> &'static str {
        #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
        {
            "silas-blue-windows-x86_64.exe"
        }
        #[cfg(all(target_os = "windows", target_arch = "aarch64"))]
        {
            "silas-blue-windows-aarch64.exe"
        }
        #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
        {
            "silas-blue-linux-x86_64"
        }
        #[cfg(all(target_os = "linux", target_arch = "aarch64"))]
        {
            "silas-blue-linux-aarch64"
        }
        #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
        {
            "silas-blue-macos-x86_64"
        }
        #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
        {
            "silas-blue-macos-aarch64"
        }
        #[cfg(not(any(
            all(target_os = "windows", target_arch = "x86_64"),
            all(target_os = "windows", target_arch = "aarch64"),
            all(target_os = "linux", target_arch = "x86_64"),
            all(target_os = "linux", target_arch = "aarch64"),
            all(target_os = "macos", target_arch = "x86_64"),
            all(target_os = "macos", target_arch = "aarch64")
        )))]
        {
            "silas-blue-generic"
        }
    }

    /// Check GitHub Releases for a newer version on the requested channel (`"stable"` or `"prerelease"`).
    pub async fn check_for_updates(&self, channel_override: Option<&str>) -> ReleaseCheckResult {
        let settings = self.storage.get_app_settings();
        let channel = channel_override
            .unwrap_or(&settings.update_channel)
            .trim()
            .to_lowercase();
        let include_prerelease = channel == "prerelease" || channel == "beta";

        let url = format!("https://api.github.com/repos/{GITHUB_REPO}/releases?per_page=15");
        let resp = match self
            .http
            .get(&url)
            .header("Accept", "application/vnd.github+json")
            .send()
            .await
        {
            Ok(r) => r,
            Err(e) => {
                return ReleaseCheckResult {
                    update_available: false,
                    current_version: CURRENT_VERSION.to_string(),
                    latest_version: CURRENT_VERSION.to_string(),
                    channel,
                    release_url: format!("https://github.com/{GITHUB_REPO}/releases"),
                    release_notes: String::new(),
                    published_at: String::new(),
                    binary_asset_name: None,
                    binary_download_url: None,
                    checksums_url: None,
                    signature_url: None,
                    error: Some(format!("Failed to reach GitHub Releases API: {e}")),
                };
            }
        };

        if !resp.status().is_success() {
            return ReleaseCheckResult {
                update_available: false,
                current_version: CURRENT_VERSION.to_string(),
                latest_version: CURRENT_VERSION.to_string(),
                channel,
                release_url: format!("https://github.com/{GITHUB_REPO}/releases"),
                release_notes: String::new(),
                published_at: String::new(),
                binary_asset_name: None,
                binary_download_url: None,
                checksums_url: None,
                signature_url: None,
                error: Some(format!("GitHub API returned HTTP {}", resp.status())),
            };
        }

        let releases: Vec<GithubRelease> = match resp.json().await {
            Ok(list) => list,
            Err(e) => {
                return ReleaseCheckResult {
                    update_available: false,
                    current_version: CURRENT_VERSION.to_string(),
                    latest_version: CURRENT_VERSION.to_string(),
                    channel,
                    release_url: format!("https://github.com/{GITHUB_REPO}/releases"),
                    release_notes: String::new(),
                    published_at: String::new(),
                    binary_asset_name: None,
                    binary_download_url: None,
                    checksums_url: None,
                    signature_url: None,
                    error: Some(format!("Invalid GitHub Releases JSON: {e}")),
                };
            }
        };

        let candidate = releases
            .into_iter()
            .filter(|r| !r.draft && (include_prerelease || !r.prerelease))
            .max_by(|a, b| compare_versions(&a.tag_name, &b.tag_name));

        let Some(latest) = candidate else {
            return ReleaseCheckResult {
                update_available: false,
                current_version: CURRENT_VERSION.to_string(),
                latest_version: CURRENT_VERSION.to_string(),
                channel,
                release_url: format!("https://github.com/{GITHUB_REPO}/releases"),
                release_notes: "No published releases found for this channel.".to_string(),
                published_at: String::new(),
                binary_asset_name: None,
                binary_download_url: None,
                checksums_url: None,
                signature_url: None,
                error: None,
            };
        };

        let latest_clean = latest.tag_name.trim_start_matches('v').to_string();
        let update_available =
            compare_versions(&latest_clean, CURRENT_VERSION) == std::cmp::Ordering::Greater;

        let expected_asset = Self::platform_asset_name();
        let binary_asset = latest
            .assets
            .iter()
            .find(|a| a.name.eq_ignore_ascii_case(expected_asset) && a.size <= MAX_BINARY_SIZE);
        let checksums_asset = latest
            .assets
            .iter()
            .find(|a| a.name.eq_ignore_ascii_case("SHA256SUMS.txt"));
        let sig_asset = latest
            .assets
            .iter()
            .find(|a| a.name.eq_ignore_ascii_case("SHA256SUMS.txt.sig"));

        ReleaseCheckResult {
            update_available,
            current_version: CURRENT_VERSION.to_string(),
            latest_version: latest_clean,
            channel,
            release_url: latest.html_url,
            release_notes: latest.body.unwrap_or_default(),
            published_at: latest.published_at.unwrap_or_default(),
            binary_asset_name: binary_asset.map(|a| a.name.clone()),
            binary_download_url: binary_asset.map(|a| a.browser_download_url.clone()),
            checksums_url: checksums_asset.map(|a| a.browser_download_url.clone()),
            signature_url: sig_asset.map(|a| a.browser_download_url.clone()),
            error: None,
        }
    }

    /// Create a local snapshot of `silas.db` (and current binary if readable) inside `backups/`.
    pub async fn create_snapshot(&self, prefix: &str) -> Result<SnapshotInfo, UpdaterError> {
        let timestamp = Utc::now().format("%Y%m%d_%H%M%S").to_string();
        let id = format!("{prefix}_v{CURRENT_VERSION}_{timestamp}");
        let snap_dir = self.backups_dir.join(&id);
        std::fs::create_dir_all(&snap_dir)?;

        let db_backup_path = snap_dir.join("silas.db");
        self.storage.create_internal_backup(&db_backup_path)?;

        let mut has_binary_backup = false;
        if let Ok(exe_path) = std::env::current_exe() {
            let bin_name = if cfg!(windows) {
                "silas-blue.exe"
            } else {
                "silas-blue"
            };
            let bin_dest = snap_dir.join(bin_name);
            if std::fs::copy(&exe_path, &bin_dest).is_ok() {
                has_binary_backup = true;
            }
        }

        let meta_json = serde_json::json!({
            "id": id,
            "version": CURRENT_VERSION,
            "created_at": Utc::now().to_rfc3339(),
            "has_db_backup": true,
            "has_binary_backup": has_binary_backup,
        });
        std::fs::write(
            snap_dir.join("snapshot.json"),
            serde_json::to_string_pretty(&meta_json)?,
        )?;

        let size_bytes = dir_size_bytes(&snap_dir);

        Ok(SnapshotInfo {
            id,
            version: CURRENT_VERSION.to_string(),
            created_at: Utc::now().to_rfc3339(),
            has_db_backup: true,
            has_binary_backup,
            size_bytes,
        })
    }

    /// List all available snapshots in `backups/`, sorted newest first.
    pub fn list_snapshots(&self) -> Result<Vec<SnapshotInfo>, UpdaterError> {
        if !self.backups_dir.exists() {
            return Ok(Vec::new());
        }
        let mut results = Vec::new();
        for entry in std::fs::read_dir(&self.backups_dir)? {
            let entry = entry?;
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let Some(dir_name) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            if dir_name == "v3_local_snapshot" {
                continue;
            }
            let db_exists = path.join("silas.db").exists();
            let bin_exists =
                path.join("silas-blue.exe").exists() || path.join("silas-blue").exists();
            if !db_exists && !bin_exists {
                continue;
            }
            let size_bytes = dir_size_bytes(&path);
            let mut version = env!("CARGO_PKG_VERSION").to_string();
            let mut created_at = String::new();
            let meta_file = path.join("snapshot.json");
            if meta_file.exists() {
                if let Ok(raw) = std::fs::read_to_string(&meta_file) {
                    if let Ok(val) = serde_json::from_str::<serde_json::Value>(&raw) {
                        if let Some(v) = val.get("version").and_then(|v| v.as_str()) {
                            version = v.to_string();
                        }
                        if let Some(c) = val.get("created_at").and_then(|v| v.as_str()) {
                            created_at = c.to_string();
                        }
                    }
                }
            }
            results.push(SnapshotInfo {
                id: dir_name.to_string(),
                version,
                created_at,
                has_db_backup: db_exists,
                has_binary_backup: bin_exists,
                size_bytes,
            });
        }
        results.sort_by(|a, b| b.id.cmp(&a.id));
        Ok(results)
    }

    /// Verify Ed25519 signature over `SHA256SUMS.txt` using the compiled-in public key.
    pub fn verify_manifest_signature(
        manifest_bytes: &[u8],
        sig_raw: &[u8],
    ) -> Result<(), UpdaterError> {
        let verifying_key = VerifyingKey::from_bytes(&RELEASE_SIGNING_PUBLIC_KEY)
            .map_err(|e| UpdaterError::Verification(format!("Invalid public key: {e}")))?;

        // Accept either raw 64-byte signature or hex-encoded 128-char signature
        let sig_bytes: [u8; 64] = if sig_raw.len() == 64 {
            let mut arr = [0u8; 64];
            arr.copy_from_slice(sig_raw);
            arr
        } else {
            let text = std::str::from_utf8(sig_raw)
                .map_err(|_| UpdaterError::Verification("Invalid signature encoding".to_string()))?
                .trim();
            let decoded = hex::decode(text)
                .map_err(|e| UpdaterError::Verification(format!("Invalid hex signature: {e}")))?;
            if decoded.len() != 64 {
                return Err(UpdaterError::Verification(format!(
                    "Expected 64-byte Ed25519 signature, got {} bytes",
                    decoded.len()
                )));
            }
            let mut arr = [0u8; 64];
            arr.copy_from_slice(&decoded);
            arr
        };

        let signature = Signature::from_bytes(&sig_bytes);
        verifying_key
            .verify(manifest_bytes, &signature)
            .map_err(|e| UpdaterError::Verification(format!("Ed25519 signature mismatch: {e}")))
    }

    /// Download, cryptographically verify (Ed25519 + SHA-256), snapshot `silas.db`, and atomically
    /// replace the running binary.
    pub async fn apply_update(&self, channel_override: Option<&str>) -> Result<String, UpdaterError> {
        let check = self.check_for_updates(channel_override).await;
        if let Some(err) = check.error {
            return Err(UpdaterError::Message(err));
        }
        if !check.update_available {
            return Err(UpdaterError::Message(format!(
                "Already up to date (v{}).",
                check.current_version
            )));
        }

        let asset_name = check.binary_asset_name.ok_or_else(|| {
            UpdaterError::Message(format!(
                "Release v{} does not include a prebuilt binary for '{}'.",
                check.latest_version,
                Self::platform_asset_name()
            ))
        })?;
        let binary_url = check.binary_download_url.ok_or_else(|| {
            UpdaterError::Message("Missing binary download URL in release.".to_string())
        })?;
        let checksums_url = check.checksums_url.ok_or_else(|| {
            UpdaterError::Verification(
                "Release is missing SHA256SUMS.txt manifest; refusing unverified update.".to_string(),
            )
        })?;
        let sig_url = check.signature_url.ok_or_else(|| {
            UpdaterError::Verification(
                "Release is missing SHA256SUMS.txt.sig Ed25519 signature; refusing unverified update."
                    .to_string(),
            )
        })?;

        // 1. Download SHA256SUMS.txt and SHA256SUMS.txt.sig
        let manifest_bytes = self
            .http
            .get(&checksums_url)
            .send()
            .await?
            .error_for_status()?
            .bytes()
            .await?;
        let sig_bytes = self
            .http
            .get(&sig_url)
            .send()
            .await?
            .error_for_status()?
            .bytes()
            .await?;

        // 2. Verify Ed25519 signature
        Self::verify_manifest_signature(&manifest_bytes, &sig_bytes)?;

        // 3. Extract expected SHA-256 hex digest for our platform asset
        let manifest_str = std::str::from_utf8(&manifest_bytes)
            .map_err(|_| UpdaterError::Verification("Non-UTF8 SHA256SUMS.txt".to_string()))?;
        let expected_sha256 = parse_sha256_for_asset(manifest_str, &asset_name).ok_or_else(|| {
            UpdaterError::Verification(format!(
                "Asset '{asset_name}' not listed in verified SHA256SUMS.txt"
            ))
        })?;

        // 4. Create pre-update snapshot of SQLite DB and current binary
        let snapshot = self.create_snapshot("pre_update").await?;
        let snap_dir = self.backups_dir.join(&snapshot.id);

        // 5. Stream download new binary to temporary file while computing SHA-256 and enforcing MAX_BINARY_SIZE
        let staged_path = snap_dir.join(format!("{asset_name}.staged"));
        let mut file = std::fs::File::create(&staged_path)?;
        let resp = self
            .http
            .get(&binary_url)
            .send()
            .await?
            .error_for_status()?;

        let mut stream = resp.bytes_stream();
        let mut hasher = Sha256::new();
        let mut total_bytes: u64 = 0;

        while let Some(chunk_res) = stream.next().await {
            let chunk = chunk_res?;
            total_bytes = total_bytes.saturating_add(chunk.len() as u64);
            if total_bytes > MAX_BINARY_SIZE {
                let _ = std::fs::remove_file(&staged_path);
                return Err(UpdaterError::Verification(format!(
                    "Downloaded binary exceeded maximum size limit ({MAX_BINARY_SIZE} bytes)"
                )));
            }
            hasher.update(&chunk);
            std::io::Write::write_all(&mut file, &chunk)?;
        }
        std::io::Write::flush(&mut file)?;
        drop(file);

        let actual_sha256 = hex::encode(hasher.finalize());
        if !actual_sha256.eq_ignore_ascii_case(&expected_sha256) {
            let _ = std::fs::remove_file(&staged_path);
            return Err(UpdaterError::Verification(format!(
                "SHA-256 digest mismatch for '{asset_name}': expected {expected_sha256}, got {actual_sha256}"
            )));
        }

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&staged_path, std::fs::Permissions::from_mode(0o755));
        }

        // 6. Record pre-SQLite BootState before replacing binary
        let prev_bin_name = if cfg!(windows) {
            "silas-blue.exe"
        } else {
            "silas-blue"
        };
        let boot_state = BootState {
            pending_update: true,
            target_version: check.latest_version.clone(),
            previous_version: CURRENT_VERSION.to_string(),
            previous_binary_path: snap_dir.join(prev_bin_name).to_string_lossy().to_string(),
            previous_db_backup_path: snap_dir.join("silas.db").to_string_lossy().to_string(),
            boot_attempts: 0,
            updated_at: Utc::now().timestamp(),
        };
        boot_state.save(&self.boot_state_path)?;

        // 7. Atomically replace current executable
        self_replace::self_replace(&staged_path).map_err(|e| {
            UpdaterError::Message(format!("Failed to replace executable via self-replace: {e}"))
        })?;
        let _ = std::fs::remove_file(&staged_path);

        self.telemetry.emit(TelemetryEvent::System {
            message: format!(
                "Successfully staged update from v{} to v{} (snapshot: {}). Ready for restart.",
                CURRENT_VERSION, check.latest_version, snapshot.id
            ),
        });

        Ok(format!(
            "Updated binary from v{} to v{} (snapshot `{}`). Restart the process to boot into v{}.",
            CURRENT_VERSION, check.latest_version, snapshot.id, check.latest_version
        ))
    }

    /// Spawn a background periodic update check task.
    pub fn spawn_auto_update_loop(self: Arc<Self>) {
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(3600));
            // Consume immediate first tick
            interval.tick().await;

            let mut hours_elapsed: u32 = 0;
            loop {
                interval.tick().await;
                hours_elapsed = hours_elapsed.saturating_add(1);

                let settings = self.storage.get_app_settings();
                if !settings.auto_update_enabled {
                    continue;
                }
                let target_hours = settings.auto_update_interval_hours.max(1);
                if hours_elapsed < target_hours {
                    continue;
                }
                hours_elapsed = 0;

                let check = self.check_for_updates(None).await;
                if check.update_available {
                    self.telemetry.emit(TelemetryEvent::System {
                        message: format!(
                            "Auto-update check found new release v{} (current: v{}). Applying update...",
                            check.latest_version, check.current_version
                        ),
                    });
                    if let Err(e) = self.apply_update(None).await {
                        self.telemetry.emit(TelemetryEvent::System {
                            message: format!("Auto-update failed: {e}"),
                        });
                    }
                }
            }
        });
    }
}

/// Parse a `SHA256SUMS.txt` manifest and extract the 64-character hex digest for `asset_name`.
pub fn parse_sha256_for_asset(manifest: &str, asset_name: &str) -> Option<String> {
    for line in manifest.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let mut parts = trimmed.split_whitespace();
        let hash = parts.next()?;
        let file = parts.next()?.trim_start_matches('*');
        if file.eq_ignore_ascii_case(asset_name) && hash.len() == 64 && hash.chars().all(|c| c.is_ascii_hexdigit()) {
            return Some(hash.to_lowercase());
        }
    }
    None
}

/// Compare two version strings (`v4.0.0`, `4.1.0-beta.1`) by `(major, minor, patch, is_stable)`.
pub fn compare_versions(a: &str, b: &str) -> std::cmp::Ordering {
    let parse = |s: &str| -> (u64, u64, u64, bool) {
        let clean = s.trim().trim_start_matches('v');
        let (core, pre) = clean.split_once('-').unwrap_or((clean, ""));
        let mut nums = core.split('.').map(|p| p.parse::<u64>().unwrap_or(0));
        let major = nums.next().unwrap_or(0);
        let minor = nums.next().unwrap_or(0);
        let patch = nums.next().unwrap_or(0);
        let is_stable = pre.is_empty();
        (major, minor, patch, is_stable)
    };
    parse(a).cmp(&parse(b))
}

fn dir_size_bytes(path: &Path) -> u64 {
    let Ok(entries) = std::fs::read_dir(path) else {
        return 0;
    };
    let mut total: u64 = 0;
    for entry in entries.flatten() {
        if let Ok(meta) = entry.metadata() {
            if meta.is_file() {
                total = total.saturating_add(meta.len());
            }
        }
    }
    total
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};

    #[test]
    fn test_compare_versions() {
        assert_eq!(
            compare_versions("v4.0.1", "v4.0.0"),
            std::cmp::Ordering::Greater
        );
        assert_eq!(
            compare_versions("4.0.0", "4.0.0-beta.1"),
            std::cmp::Ordering::Greater
        );
        assert_eq!(
            compare_versions("v4.0.0", "4.0.0"),
            std::cmp::Ordering::Equal
        );
    }

    #[test]
    fn test_parse_sha256_for_asset() {
        let manifest = "\
e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855  *silas-blue-windows-x86_64.exe
0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef  silas-blue-linux-x86_64
";
        let digest = parse_sha256_for_asset(manifest, "silas-blue-windows-x86_64.exe").unwrap();
        assert_eq!(
            digest,
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn test_ed25519_roundtrip_with_rfc8032_test_vector_key() {
        // RFC 8032 test vector 1 secret key whose public key matches RELEASE_SIGNING_PUBLIC_KEY
        let secret_bytes: [u8; 32] = [
            0x9d, 0x61, 0xb1, 0x9d, 0xef, 0xfd, 0x5a, 0x60,
            0xba, 0x84, 0x4a, 0xf4, 0x92, 0xec, 0x2c, 0xc4,
            0x44, 0x49, 0xc5, 0x69, 0x7b, 0x32, 0x69, 0x19,
            0x70, 0x3b, 0xac, 0x03, 0x1c, 0xae, 0x7f, 0x60,
        ];
        let signing_key = SigningKey::from_bytes(&secret_bytes);
        assert_eq!(
            signing_key.verifying_key().to_bytes(),
            RELEASE_SIGNING_PUBLIC_KEY
        );

        let manifest = b"e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855  silas-blue-linux-x86_64\n";
        let sig = signing_key.sign(manifest);
        let sig_hex = hex::encode(sig.to_bytes());

        assert!(BotUpdater::verify_manifest_signature(manifest, sig_hex.as_bytes()).is_ok());
        assert!(BotUpdater::verify_manifest_signature(b"tampered", sig_hex.as_bytes()).is_err());
    }
}
