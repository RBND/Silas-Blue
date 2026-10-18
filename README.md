# Silas Blue v3.2.0

[![Version](https://img.shields.io/badge/version-3.2.0-blue.svg)](https://github.com/RobotsNeverDie/Silas-Blue)
[![Python](https://img.shields.io/badge/python-3.10%2B-brightgreen.svg)](https://www.python.org/)
[![GUI](https://img.shields.io/badge/GUI-PySide6-purple.svg)](https://www.qt.io/qt-for-python)
[![Web](https://img.shields.io/badge/Web-aiohttp-ff69b4.svg)](https://docs.aiohttp.org/)
[![License](https://img.shields.io/badge/license-RBND--NC--1.2-orange.svg)](LICENSE.md)
[![Discord](https://img.shields.io/badge/Discord-discord.py-5865F2.svg)](https://discordpy.readthedocs.io/)

**Silas Blue** is a powerful, privacy-first Discord AI Assistant bot designed to run either completely locally using **Ollama** or **Llama.cpp**, or connected to high-performance **OpenAI-compatible Cloud APIs** (Xiaomi MiMo, Groq, OpenRouter, DeepSeek, xAI).

Silas Blue v3.2.0 introduces **Real-Time Web Search & Smart Auto-Search Grounding (RAG)**, zero-configuration **DuckDuckGo search**, optional **Tavily** and **Brave Search** integrations, dedicated **GUI controls & diagnostics**, and explicit search commands (`!search`, `!links`, `!autosearch`, `!setwebsearch`).

---

## 🚀 What's New in Version 3.2.0

* **Real-Time Web Search & Auto-Grounding (RAG)**:
  * **Smart Auto-Search Mode**: Automatically evaluates user prompts for search intent or temporal/real-time requirements (e.g. news, releases, scores, current facts) and fetches live web context in the background before generating a response.
  * **Zero-Config DuckDuckGo Search**: Built-in async search requiring no external API keys or accounts.
  * **API Search Integrations**: Optional high-speed search via Tavily AI Search and Brave Search APIs.
  * **Source Citations**: Model synthesizes grounded answers with clean, clickable Discord markdown links to the sources used.
  * **Search Caching**: In-memory query caching (5-minute TTL) prevents redundant search queries.
* **New Discord Search Commands**:
  * `!search <query>` / `!web <query>`: Forces a live web search and synthesizes an answer citing sources.
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
* **Help & Server Info**:
  ```text
  !help                      - View available commands and usage
  !invite                    - Generate an invitation link to add Silas Blue to a server
  !ping                      - Check bot gateway latency
  !config                    - View active server configuration (Admin only)
  ```

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