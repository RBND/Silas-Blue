# Silas Blue v3.5.8

[![Version](https://img.shields.io/badge/version-3.5.8-blue.svg)](https://github.com/RobotsNeverDie/Silas-Blue)
[![Python](https://img.shields.io/badge/python-3.10%2B-brightgreen.svg)](https://www.python.org/)
[![GUI](https://img.shields.io/badge/GUI-PySide6-purple.svg)](https://www.qt.io/qt-for-python)
[![Web](https://img.shields.io/badge/Web-aiohttp-ff69b4.svg)](https://docs.aiohttp.org/)
[![License](https://img.shields.io/badge/license-RBND--NC--1.2-orange.svg)](LICENSE.md)
[![Discord](https://img.shields.io/badge/Discord-discord.py-5865F2.svg)](https://discordpy.readthedocs.io/)

**Silas Blue** is a powerful, privacy-first Discord AI Assistant bot designed to run either completely locally using **Ollama** or **Llama.cpp**, or connected to high-performance **OpenAI-compatible Cloud APIs** (Xiaomi MiMo, Groq, OpenRouter, DeepSeek, xAI).

---

## 🚀 What's New in Version 3.5.8

* **Brand Favicon & Web Management Assets**:
  * **Cyberpunk Mascot Vector Favicon**: Introduced scalable vector SVG (`favicon.svg`) modeled after the Silas Blue neon robot mascot with dual magenta antenna orbs, cyan visor, glowing eyes, and dark high-contrast squircle plate.
  * **Cross-Platform Multi-Resolution Formats**: Packaged multi-resolution `favicon.ico` (16x16, 32x32, 48x48, 64x64, 128x128, 256x256), high-DPI PNGs (`16x16`, `32x32`, `48x48`), Apple touch icon (`180x180`), and Android/Chrome icons (`192x192`, `512x512`).
  * **PWA Web Manifest & Dedicated Route**: Added `/favicon.ico` root server route and standard `site.webmanifest` for mobile and desktop browser homescreens.
  * **Branded Web Interface**: Integrated mascot iconography across both the login portal and main sidebar navigation header.

---

## 🚀 Prior Features (v3.5.7)

* **Automated Dependency Monitoring with GitHub Dependabot**:
  * **Dependabot Version Updates**: Added `.github/dependabot.yml` configured to monitor Python dependencies (`requirements.txt`) and Docker base images (`Dockerfile`) weekly.
  * **Automated Pull Requests & Notifications**: GitHub will automatically open PRs and trigger notifications whenever updates or security advisories become available for project dependencies.

---

## 🚀 Prior Features (v3.5.6)

* **Prominent Forum Reply Pagination Configuration**:
  * **Explicit Pagination Naming**: Labeled and styled clearly as **Forum Reply Pagination Mode** across all settings interfaces so technicians and server owners can easily find and configure pagination for forum replies.
  * **Dual-Tab Accessibility**: The setting is now directly accessible and synchronized across both **Prompt & Pagination** and **Forum Auto Reply** sub-pages.
  * **Supported Delivery Modes**:
    * `all_chunks`: **Send All Chunks Sequentially ([Part 1/N])** - Permanent thread history without expiring buttons (Default & Recommended).
    * `interactive`: **Interactive Pagination Buttons (Persistent)** - Persistent `[Previous]` / `[Next]` buttons without timeout, open to any thread participant.
    * `disabled`: **Single Message Only (Truncate at 2,000 chars)** - Disables multi-page pagination in forum posts.

---

## 🚀 Prior Features (v3.5.5)

* **Configurable Forum Multi-Page Delivery**:
  * **Sequential Multi-Message Mode (Default)**: Long responses exceeding the character limit are sent sequentially as separate messages with `**[Part X/Y]**` headers, ensuring the full diagnostic solution remains permanently readable in the forum thread without button expiration.
  * **Persistent Interactive Buttons Mode**: Alternative mode where pagination buttons do not time out and can be clicked by any technician or thread participant, rather than being restricted to the thread creator.
  * **Configurable in Web & Desktop**: Toggle between delivery modes directly within the **Forum Auto-Reply** settings tab in the Web Management Console and PySide6 Desktop GUI.

---

## 🚀 Prior Features (v3.5.4)

* **Fixed Forum Post Auto-Reply Event Race Condition**:
  * Fixed an issue where the Discord `MESSAGE_CREATE` event for a new forum post's starter message was prematurely registering the thread ID as handled, preventing `on_thread_create` from executing and suppressing automated answers.
  * Unified forum post processing across both `on_thread_create` and starter `on_message` listeners, guaranteeing robust and instant replies regardless of Discord event delivery order.
  * Synchronized string and integer server configuration lookup keys across in-memory runtime caches.

---

## 🚀 Prior Features (v3.5.3)

* **Searchable Forum Channel Selector (Permissions UX)**:
  * Upgraded Forum Channel whitelist selection from a comma-separated ID text field to a Discord-grade chip and search interface matching the permissions manager:
    * 💬 **Channel Badges & Chips**: Monitored channels display as discrete chips with individual `✕` removal buttons.
    * 🔍 **Autocomplete & Smart Search**: Type channel names (`#forum`), `#channel (ID)`, raw Snowflake IDs, or channel mentions (`<#channel_id>`) with autocomplete populated from live server forum channels.
    * 🌐 **Server Default Empty State**: Clearly indicates when all forum channels are monitored by default vs restricted to specific channels.
    * 💻 **Unified Web & Desktop Support**: Available in both the Web Management Console and PySide6 Desktop GUI with full state persistence.

---

## 🚀 Prior Features (v3.5.2 & v3.5.1)

* **Web Management Console Modular Sub-Navigation**:
  * Added dedicated left rail sub-tabs under **Server Config**:
    * 🔑 **Permissions**: Role-based and user Snowflake access control across chat, models, administration, and web search.
    * 🪙 **Token Settings**: Role-based token quotas, automated reset schedules, and overage buffers.
    * 📝 **Prompt & Pagination**: Server persona override, response pagination, and inactivity memory TTL.
    * 💬 **Forum Auto-Reply**: Discord forum monitoring, channel whitelists, and tag filters.
  * **Persistent Server Selection**: Active server choice remains locked across all sub-tab switches in both Web and Desktop interfaces, ensuring seamless saving without state loss.
  * **Auto-Expanding System Prompt Layout**: The system prompt text editor dynamically expands to consume all available vertical space, neatly pushing secondary controls to the bottom of the viewport without adding a vertical scrollbar for the page.
  * **Asset Cache-Busting**: Added versioned asset query parameters to prevent browser caching stale UI assets on bot updates.

---

## 🚀 Prior Features (v3.5.0)

* **Discord Forum Post Monitoring & Auto-Replies**:
  * Proactively monitors newly created forum threads in Discord `ForumChannel`s and generates AI responses without requiring user @mentions.
  * Context-aware prompt injection combining the thread title (`[Forum Post Title: ...]`) with the starter post body.
  * Channel whitelist filtering: Restrict forum auto-replies to specific forum channels (e.g. `#support`, `#questions`), or allow all forums in the server.
  * Tag whitelist & blacklist filtering: Trigger replies only on posts matching designated tags (e.g. `help`, `unsolved`) and automatically skip posts carrying ignored tags (e.g. `closed`, `resolved`, `announcement`).
  * Specialized forum system prompts: Configure dedicated AI personas/instructions specifically for forum post answers.
  * In-Discord admin commands: `!setforum on/off`, `!setforum status`, `!setforum prompt <text>`, `!setforum title on/off`.
  * Dedicated forum auto-reply configuration card in Desktop GUI and Web Management Dashboard.

---

## 🚀 Prior Features (v3.4.0)

* **Role-Based Token Limits & Quotas**:
  * Set token quotas per Discord role with automatic Administrator/Server Owner bypass.
  * Hierarchical resolution: Members with multiple roles receive the highest configured allowance.
  * Configurable automated reset schedules: **Daily**, **Weekly** (day of week), **Monthly** (day of month), or **Rolling 24 Hours**.
  * Flexible reset time anchoring (`HH:MM`) with timezone offset selection (`UTC-12` to `UTC+14`).
  * Granular partial overage controls: Enable or disable overage on a user's final turn, with configurable token buffer caps (`max_overage_tokens`) and generation clamping.
  * In-Discord `!quota` (aliases: `!tokens`, `!myquota`) embed command displaying current role tier, cycle, tokens used, remaining allowance, and a dynamic relative reset countdown (`<t:{epoch}:R>`).
  * Web management dashboard integration for managing quotas per server.

---

## 🚀 Prior Features (v3.3.5)

* **Automated & Manual In-App/Web Updater**:
  * Dual-track update channels: **Stable** (tracked via GitHub releases) and **Latest** (tracking HEAD commits on git branch).
  * Background update scheduler with configurable check intervals and automated Discord notifications for server administrators.
  * Live multi-step update modal displaying download progress, git pull status, backup creation, and service restart.
  * Resilient Docker container restart handling (`SIGTERM` signaling) with client-side polling until the service returns online.
* **Modern Web & Console Improvements**:
  * Non-blocking, high-contrast toast notifications replacing browser `alert()` popups on configuration saves.
  * Real-time version synchronization across the web management console sidebar, KPIs, and login portal.
  * Desktop Webview mode integration for running the lightweight web interface inside native desktop frames.

---

## 🚀 Prior Features (v3.3.4)

* **AI Configuration Persistence & Auto-Detection Guard**:
  * Fixed an issue where startup background service checking would detect a running local Llama.cpp instance and overwrite user-configured Remote APIs (Xiaomi MiMo, OpenAI, Groq, etc.) with Llama.cpp.
  * Configured remote cloud APIs and explicitly configured local providers are now strictly preserved across restarts.
  * Added dedicated **"💾 Save & Apply"** buttons directly inside each AI provider card (Ollama, Llama.cpp, and Remote) for quick 1-click persistence.
  * Added auto-persistence in the desktop GUI when switching provider radio buttons, selecting presets, fetching models, and selecting remote models.

---

## 🚀 What's New in Version 3.3.3

* **Discord Prompt Handling & Refusal Guard Fix**:
  * Fixed a `NameError: name 're' is not defined` in `bot_core.py` that occurred when inspecting AI responses for refusal patterns prior to attaching web citations.
  * Added automated regression test in `tests/test_web_search.py` verifying clean execution.

---

## 🚀 What's New in Version 3.3.2

* **Interactive Remote Model Dropdown (`QComboBox`)**:
  * Upgraded the Remote Provider Default Model field from a static single-line input into an editable `QComboBox` dropdown.
  * Fetching models via "📡 Fetch Models" now loads all retrieved models directly into the dropdown and automatically unfolds the list for instant one-click selection.
  * Selecting a preset (Xiaomi MiMo, Groq, OpenAI, etc.) auto-populates the dropdown with the provider's known models while still allowing custom model name typing.
  * Real-time synchronization between the AI Settings dropdown and Dashboard Active Model selector.
* **Web Dashboard Datalist Integration**:
  * Integrated an HTML5 `<datalist>` into the Web Dashboard for automatic model suggestions and preset synchronization.

---

## 🚀 What's New in Version 3.3.1

* **Precision Search Intent Detection**:
  * Eliminated false-positive search triggers on conversational connectives (`"now that..."`, `"from now on"`, `"now I see"`, `"today was great"`) and host command attempts (`"shutdown your computer"`).
  * Temporal search indicators now strictly require informational inquiry pairing (e.g. news, weather, release dates, market data, sports scores).
* **Quoted Context Auto-Search Guard**:
  * Resolved an issue where replying to a prior message containing temporal words inadvertently triggered auto-search; intent detection now evaluates only the user's active message prompt.
* **Contextual Memory & Citation Retention**:
  * Increased default assistant history buffer from 400 to 1,500 characters, preventing premature conversational amnesia.
  * Added smart compression that preserves `**Sources:**` footers in memory, ensuring the bot retains full awareness of cited links in follow-up turns.
* **Refusal & Command Defense-in-Depth**:
  * Prevents attaching search citation footers to system command refusals or inability notices.

---

## 🚀 What's New in Version 3.3.0

* **Access Control Scalability for Large Discord Servers (10,000+ Members)**:
  * Replaced unfeasible full-member listboxes with an on-demand **Active Grants Chip Cloud** + **Smart Search & Add** system.
  * Instant autocomplete for server roles (`guild.roles`), with zero memory bloat or gateway rate-limiting risks on servers with tens of thousands of members.
  * Direct 18-digit Discord Snowflake ID and `@mention` resolution (`<@!id>`, `<@&id>`) to grant access to specific individual users.
* **Resolved 1-Line Layout Crushing Bug & High-DPI Polish**:
  * Re-architected the Server Configuration page with a **Sticky Server Header**, root **`QScrollArea`**, and a **Sticky Action Footer**.
  * Enabled `HighDpiScaleFactorRoundingPolicy.PassThrough` to prevent widget squishing and text clipping on Windows 125% and 150% display scaling.
* **Four-Domain Granular Permissions**:
  * **💬 Bot Chat & Replies** (`can_reply`): Who can prompt Silas Blue in server channels and threads.
  * **⚡ AI Model Switching** (`can_change_model`): Who can use `!model` to switch active LLMs.
  * **🛡️ Server Bot Administration** (`can_change_permissions`): Who can configure prompts, memory, and permissions via commands.
  * **🌐 Web Search & Grounding** (`can_search`): Who can use `!search` or trigger live grounded web queries.
* **Security & Concurrency Remediations**:
  * **Eliminated Global Takeover Vector**: Locked `!setglobalname` strictly to the Bot Application Owner (`bot.is_owner()`).
  * **Safety Guardrail Persistence**: System prompt overrides now concatenate cleanly with foundational anti-malware and jailbreak safety guardrails.
  * **Strict Schema Sanitization**: Added `validate_and_sanitize_guild_config()` with Snowflake regex filtering and parameter boundary clamps.
  * **Normalized Snowflake Matching**: String-normalized comparisons across all permission checks to eliminate 64-bit integer type-mismatch bypasses.
* **Performance & Accessibility Optimizations (WCAG 2.1 AA)**:
  * In-memory caching for `config.DEBUG`, completely eliminating repetitive disk reads of `app_config.json`.
  * Removed timer CSS repolish thrashing on status pills.
  * Dynamic luminance contrast calculation for status pills across dark and light themes (contrast ratio $\ge 4.5:1$).
  * Added universal keyboard focus rings and unsaved changes confirmation modals on server switches.

---

## 🚀 Prior Features (v3.2.1)

* **Bing Zero-Config Search Provider & Resilient Fallback**:
  * Added Bing Search as a first-class, free, zero-configuration provider that extracts clean titles, snippets, and decodes target redirect URLs without triggering anti-bot blocks.
  * Resilient cross-provider fallback: if any search provider fails or returns zero results, Silas Blue automatically fails over to Bing (or DuckDuckGo).
* **Context-Aware Search Query Formulation**:
  * When a user replies to an ongoing conversation thread (e.g. asking *"Give me a part number to search"*), Silas Blue automatically inspects the referenced message and conversation history to formulate a grounded, high-signal query (e.g. *"Mellanox ConnectX-3 10Gbps SFP PCIe 3.0 x8 part number"*).
* **Broadened Auto-Search Intent Detection**:
  * Detects hardware parts, model numbers, SKUs, datasheets, pinouts, benchmarks, specs, and conversational lookup queries.
* **Refined Discord System Prompt & Safety Guardrails**:
  * Optimized strictly for Discord's exact 1,000-character line-based pagination, clean paragraph breaks, and anti-abuse / prompt injection guardrails.
  * `!links <query>` / `!rawsearch <query>`: Fetches raw web links and snippets directly without calling the AI model.
  * `!autosearch [on|off]`: Server admin command to toggle auto-search for live questions.
  * `!setwebsearch [on|off]`: Server admin command to enable or disable web search features server-wide.
* **Control Panel & Web Dashboard Controls**:
  * Dedicated **Web Search & Auto-Grounding** configuration card in the PySide6 desktop GUI.
  * Live **Test Web Search** tool with real-time status and latency reporting.
  * Server-level toggles in the GUI Server Config page.
  * Real-time search logging streamed to both desktop and web consoles.

---

## 🚀 Prior Features (v3.1.0)

* **Stateful Multi-Turn Conversation Memory**:
  * **Per-Channel Server Context**: Discussions within a server channel remain contextual across multiple conversation turns.
  * **Strict DM Privacy Isolation**: DMs are strictly isolated per user ID (`dm_user_<author_id>`), ensuring zero cross-user context leakage or privacy spillover.
  * **Configurable Sliding Window**: Customize how many turns are remembered (default 6 turns, up to 50, or 0 to disable).
  * **Configurable Inactivity Expiration (TTL)**: Automatically clears inactive conversation memory after a configurable period of silence (default 30 minutes, 1–1440 min).
  * **Smart Token Compression**: Older historical assistant responses are compressed to conserve model context tokens without losing user intent.
* **Native Discord Message Reply Context**:
  * Replying directly to any previous message in Discord passes the referenced message and author as context into the AI model.
  * Replying directly to Silas Blue automatically continues the conversation seamlessly.
* **Explicit Message Recall Command**:
  * `!recall <message_id_or_link> [optional instructions]`: Fetches any prior message in the channel and prompts the AI to analyze, summarize, or respond to it.
* **Memory Management Commands & UI Controls**:
  * `!clear` / `!reset` / `!forget`: Instantly wipes channel or DM memory.
  * `!setmemory <0-50>`: Configures conversation turns for the server.
  * `!setttl <1-1440>`: Configures inactivity expiration in minutes for the server.
  * Full GUI and Web Dashboard sliders and inputs for both server memory and global DM memory.

---

## 🌟 Key Features

| Feature | Description |
| :--- | :--- |
| **Flexible AI Backends** | Run locally via **Ollama** or **Llama.cpp** (llama-server), or connect to remote endpoints (Xiaomi MiMo, OpenAI, Groq, OpenRouter, DeepSeek). |
| **Dual Management UIs** | Desktop application built with PySide6 + responsive Single-Page App (SPA) web dashboard. |
| **Discord OAuth2 Login** | Access-controlled web management locked to designated Discord User IDs. |
| **Per-Server Role Permissions** | Define who can prompt the bot, switch AI models, or adjust server settings on a per-guild basis. |
| **Custom Server Personas** | Set dedicated AI system prompts / personas per Discord server to tailor bot behavior to specific server communities. |
| **Model Manager & Puller** | Discover installed models, switch active models on the fly, or pull new models from Ollama / HuggingFace. |
| **Smart Response Pagination** | Intelligently breaks long AI responses into readable, Discord-safe paginated embeds (up to 1900 chars). |
| **Web Search & Auto-Grounding** | Real-time RAG web search via DuckDuckGo (free, zero-config), Tavily, or Brave with intelligent auto-search intent detection. |
| **Live WebSocket Logging** | Real-time monitoring of user prompts, AI completions, and gateway status without page refreshing. |
| **Crash Protection & Auto-Restart** | Automatic recovery counters and optional periodic daemon auto-restarts for continuous uptime. |

---

## 📋 System Requirements

* **Python**: 3.10, 3.11, 3.12, or 3.13
* **AI Provider** (choose one or more):
  * [Ollama](https://ollama.com/download) (Recommended for local inference)
  * [Llama.cpp](https://github.com/ggerganov/llama.cpp) (`llama-server.exe` / `llama-server`)
  * An API key for an OpenAI-compatible remote provider (Xiaomi MiMo, Groq, OpenRouter, etc.)
* **Discord Bot Account**: Created via the [Discord Developer Portal](https://discord.com/developers/applications).

---

## 🛠️ Installation & Setup

### 1. Local Desktop Installation (Windows, macOS, Linux)

#### Windows Quick Start
1. Clone or download this repository:
   ```cmd
   git clone https://github.com/RobotsNeverDie/Silas-Blue.git
   cd Silas-Blue
   ```
2. Double-click **`start.bat`** (or run `start.bat` in Command Prompt / PowerShell). 
   * `start.bat` automatically detects your virtual environment (`.venv`) and launches Silas Blue.

#### Linux & macOS Quick Start
1. Clone or download this repository:
   ```bash
   git clone https://github.com/RobotsNeverDie/Silas-Blue.git
   cd Silas-Blue
   ```
2. Make `start.sh` executable and run it:
   ```bash
   chmod +x start.sh
   ./start.sh
   ```
   * `start.sh` automatically detects `.venv` or system Python 3 and launches Silas Blue.

#### Manual Installation (All Platforms)
1. **Clone the repository**:
   ```bash
   git clone https://github.com/RobotsNeverDie/Silas-Blue.git
   cd Silas-Blue
   ```

2. **Create and activate a virtual environment**:
   * **Windows (PowerShell)**:
     ```powershell
     python -m venv .venv
     .venv\Scripts\Activate.ps1
     ```
   * **Linux / macOS**:
     ```bash
     python3 -m venv .venv
     source .venv/bin/activate
     ```

3. **Install dependencies**:
   ```bash
   pip install --upgrade pip
   pip install -r requirements.txt
   ```

4. **Launch the application**:
   ```bash
   python SilasBlue.py
   ```

---

### 2. Headless Server & VPS Deployment (No GUI / Display Required)

Silas Blue v3.0.0 can run as a background service on headless Linux servers or virtual private servers (VPS):

```bash
# Run headless with web management bound to all network interfaces:
python SilasBlue.py --headless --web --web-host 0.0.0.0 --web-port 8080
```

On startup, Silas Blue will print your secure **One-Time Magic Link (OTP)**:
```text
=================================================================
          SILAS BLUE WEB DASHBOARD ACTIVE
=================================================================
 Web URL:         http://0.0.0.0:8080
 One-Time Login:  http://localhost:8080/auth/otp?token=f9a8b7c6d5e4...
 (Use the One-Time Login link above to access the dashboard)
=================================================================
```
Open the login link in your browser to immediately access the web dashboard!

---

### 3. Docker & Automated HTTPS Deployment (Turnkey VPS)

For public VPS servers, run Silas Blue behind an automated **Caddy** reverse proxy that provisions and renews Let's Encrypt SSL certificates automatically.

1. **Configure your domain**:
   Copy the example Caddyfile:
   ```bash
   cp Caddyfile.example Caddyfile
   ```
   Edit `Caddyfile` and replace `bot.yourdomain.com` with your domain or VPS hostname.

2. **Start the containers**:
   ```bash
   docker compose up -d
   ```

3. **Access your dashboard**:
   Navigate to `https://bot.yourdomain.com` in your browser. All traffic is securely encrypted with automated HTTPS!

---

## 🤖 Discord Bot Setup Guide

### 1. Create Your Discord Application
1. Visit the [Discord Developer Portal](https://discord.com/developers/applications).
2. Click **New Application**, give your bot a name (e.g. `Silas Blue`), and accept the Developer Terms.

### 2. Configure Privileged Gateway Intents
1. In the left navigation, click **Bot**.
2. Scroll down to **Privileged Gateway Intents** and enable:
   * ✅ **Message Content Intent** *(Required for reading prompts and commands)*
   * ✅ **Presence Intent**
   * ✅ **Server Members Intent**
3. Under the **Token** section, click **Reset Token**, copy the token, and keep it safe.

### 3. Invite the Bot to Your Server
* **Via Desktop/Web GUI**: Open Silas Blue and click **➕ Invite Bot** in the sidebar. Select your desired permissions and click **Open in Browser**.
* **Via Discord Command**: Type `!invite` in any channel where the bot is present.

### 4. (Optional) Set Up Discord OAuth2 for Web Management
To allow logging into the Web Management Dashboard via Discord:
1. In the Developer Portal, navigate to **OAuth2**.
2. Under **Redirects**, click **Add Redirect** and enter your dashboard callback URL:
   * For local testing: `http://localhost:8080/auth/discord/callback`
   * For a VPS / domain: `https://bot.yourdomain.com/auth/discord/callback`
3. Copy your **Client ID** and **Client Secret**.
4. In Silas Blue (GUI or `config/app_config.json`), set:
   ```json
   "discord_oauth_client_id": "YOUR_CLIENT_ID",
   "discord_oauth_client_secret": "YOUR_CLIENT_SECRET",
   "bot_owner_id": "YOUR_DISCORD_USER_ID"
   ```

---

## 🎮 How to Use Silas Blue

### In Discord
Silas Blue responds to mentions or standard prefix commands:
* **Chatting**: Simply mention the bot or use `!` prefix:
  ```text
  @SilasBlue Explain quantum computing in three sentences.
  !Explain the rules of chess.
  ```
* **Switching Models**:
  ```text
  !model llama3:8b
  !models
  ```
* **Custom Server Personas (System Prompts)**:
  ```text
  !setprompt You are a helpful assistant for the Coding Club. Keep answers concise.
  !setprompt reset   (resets back to the global default persona)
  ```
* **Discord Forum Auto-Reply**:
  ```text
  !setforum status           - View current forum monitoring settings (Admin only)
  !setforum on / off         - Enable or disable proactive replies in forum channels
  !setforum prompt <text>    - Set specialized AI persona/instructions for forum replies
  !setforum resetprompt      - Reset forum persona to server default
  !setforum title on / off   - Toggle including the forum post title in prompt context
  ```
* **Conversation Memory & Message Recall**:
  ```text
  !clear                     - Clear conversation memory for the current channel or DM (aliases: !reset, !forget)
  !recall <msg_id_or_link>   - Fetch a previous message by ID or link and analyze/respond to it
  !setmemory <0-50>          - Set conversation memory turns for this server (0 to disable)
  !setttl <1-1440>           - Set memory inactivity expiration in minutes for this server
  ```
* **Real-Time Web Search & Auto-Grounding (RAG)**:
  ```text
  !search <query>            - Search the web and answer with live AI citations (aliases: !web, !ddg)
  !links <query>             - Fetch raw web search links and snippets directly (alias: !rawsearch)
  !autosearch [on|off]       - Toggle auto-search for questions requiring live/temporal info (Admin only)
  !setwebsearch [on|off]     - Enable or disable web search features for this server (Admin only)
  ```
* **System Updates**:
  ```text
  /update check [channel]    - Check GitHub for latest updates (Stable or Latest)
  /update apply [channel]    - Snapshot config, pull update, and restart (Owner/Admin)
  /update channel <stable|latest> - Set active release tracking channel
  !update [check|apply|channel]   - Prefix command equivalent
  ```
* **Help & Server Info**:
  ```text
  !help                      - View available commands and usage
  !invite                    - Generate an invitation link to add Silas Blue to a server
  !ping                      - Check bot gateway latency
  !config                    - View active server configuration (Admin only)
  ```

---

## 🔄 Automated & Manual Updates

Silas Blue features a full-featured update management system supporting both native virtual environments and Docker Compose:

* **Discord Commands**: Check and apply updates directly in Discord using `/update` (slash) or `!update`.
* **Web Management Dashboard**: Dedicated **Updates** page with version diffs, changelogs, channel switcher, and 1-click updates.
* **Desktop GUI**: Located in **Settings & Diagnostics** -> **System Updates & Release Channels**.
* **Terminal Scripts**: Standalone cross-platform scripts:
  * Linux / macOS: `./update.sh` (supports `--channel stable|latest` and `--docker`)
  * Windows: `update.bat`
* **Automated Background Updates**: Configure periodic checks (e.g. every 24 hours) with choice of **Notify-only** or **Auto-Apply & Restart**.
* **Enterprise-Grade Safety**:
  * Automatic snapshots of `config/` and `state/` saved to `backups/` before any update.
  * Strict clean Git pull (`git fetch` + `reset --hard`) to prevent merge conflicts while leaving user configs untouched.
  * Boot health verification: Silas Blue verifies stability on startup. If a fatal crash occurs on boot, it automatically rolls back to the previous stable release and restores configuration backups!

---

### Command-Line Arguments Reference

| Option | Description | Default |
| :--- | :--- | :--- |
| `--headless`, `--no-gui` | Runs without desktop GUI (auto-detected on display-less Linux). | `False` |
| `--web` | Enables the embedded aiohttp web management server. | `True` |
| `--no-web` | Disables the web management server. | `False` |
| `--web-host` | Network interface to bind (`127.0.0.1` for local, `0.0.0.0` for LAN/VPS). | `127.0.0.1` |
| `--web-port` | TCP port for the web dashboard. | `8080` |

---

## ⚙️ Configuration Reference (`config/app_config.json`)

```json
{
  "backend_type": "ollama",
  "active_model": "llama3:8b",
  "ollama_url": "http://localhost:11434",
  "llamacpp_url": "http://localhost:8080",
  "remote_preset": "Xiaomi MiMo",
  "remote_base_url": "https://api.xiaomimimo.com/v1",
  "remote_model": "mimo-v2.6-pro",
  "system_prompt": "You are Silas Blue, a helpful and knowledgeable AI Discord assistant.",
  "allow_dms": true,
  "theme": "retrowave",
  "debug": false
}
```

---

## 🧪 Testing & Verification

Run the automated test suite across all subsystems (providers, configuration, permissions, invite link generator, web authentication, and REST API):

```bash
# Run all unit tests
python -m unittest discover tests
```

---

## 📜 Legal & Policies

* **License**: [RBND-NC License Version 1.2](LICENSE.md)
* **Terms of Service**: [TERMS_OF_SERVICE.md](TERMS_OF_SERVICE.md)
* **Privacy Policy**: [PRIVACY_POLICY.md](PRIVACY_POLICY.md)

---

## 💬 Community & Support

* **Discord**: [Contact RobotsNeverDie](https://discord.com/users/296353246920835074)
* **GitHub Issues**: [github.com/RobotsNeverDie/Silas-Blue/issues](https://github.com/RobotsNeverDie/Silas-Blue/issues)