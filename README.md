# Silas Blue v4.0.0 — Clean-Room Rust Discord AI Assistant & Web Console

**Silas Blue v4.0.0** is a 100% ground-up, clean-room rewrite of Silas Blue in **Rust (2021 Edition)**, engineered for sub-millisecond hot-path latency, memory safety (`#![forbid(unsafe_code)]`), single-binary deployment, and strict Discord Developer Policy compliance.

> **Upgrading from v3 (Python)?**
> Legacy Python v3 (`v3.5.11`) is preserved on the `v3-legacy` branch and `v3.5.11` git tag. Use the bundled `silas-migrate` utility to convert your existing `config/*.json`, `config/bot_token.txt`, `config/web_secret.key`, and `state/usage.db` into the unified v4 SQLite `STRICT` database (`data/silas.db`) in one command.

---

## Key Architecture Highlights (v4.0.0)

- **Zero-Unsafe Multi-Crate Rust Workspace**:
  - `silas-core`: Strongly-typed configurations (`AppSettings`, `GuildConfig`), deep partial patching (`GuildConfigPatch`), `< 50ns` pre-compiled O(1) `HashSet<u64>` permission evaluators, saturating role-based token quotas with in-flight burst reservation guards, code-fence-aware UTF-8 character pagination, volatile LRU conversation memory (`MAX_ACTIVE_SESSIONS = 1,000`), and Sandwich-Defense system prompt composition with immutable safety guardrails.
  - `silas-storage`: Unified SQLite `STRICT` database (`data/silas.db`) in WAL mode with lock-free in-memory `ArcSwap<AppSettings>` and `DashMap<u64, Arc<GuildConfig>>` hot caches, serialized single-writer transactions, 35-day automatic ledger retention pruning, and double-`VACUUM` secret-redacted backup exports.
  - `silas-ai`: Async LLM provider trait (`LlmProvider`) supporting **Ollama** (`/api/chat`, `/api/tags`, streaming `/api/pull`), **Llama.cpp** (`/v1/chat/completions`, `/v1/models`, `/health`), and **OpenAI-Compatible Remote Cloud APIs** (Xiaomi MiMo, OpenAI, OpenRouter, Groq, DeepSeek, xAI, and Custom endpoints) with SSRF metadata/loopback protection, `redirect::Policy::none()`, and 30-second model list caching.
  - `silas-search`: Autonomous RAG web search coordinator with pre-compiled `RegexSet` intent classification, context-aware query enrichment, XML-sandboxed `<untrusted_web_search_results>` prompt grounding, dual-regex refusal suppression guard, and resilient provider failover (**Bing Scraper -> DuckDuckGo HTML -> Tavily API -> Brave Search API**).
  - `silas-discord`: Built on `serenity 0.12` + `poise 0.6` with least-privilege Gateway Intents (`GUILDS | GUILD_MESSAGES | DIRECT_MESSAGES | MESSAGE_CONTENT` — **never** `GUILD_PRESENCES` or `GUILD_MEMBERS`), zero-ping `CreateAllowedMentions`, race-free `DashMap<u64, ForumThreadState>` Forum post auto-reply, 60-second anti-spam spontaneous reply cooldowns, interactive button pagination (`◀ Previous` / `Next ▶`), and full hybrid `!prefix` + `/slash` command parity.
  - `silas-web`: Embedded `axum` Web Administration Console compiled directly into the binary via `rust-embed`, protected by strict CSP (`default-src 'none'; script-src 'self'`), custom CSRF header + `Origin` validation, loopback-bound single-use SHA-256 OTP magic links, Discord OAuth2 owner/team-admin verification, and authenticated WebSocket live telemetry (`/api/ws/logs`).
  - `silas-updater`: Prebuilt binary self-updater verifying **Ed25519 signatures** (`ed25519-dalek`) on `SHA256SUMS.txt.sig` + streaming SHA-256 digests, snapshotting `silas.db` before replacement (`self-replace`), and enforcing a line-1 pre-SQLite crash-loop rollback guard (`data/boot_state.json`).
  - `silas-migrate`: Standalone read-only migration CLI converting legacy v3 configurations and token ledgers into `data/silas.db`.

---

## Quick Start

### 1. Build & Run Locally (Desktop / Server)
```bash
# Build release binaries
cargo build --release --workspace --bins

# Run Silas Blue (prints a single-use One-Time Login link and opens your browser on desktop)
./target/release/silas-blue
```

### 2. Migrate Existing v3 Configuration & Token Ledger
```bash
# Preview migration without writing to disk
cargo run --release --bin silas-migrate -- --from /path/to/v3 --to data/silas.db --dry-run

# Execute atomic migration into data/silas.db
cargo run --release --bin silas-migrate -- --from /path/to/v3 --to data/silas.db --force
```

### 3. Headless Server & Docker Compose Deployment
```bash
docker compose up -d --build
# Inspect container logs to copy your single-use One-Time Login URL:
docker compose logs -f silas-blue
```

---

## Discord Commands (Hybrid `/slash` & `!prefix`)

| Command | Description | Permission Required |
| :--- | :--- | :--- |
| `/ask <prompt>` or `!ask <prompt>` (or `@Silas Blue`) | Prompt Silas Blue with channel memory & optional auto-search grounding | `ChatReply` (`reply_roles` / `reply_users`) |
| `/search <query>` or `!search <query>` | Search the live web and synthesize a cited answer | `WebSearch` (`search_roles` / `search_users`) |
| `/links <query>` or `!links <query>` | Return top verified web links directly without LLM synthesis | `WebSearch` |
| `/quota` or `!quota` | Check your current token quota allowance, usage, and reset countdown | Everyone |
| `/clear` or `!clear` | Clear volatile conversation memory for the current channel or DM | `ChatReply` |
| `/forget_me` or `!forget_me` | Self-service privacy erasure: wipe all your turns from RAM and pseudonymize ledger rows | Everyone |
| `/models` or `!models` | List available AI models on the active backend | Everyone |
| `/setmodel <model>` or `!setmodel <model>` | Set the server's default AI model override | `ChangeModel` (`change_model_roles` / Admins) |
| `/pull <model>` or `!pull <model>` | Pull a model on the local Ollama server | `ChangeModel` |
| `/status` or `!status` | Show active AI backend, model, memory stats, and server settings | Everyone |
| `/permissions` or `!permissions` | Inspect or update server role permissions | `ManageBot` |
| `/forum` or `!forum` | Inspect or toggle Forum channel auto-reply rules | `ManageBot` |
| `/version` or `!version` | Display current Silas Blue version and release channel | Everyone |
| `/update [check\|apply]` or `!update` | Check or apply Ed25519-verified binary updates | Bot Owner Only |
| `/help` or `!help` | Interactive categorized command reference | Everyone |

---

## CLI Flags & Environment Variables

| CLI Flag | Environment Variable | Default | Description |
| :--- | :--- | :--- | :--- |
| `--headless` | `SILAS_HEADLESS` | `false` | Run in headless daemon mode without opening a desktop browser |
| `--no-browser` | — | `false` | Do not auto-open the default browser on startup |
| `--no-web` | `SILAS_NO_WEB` | `false` | Disable the embedded Axum Web Administration Console |
| `--web-host` | `SILAS_WEB_HOST` | `127.0.0.1` | Bind address for the Web Console (`0.0.0.0` in Docker) |
| `--web-port` | `SILAS_WEB_PORT` | `8080` | Bind TCP port for the Web Console |
| `--db-path` | `SILAS_DB_PATH` | `data/silas.db` | Path to the unified SQLite `STRICT` database |
| `--backups-dir` | `SILAS_BACKUPS_DIR` | `backups` | Directory for pre-update snapshots and redacted DB exports |
| `--allow-lan-otp` | `SILAS_ALLOW_LAN_OTP` | `false` | Allow One-Time Login (OTP) links from non-loopback LAN IPs |
| `--debug` | `SILAS_DEBUG` | `false` | Enable verbose debug logging |

---

## License

Licensed under the **RBND-NC License (Version 1.2)**. Non-commercial use only with attribution to **RobotsNeverDie** ([https://github.com/RBND/Silas-Blue](https://github.com/RBND/Silas-Blue)). See [TERMS_OF_SERVICE.md](TERMS_OF_SERVICE.md) and [PRIVACY_POLICY.md](PRIVACY_POLICY.md) for full details.
