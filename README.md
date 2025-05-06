# About Silas Blue

## What is Silas Blue?

Silas Blue is a versatile Discord bot powered by local AI models through Ollama. It allows you to bring powerful AI capabilities directly to your Discord server without relying on external API services, ensuring privacy and control over your data.

## Key Features

- **Local AI Processing**: Runs AI models locally through Ollama for privacy and control
- **Multi-Model Support**: Compatible with various Ollama models (Gemma, Llama, etc.)
- **Discord Integration**: Seamless interaction within your server channels
- **Server-Specific Configuration**: Customize settings per Discord server
- **Permission Management**: Control who can use which features
- **Automatic Restart Option**: Optional scheduled restarts for stability
- **Paginated Responses**: Clean formatting for longer AI responses
- **Terminal Control Interface**: Manage your bot settings via terminal commands
- **Simple Command Structure**: Interact using `!` prefix or by tagging the bot

## Requirements

- **Python 3**: [Download from python.org](https://www.python.org/downloads)
- **Ollama**: [Download from ollama.com](https://ollama.com/download)
- **Python Libraries**: Discord.py, aiohttp, asyncio, colorama
- **Discord Developer Account**: You'll need to create an application in the [Discord Developer Portal](https://discord.com/developers/applications)
- **Discord Bot Token**: Generate a private token for your bot through the Developer Portal

## Detailed Setup Instructions

### Installing Python and Required Libraries

1. **Install Python 3**:
   - Visit [python.org/downloads](https://www.python.org/downloads)
   - Download the latest version for your operating system
   - During installation, **make sure to check the box** "Add Python to PATH"
   - Complete the installation wizard

2. **Install Required Python Libraries**:
   - Open a command prompt or terminal
   - For Windows (Run as Administrator):
     ```
     py -3 -m pip install -U discord.py aiohttp asyncio colorama
     ```
   - For macOS/Linux:
     ```
     python3 -m pip install -U discord.py aiohttp asyncio colorama
     ```
   - Wait for the installation to complete

### Setting Up Ollama and Models

1. **Install Ollama**:
   - Visit [ollama.com/download](https://ollama.com/download)
   - Download and install the version for your operating system
   - Follow the installation prompts

2. **Verify Ollama Installation**:
   - Open a terminal or command prompt
   - Type: `ollama --version`
   - You should see the version number displayed

3. **Start Ollama Service**:
   - In your terminal, run: `ollama serve`
   - This starts the Ollama service in the background

4. **Download AI Models**:
   - In a new terminal window, download your preferred models:
   - For example: `ollama pull gemma3:1b`
   - You can find more models at [ollama.com/search](https://ollama.com/search)

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

## Running Silas Blue

1. **Download Silas Blue**:
   - Download and extract the [Silas Blue files](https://github.com/RBND/Silus-Blue/releases/latest) to a folder on your computer

2. **Launch the Bot**:
   - Open a terminal in the folder containing the bot files
   - To run with auto-restart: `python starter.py`
   - To run without auto-restart: `python SilasBlue.py`

3. **First-Time Setup**:
   - When prompted, paste your Discord bot token
   - The bot will connect to Discord and display connection information
   - You'll see configuration information for any servers the bot has joined

4. **Using the Bot**:
   - Interact with the bot in Discord using `!command` or by tagging `@SilasBlue command`
   - Type `!help` or `@SilasBlue help` to see available commands
   - Use terminal commands for advanced configuration (type `Help` in the terminal)

## Terminal Commands

Silas Blue offers a powerful terminal interface for configuration:

- `help` - Display all available commands
- `servers` - List all connected servers
- `server <server_id>` - View configuration for a specific server
- `edit <server_id> <setting> <value>` - Edit server settings
- `permissions <server_id> <action> <permission_type>` - Manage permissions
- `token [new_token|show]` - Change or view the Discord token
- `restart` - Restart the bot
- `shutdown` - Shut down the bot

## Keeping Your Bot Updated

When updating to a new version of Silas Blue:
- Keep your `bot_config.pkl` and `token.txt` files
- Replace all other files with the new version

## Need Help?

Contact RobotsNeverDie via [Discord](https://discord.com/users/296353246920835074) (preferred) or [Reddit](https://www.reddit.com/user/Robots_Never_Die/)