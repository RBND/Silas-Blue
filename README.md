# Silas Blue

Silas Blue is a self-hosted Discord bot for running AI models in your server. Point it at local models through **Ollama** or **llama.cpp**, or hook it up to any OpenAI-compatible API (OpenRouter, OpenAI, Groq, DeepSeek, etc.).

v4 is a single Rust binary with a built-in web UI and SQLite baked in, so there's no Python environment or external database to set up.

## Features

- Switch between local models and remote APIs globally or per server.
- Configure your bot token, pick models, edit system prompts, and watch live logs from the browser (`http://127.0.0.1:8080`).
- Optional web search for current events via Bing, DuckDuckGo, Tavily, or Brave Search.
- Restrict who can chat with the bot, run web searches, or change models by Discord role or user ID.
- Set daily, weekly, or monthly token limits per role so one person doesn't burn through your GPU or API budget.
- Auto-reply to new posts in designated Discord Forum channels with custom instructions.
- Conversation history stays in RAM and expires automatically. Users can also run `/forget_me` anytime to wipe their messages from memory.

## Getting Started

### Docker

```bash
docker compose up -d --build
docker compose logs -f silas-blue
```

Grab the one-time login link from the logs, open it in your browser, paste in your Discord bot token, and click **Start Bot**. (If you're running Docker on another machine on your LAN, expose `8080:8080` and set `SILAS_ALLOW_LAN_OTP=true` so the login link works across the network.)

> **Note:** Make sure **Message Content Intent** is enabled under **Bot -> Privileged Gateway Intents** in the [Discord Developer Portal](https://discord.com/developers/applications) so Silas can read messages when mentioned or replied to.

### Build from source

Requires [Rust](https://rustup.rs/) 1.80+.

```bash
cargo build --release --bin silas-blue
./target/release/silas-blue
```

On a desktop, Silas opens the web UI in your browser automatically. On a headless server, pass `--headless` and copy the login link printed in the terminal.

## Discord Commands

All commands work as `/slash` commands or `!prefix` commands. You can also just `@mention` the bot or reply to one of its messages.

| Command | What it does |
| :--- | :--- |
| `/ask <prompt>` | Chat with the bot |
| `/search <query>` | Search the web and answer with sources |
| `/links <query>` | Just return web links, no AI summary |
| `/clear` | Clear the current channel's conversation memory |
| `/forget_me` | Wipe your messages from memory and scrub your ID from usage stats |
| `/quota` | See how many tokens you have left |
| `/models` | List available models on the current backend |
| `/setmodel <name>` | Change the server's active model |
| `/pull <name>` | Pull a model on your local Ollama server |
| `/status` | Check which backend and model the bot is using |

Server admins can also configure permissions, forum rules, prompts, and quotas from the web UI.

## Startup Flags

Everything is configured in the web UI (saved to `data/silas.db`), but a few flags are useful when hosting:

| Flag | Env Var | What it does |
| :--- | :--- | :--- |
| `--headless` | `SILAS_HEADLESS` | Don't open a browser window on startup |
| `--web-host` | `SILAS_WEB_HOST` | Bind address for the web UI (default `127.0.0.1`, or `0.0.0.0` in Docker) |
| `--web-port` | `SILAS_WEB_PORT` | Port for the web UI (default `8080`) |
| `--allow-lan-otp` | `SILAS_ALLOW_LAN_OTP` | Allow using the terminal login link from another machine on your LAN |
| `--db-path` | `SILAS_DB_PATH` | Path to the SQLite database file (default `data/silas.db`) |

## Upgrading from v3 (Python)

The older Python version (`v3.x`) lives on the `v3-legacy` branch. To import your old server configs, bot token, and usage history into v4:

```bash
cargo run --release --bin silas-migrate -- --from /path/to/old/silas --to data/silas.db --force
```

## License

Licensed under the **RBND-NC License (Version 1.2)** for non-commercial use with attribution to **RobotsNeverDie**. See [TERMS_OF_SERVICE.md](TERMS_OF_SERVICE.md) and [PRIVACY_POLICY.md](PRIVACY_POLICY.md) for details.
