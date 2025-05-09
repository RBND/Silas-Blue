███████╗██╗██╗     ██╗   ██╗ █████╗ ███████╗    ██████╗ ██╗      ██████╗ ██╗   ██╗███████╗
██╔════╝██║██║     ██║   ██║██╔══██╗██╔════╝    ██╔══██╗██║     ██╔═══██╗██║   ██║██╔════╝
███████╗██║██║     ██║   ██║███████║█████╗      ██████╔╝██║     ██║   ██║██║   ██║███████╗
╚════██║██║██║     ██║   ██║██╔══██║██╔══╝      ██╔══██╗██║     ██║   ██║██║   ██║╚════██║
███████║██║███████╗╚██████╔╝██║  ██║███████╗    ██████╔╝███████╗╚██████╔╝╚██████╔╝███████║
╚══════╝╚═╝╚══════╝ ╚═════╝ ╚═╝  ╚═╝╚══════╝    ╚═════╝ ╚══════╝ ╚═════╝  ╚═════╝ ╚══════╝

---

# 🎸 ** Silas Blue V2 **

> _A Discord bot with a Retrowave soul, powered by local AI models via Ollama_

---

## 🌈 Table of Contents
- [✨ About Silas Blue](#-about-silas-blue)
- [🚀 Key Features](#-key-features)
- [🛠️ Requirements](#️-requirements)
- [📦 Setup Instructions](#-setup-instructions)
  - [Install Requirements](#install-requirements)
  - [Create a Discord Bot](#create-a-discord-bot)
  - [Run the Bot](#run-the-bot)
- [❓ Need Help?](#-need-help)

---

## ✨ About Silas Blue

Silas Blue is a versatile Discord bot powered by local AI models through Ollama. Bring powerful AI to your Discord server _without_ relying on external APIs—**privacy and control are yours**.

---

## 🚀 Key Features

🎨 **Modern GUI** with custom Retrowave themes  
🤖 **Local AI Processing** (Ollama)  
🔄 **Multi-Model Support** (Gemma, Llama, etc.)  
💬 **Discord Integration**  
⚙️ **Server-Specific Configuration**  
🔒 **Permission Management**  
⏰ **Automatic Restart Option**  
📄 **Paginated Responses**  
❗ **Simple Command Structure** (`!command` or `@BotName command`)

---

## 🛠️ Requirements

- **Python 3** ([python.org](https://www.python.org/downloads))
- **Ollama** ([ollama.com](https://ollama.com/download))
- **Python Libraries**: `discord.py`, `PySide6`, `requests`, `psutil`, `pynvml`
- **Discord Developer Account** ([Discord Developer Portal](https://discord.com/developers/applications))
- **Discord Bot Token**

---

## 📦 Setup Instructions

### Install Requirements

```diff
+ Recommended: Use a virtual environment for isolation!

```

#### 1. Download Silas Blue
- Download and extract the Silas Blue V2 files to a folder on your computer.

#### 2. Install Python 3
- Download from [python.org/downloads](https://www.python.org/downloads)
- **Check the box** "Add Python to PATH" during install

#### 3. Install Required Python Libraries

**Create a new virtual environment and install requirements:**

- **Windows:**
  ```sh
  py -3 -m venv .venv
  .venv\Scripts\activate
  pip install -r requirements.txt
  ```
- **macOS/Linux:**
  ```sh
  python3 -m venv .venv
  source .venv/bin/activate
  pip install -r requirements.txt
  ```

**To install into an existing virtual environment:**
- Activate your venv first:
  - Windows: `path\to\your\venv\Scripts\activate`
  - macOS/Linux: `source path/to/your/venv/bin/activate`
- Then:
  ```sh
  pip install -r requirements.txt
  ```

**Alternatively, install globally (not recommended):**
- Windows: `py -3 -m pip install -r requirements.txt`
- macOS/Linux: `python3 -m pip install -r requirements.txt`

#### 4. Install Ollama
- Download and install from [ollama.com/download](https://ollama.com/download)

---

### Create a Discord Bot

1. **Create a Discord Account** ([register](https://discord.com/register))
2. **Go to the [Discord Developer Portal](https://discord.com/developers/applications)**
3. **Create a New Application**
   - Click "New Application", name it (e.g., _Silas Blue_)
4. **Configure Bot Settings**
   - Go to "Bot" > "Add Bot"
   - Enable:
     - `PUBLIC BOT` (optional)
     - `MESSAGE CONTENT INTENT` (required)
     - `PRESENCE INTENT`
     - `SERVER MEMBERS INTENT`
5. **Get Your Bot Token**
   - Click "Reset Token" and copy it (keep it secret!)
6. **Generate Invite Link**
   - OAuth2 > URL Generator
   - SCOPES: `bot`
   - BOT PERMISSIONS: `Read Messages`, `Send Messages`, `Embed Links`, `Attach Files`, `Read Message History`, `Add Reactions`
   - Copy the generated URL
7. **Invite Bot to Your Server**
   - Paste the URL in your browser, select your server, authorize

---

### Run the Bot

```diff
+ The GUI is Retrowave-themed by default! Enjoy the vibes.
```

1. **Launch the Bot:**
   - Open a terminal in the folder containing the bot files
   - **On Windows:**
     ```sh
     pythonw SilasBlue.py
     ```
   - **On macOS/Linux:**
     ```sh
     python3 SilasBlue.py
     ```
   - _(Or use `python SilasBlue.py` if `python` points to Python 3)_

2. **First-Time Setup:**
   - When prompted, paste your Discord bot token into the terminal
   - Or create a file `Config/bot_token.txt` with your token
   - The bot will connect to Discord and display connection info

3. **Using the Bot:**
   - Configure via the GUI (download new models, change themes, etc.)
   - Or interact in Discord using `!command` or `@BotName command`
   - Type `!help` or `@BotName help` for available commands

---

## ❓ Need Help?

Contact **RobotsNeverDie** via [Discord](https://discord.com/users/296353246920835074) _(preferred)_ or [Reddit](https://www.reddit.com/user/Robots_Never_Die/)

---

> _Stay retro. Stay blue. Enjoy your AI-powered Discord experience!_
 