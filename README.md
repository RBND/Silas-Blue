# About Silas Blue V2 Alpha

## What is Silas Blue?

Silas Blue is a versatile Discord bot powered by local AI models through Ollama. It allows you to bring powerful AI capabilities directly to your Discord server without relying on external API services, ensuring privacy and control over your data.

## Key Features

- **Easy To Use Gui**: Modern gui with custom themes. (RetroWave theme included)
- **Local AI Processing**: Runs AI models locally through Ollama for privacy and control
- **Multi-Model Support**: Compatible with various Ollama models (Gemma, Llama, etc.)
- **Discord Integration**: Seamless interaction within your server channels
- **Server-Specific Configuration**: Customize settings per Discord server
- **Permission Management**: Control who can use which features
- **Automatic Restart Option**: Optional scheduled restarts for stability
- **Paginated Responses**: Clean formatting for longer AI responses
- **Terminal Control Interface**: Manage your bot settings via terminal commands
- **Simple Command Structure**: Interact using `!` prefix or by tagging the bot


## Quick Links ##  
* [Requirements](https://github.com/RBND/Silas-Blue/edit/V2-Alpha/README.md#requirements)
* [Installing Requirements](https://github.com/RBND/Silas-Blue/edit/V2-Alpha/README.md#installing-requirements) <--- Start here if you're new
* [Installing into an existing venv](https://github.com/RBND/Silas-Blue/edit/V2-Alpha/README.md#-installing-into-an-existing-virtual-environment)
* [Creating a Discord Bot / Get your private token](https://github.com/RBND/Silas-Blue/edit/V2-Alpha/README.md#creating-a-discord-bot)
* [Running the bot](https://github.com/RBND/Silas-Blue/edit/V2-Alpha/README.md#running-the-bot)

## Requirements

- **Python 3**: [Download from python.org](https://www.python.org/downloads)
- **Ollama**: [Download from ollama.com](https://ollama.com/download)
- **Python Libraries**: Discord.py, PySide6, requests
- **Discord Developer Account**: You'll need to create an application in the [Discord Developer Portal](https://discord.com/developers/applications)
- **Discord Bot Token**: Generate a private token for your bot through the Developer Portal

# Detailed Setup Instructions

## Installing Requirements

1. **Download Silas Blue**:
   - Download and extract the Silas Blue V2 files to a folder on your computer

2. **Install Python 3**:
   - Visit [python.org/downloads](https://www.python.org/downloads)
   - Download the latest version for your operating system
   - During installation, **make sure to check the box** "Add Python to PATH"
   - Complete the installation wizard


3. **Installing Python Packages in a Virtual Environment**

This guide provides step-by-step instructions for setting up a virtual environment and installing the following Python packages:

- `discord.py`
- `PySide6`
- `requests`

---

If you already have a venv setup skip to: [Installing to an existing virtual environment](https://github.com/RBND/Silas-Blue/edit/V2-Alpha/README.md#-installing-into-an-existing-virtual-environment)

### 🪟 Windows

1. Create a virtual environment
```cmd
python -m venv .venv
```

2. Activate the virtual environment
```cmd
venv\Scripts\activate
```

3. Install packages
```cmd
pip install discord.py PySide6 requests
```

---

### 🐧 Linux

1. Create a virtual environment
```bash
python3 -m venv .venv
```

2. Activate the virtual environment
```bash
source venv/bin/activate
```

3. Install packages
```bash
pip install discord.py PySide6 requests
```

---

### 🍎 macOS

1. Create a virtual environment
```bash
python3 -m venv .venv
```

2. Activate the virtual environment
```bash
source venv/bin/activate
```

3. Install packages
```bash
pip install discord.py PySide6 requests
```

---

### 🔁 Installing into an Existing Virtual Environment

If you already have a virtual environment set up, simply activate it:

- **Windows**:
  ```cmd
  venv\Scripts\activate
  ```

- **Linux/macOS**:
  ```bash
  source venv/bin/activate
  ```

Then install the packages:
```bash
pip install discord.py PySide6 requests
```

---

### ✅ Verification

After installation, you can verify by running:
```bash
python -m pip show discord.py PySide6 requests
```

This will display version info for each installed package.

---
  
4. **Install Ollama**:
   - Visit [ollama.com/download](https://ollama.com/download)
   - Download and install the version for your operating system
   - Follow the installation prompts

### Creating a Discord Bot

1. **Create a Discord Account** (skip if you already have one):
   - Visit [discord.com/register](https://discord.com/register)
   - Complete the registration process

2. **Access the Discord Developer Portal**:
   - Go to [discord.com/developers/applications](https://discord.com/developers/applications)
   - Log in with your Discord account

3. **Create a New Application**:
   - Click the "New Application" button in the top-right corner
   - Enter a name for your bot (e.g., "Silas Blue")
   - Accept the terms and click "Create"

4. **Configure Bot Settings**:
   - In the left sidebar, click "Bot"
   - Click "Add Bot" and confirm with "Yes, do it!"
   - Under the username section, you'll see your bot's profile
   - Toggle on these recommended settings:
     - "PUBLIC BOT" (if you want others to invite it)
     - "MESSAGE CONTENT INTENT" (required for the bot to read messages)
     - "PRESENCE INTENT"
     - "SERVER MEMBERS INTENT"

5. **Get Your Bot Token**:
   - In the "Bot" section, click "Reset Token" and confirm
   - Copy the displayed token (this is your private bot token)
   - **IMPORTANT**: Never share this token publicly - it grants control of your bot

6. **Generate Invite Link**:
   - In the left sidebar, click "OAuth2" then "URL Generator"
   - Under "SCOPES", select "bot"
   - Under "BOT PERMISSIONS", select:
     - "Read Messages/View Channels"
     - "Send Messages"
     - "Embed Links"
     - "Attach Files"
     - "Read Message History"
     - "Add Reactions"
   - Copy the generated URL from the bottom of the page

7. **Invite Bot to Your Server**:
   - Paste the URL in your browser
   - Select your server from the dropdown
   - Click "Authorize" and complete any verification
   - Your bot will now appear in your server member list (likely offline until you run it)

### Running the bot

1. **Launch the Bot**:
   - Open a terminal in the folder containing the bot files
   - To run: `python SilasBlue.py`

3. **First-Time Setup**:
   - When prompted, paste your Discord bot token into the terminal
   - or can create a `bot_token.txt` file with your token in it inside the folder  `Config`
   - The bot will connect to Discord and display connection information
   - You'll see configuration information for any servers the bot has joined

4. **Using the Bot**:
   - You can configure the bot via the gui including downloading new models
   - or interact with the bot in Discord using `!command` or by tagging `@BotName command`
   - Type `!help` or `@BotName help` to see available commands


### Need Help?

Contact RobotsNeverDie via [Discord](https://discord.com/users/296353246920835074) (preferred) or [Reddit](https://www.reddit.com/user/Robots_Never_Die/)
