# Silas Blue v3.0.0

[![Version](https://img.shields.io/badge/version-3.0.0-blue.svg)](https://github.com/RobotsNeverDie/Silas-Blue)
[![Python](https://img.shields.io/badge/python-3.10%2B-brightgreen.svg)](https://www.python.org/)
[![GUI](https://img.shields.io/badge/GUI-PySide6-purple.svg)](https://www.qt.io/qt-for-python)
[![Web](https://img.shields.io/badge/Web-aiohttp-ff69b4.svg)](https://docs.aiohttp.org/)
[![License](https://img.shields.io/badge/license-RBND--NC--1.2-orange.svg)](LICENSE.md)
[![Discord](https://img.shields.io/badge/Discord-discord.py-5865F2.svg)](https://discordpy.readthedocs.io/)

**Silas Blue** is a powerful, privacy-first Discord AI Assistant bot designed to run either completely locally using **Ollama** or **Llama.cpp**, or connected to high-performance **OpenAI-compatible Cloud APIs** (Xiaomi MiMo, Groq, OpenRouter, DeepSeek, xAI).

Silas Blue v3.0.0 introduces a **Discord-grade desktop control panel** alongside a **secure, remote web management dashboard** for managing your bot from anywhere—whether on your local network (LAN) or a remote Linux VPS.

---

## 🚀 What's New in Version 3.0.0

* **Modern Desktop Control Panel**: Complete visual overhaul featuring a left-hand navigation rail (**Dashboard**, **AI & Models**, **Server Config**, **Logs & Console**, and **Settings**), real-time status KPI cards, and responsive card layouts.
* **Secure Remote Web Management Console**: An embedded async web interface (`aiohttp`) that enables managing your bot from any browser without installing desktop dependencies.
* **Discord OAuth2 & Zero-Trust Web Security**:
  * **Discord OAuth2 Login**: Restricts web management access strictly to verified Bot Owner or Admin Discord accounts.
  * **LAN Magic Link (OTP)**: Cryptographically secure one-time login links printed to the console for instant setup on private local networks.
  * **Hardened Sessions**: HMAC-SHA256 signed `HttpOnly` session cookies, CSRF protection, and brute-force IP rate limiting.
* **Headless Server & Turnkey VPS Deployment**:
  * Run in pure headless daemon mode using `python SilasBlue.py --headless --web`.
  * Production-ready `Dockerfile`, `docker-compose.yml`, and `Caddyfile` for automated Let's Encrypt HTTPS.
* **Context-Sensitive AI Configuration**: Dynamic provider switching between Ollama, Llama.cpp, and Remote Cloud APIs—revealing only the fields relevant to your active backend.
* **Real-Time WebSocket Log Streaming**: Live dual-terminal event stream (Discord Activity vs System Logs) accessible simultaneously on desktop and web.
* **Upgraded Theme System**: Refined styling supporting 5 built-in palettes (**RetroWave**, **OLED Dark**, **Forest Night**, **Rose Pine**, and **Solarized Light**).

---

## 🌟 Key Features

| Feature | Description |
| :--- | :--- |
| **Flexible AI Backends** | Run locally via **Ollama** or **Llama.cpp** (llama-server), or connect to remote endpoints (Xiaomi MiMo, OpenAI, Groq, OpenRouter, DeepSeek). |
| **Dual Management UIs** | Desktop application built with PySide6 + responsive Single-Page App (SPA) web dashboard. |
| **Discord OAuth2 Login** | Access-controlled web management locked to designated Discord User IDs. |
| **Per-Server Role Permissions** | Define who can prompt the bot, switch AI models, or adjust server settings on a per-guild basis. |
| **Model Manager & Puller** | Discover installed models, switch active models on the fly, or pull new models from Ollama / HuggingFace. |
| **Smart Response Pagination** | Intelligently breaks long AI responses into readable, Discord-safe paginated embeds (up to 1900 chars). |
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
* **Help & Server Info**:
  ```text
  !help       - View available commands and usage
  !invite     - Generate an invitation link to add Silas Blue to a server
  !ping       - Check bot gateway latency
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
* **Reddit**: [u/Robots_Never_Die](https://www.reddit.com/user/Robots_Never_Die/)
* **GitHub Issues**: [github.com/RobotsNeverDie/Silas-Blue/issues](https://github.com/RobotsNeverDie/Silas-Blue/issues)