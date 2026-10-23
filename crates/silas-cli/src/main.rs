//! Main executable (`silas-blue`) entrypoint orchestrating pre-SQLite boot health verification,
//! unified SQLite storage, AI & search engines, Discord gateway bot, and embedded Web Console.

use clap::Parser;
use silas_ai::detector::verify_active_provider_on_startup;
use silas_ai::ProviderManager;
use silas_core::config::AppSettingsPatch;
use silas_core::events::{TelemetryBus, TelemetryEvent};
use silas_core::memory::ConversationMemory;
use silas_core::quota::QuotaReservationManager;
use silas_discord::BotController;
use silas_search::SearchCoordinator;
use silas_storage::Storage;
use silas_updater::{BootState, BotUpdater, CURRENT_VERSION};
use silas_web::{serve_web_console, AuthManager, WebState};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::sync::Arc;

#[derive(Debug, Parser)]
#[command(
    name = "silas-blue",
    version = CURRENT_VERSION,
    about = "Silas Blue v4.0 — High-Performance Rust Discord AI Assistant & Web Console"
)]
struct CliArgs {
    /// Run in headless server mode without opening a desktop browser window
    #[arg(long, env = "SILAS_HEADLESS", default_value_t = false)]
    headless: bool,

    /// Do not automatically open the default browser on startup
    #[arg(long, default_value_t = false)]
    no_browser: bool,

    /// Disable the embedded Web Administration Console
    #[arg(long, env = "SILAS_NO_WEB", default_value_t = false)]
    no_web: bool,

    /// Bind IP address for the embedded Web Console
    #[arg(long, env = "SILAS_WEB_HOST", default_value = "127.0.0.1")]
    web_host: String,

    /// Bind TCP port for the embedded Web Console
    #[arg(long, env = "SILAS_WEB_PORT", default_value_t = 8080)]
    web_port: u16,

    /// Path to the unified SQLite database
    #[arg(long, env = "SILAS_DB_PATH", default_value = "data/silas.db")]
    db_path: PathBuf,

    /// Directory for pre-update snapshots and redacted database exports
    #[arg(long, env = "SILAS_BACKUPS_DIR", default_value = "backups")]
    backups_dir: PathBuf,

    /// Permit One-Time Login (OTP) magic links to be used from non-loopback LAN IPs
    #[arg(long, env = "SILAS_ALLOW_LAN_OTP", default_value_t = false)]
    allow_lan_otp: bool,

    /// Enable verbose debug logging
    #[arg(long, env = "SILAS_DEBUG", default_value_t = false)]
    debug: bool,
}

#[tokio::main]
async fn main() {
    if let Err(e) = run().await {
        eprintln!("Fatal startup error: {e}");
        std::process::exit(1);
    }
}

async fn run() -> Result<(), Box<dyn std::error::Error>> {
    let args = CliArgs::parse();

    let filter = if args.debug {
        "silas_cli=debug,silas_core=debug,silas_storage=debug,silas_ai=debug,silas_search=debug,silas_discord=debug,silas_web=debug,silas_updater=debug,info"
    } else {
        "silas_cli=info,silas_core=info,silas_storage=info,silas_ai=info,silas_search=info,silas_discord=info,silas_web=info,silas_updater=info,warn"
    };
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(filter)),
        )
        .try_init();

    // 1. Line-1 pre-SQLite boot health & automatic crash-loop rollback check
    let boot_state_path = args
        .db_path
        .parent()
        .unwrap_or_else(|| std::path::Path::new("data"))
        .join("boot_state.json");

    if let Ok(Some(rollback_notice)) =
        BootState::check_or_rollback_pre_sqlite(&boot_state_path, &args.db_path)
    {
        tracing::warn!("{rollback_notice}");
    }

    // 2. Open unified SQLite database in WAL mode
    let storage = Storage::open(&args.db_path)?;

    // 3. Apply optional environment variable bootstrap overrides
    apply_env_overrides(&storage).await?;

    // 4. Mark boot state healthy after SQLite schema initialization succeeds
    let _ = BootState::mark_boot_healthy(&boot_state_path);

    // 5. Initialize core services
    let settings = storage.get_app_settings();
    let telemetry = TelemetryBus::new(256);
    let ai = Arc::new(ProviderManager::new(&settings));
    verify_active_provider_on_startup(&ai, &settings).await;

    let search = Arc::new(SearchCoordinator::new());
    let memory = Arc::new(ConversationMemory::default());
    let quota_reservations = Arc::new(QuotaReservationManager::new());
    let updater = Arc::new(BotUpdater::new(
        storage.clone(),
        telemetry.clone(),
        &args.backups_dir,
        &boot_state_path,
    )?);

    Arc::clone(&updater).spawn_auto_update_loop();

    // Spawn 24h ledger pruning task (35-day retention)
    let storage_for_prune = storage.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(24 * 3600));
        loop {
            interval.tick().await;
            let _ = storage_for_prune.prune_old_token_records();
        }
    });

    let bot = BotController::new(
        storage.clone(),
        Arc::clone(&ai),
        Arc::clone(&search),
        Arc::clone(&memory),
        Arc::clone(&quota_reservations),
        Arc::clone(&updater),
        telemetry.clone(),
    );

    // 6. Start Discord Gateway client if a bot token is configured
    if !settings.discord_token.trim().is_empty() {
        if let Err(e) = bot.start().await {
            tracing::warn!("Discord bot did not start automatically: {e}");
        }
    } else {
        tracing::info!(
            "No Discord bot token configured yet. Open the Web Console to configure your token."
        );
    }

    // 7. Start embedded Web Console unless --no-web was passed
    if !args.no_web {
        let host_ip: IpAddr = args
            .web_host
            .parse()
            .unwrap_or(IpAddr::V4(Ipv4Addr::LOCALHOST));
        let bind_addr = SocketAddr::new(host_ip, args.web_port);

        let auth = Arc::new(AuthManager::new(args.allow_lan_otp)?);
        let otp_token = auth.generate_otp_token();

        let display_host = if host_ip.is_unspecified() {
            "127.0.0.1".to_string()
        } else {
            host_ip.to_string()
        };
        let base_url = format!("http://{display_host}:{}", args.web_port);
        let otp_url = format!("{base_url}/auth/otp?token={otp_token}");

        println!("========================================================================");
        println!("  Silas Blue v{CURRENT_VERSION} — Rust Administration Console");
        println!("  Web Console URL : {base_url}");
        println!("  One-Time Login  : {otp_url}");
        println!("  (Valid for 2 hours, single-use)");
        println!("========================================================================");

        let web_state = Arc::new(WebState {
            storage: storage.clone(),
            bot: bot.clone(),
            ai: Arc::clone(&ai),
            search: Arc::clone(&search),
            updater: Arc::clone(&updater),
            telemetry: telemetry.clone(),
            auth,
        });

        tokio::spawn(async move {
            if let Err(e) = serve_web_console(bind_addr, web_state).await {
                tracing::error!("Web Console server error on {bind_addr}: {e}");
            }
        });

        let in_container = std::path::Path::new("/.dockerenv").exists();
        if !args.headless && !args.no_browser && !in_container {
            let url_to_open = otp_url.clone();
            tokio::spawn(async move {
                tokio::time::sleep(std::time::Duration::from_millis(350)).await;
                let _ = open::that(url_to_open);
            });
        }
    }

    telemetry.emit(TelemetryEvent::System {
        message: format!("Silas Blue v{CURRENT_VERSION} startup complete."),
    });

    // 8. Wait for termination signal (Ctrl+C / SIGTERM) and execute clean shutdown
    wait_for_shutdown_signal().await;

    tracing::info!("Shutting down Silas Blue gracefully...");
    let _ = bot.stop().await;
    let _ = storage.checkpoint_wal();
    tracing::info!("Shutdown complete.");

    Ok(())
}

async fn apply_env_overrides(storage: &Storage) -> Result<(), silas_storage::StorageError> {
    let current = storage.get_app_settings();
    let mut patch = AppSettingsPatch::default();
    let mut changed = false;

    if let Ok(tok) = std::env::var("DISCORD_TOKEN") {
        let trimmed = tok.trim();
        if !trimmed.is_empty() && current.discord_token.is_empty() {
            patch.discord_token = Some(trimmed.to_string());
            changed = true;
        }
    }
    if let Ok(prov) = std::env::var("SILAS_AI_PROVIDER") {
        let trimmed = prov.trim();
        if !trimmed.is_empty() && current.backend_type != trimmed {
            patch.backend_type = Some(trimmed.to_string());
            changed = true;
        }
    }
    if let Ok(url) = std::env::var("OLLAMA_URL") {
        let trimmed = url.trim();
        if !trimmed.is_empty() && current.ollama_url != trimmed {
            patch.ollama_url = Some(trimmed.to_string());
            changed = true;
        }
    }
    if let Ok(url) = std::env::var("LLAMACPP_URL") {
        let trimmed = url.trim();
        if !trimmed.is_empty() && current.llamacpp_url != trimmed {
            patch.llamacpp_url = Some(trimmed.to_string());
            changed = true;
        }
    }

    if changed {
        storage.update_app_settings_async(patch).await?;
    }
    Ok(())
}

async fn wait_for_shutdown_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};
        let mut sigterm = match signal(SignalKind::terminate()) {
            Ok(s) => Some(s),
            Err(_) => None,
        };
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {},
            _ = async {
                if let Some(ref mut s) = sigterm {
                    s.recv().await;
                } else {
                    std::future::pending::<()>().await;
                }
            } => {},
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}
