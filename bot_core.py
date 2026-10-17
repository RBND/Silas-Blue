"""
Discord bot core for Silas Blue.
Handles commands, permissions, per-server config, and unified AI provider integration.
Fully asynchronous and non-blocking for Discord Gateway stability.
"""

import discord
from discord.ext import commands
import logging
import os
import asyncio
import json
import getpass
import time
import threading
import queue
from typing import Optional, Dict, Any, List

from ai_providers import get_ai_provider, AIProvider
from permissions import PermissionManager
from utils import load_config, save_config, get_config_path, load_app_config

logger = logging.getLogger("silasblue")

# Thread-safe event queue for GUI communication without disk bottleneck
gui_event_queue = queue.Queue()

# Theme helper
_THEME_CACHE = None
_THEME_PATH = os.path.join("themes", "retrowave.json")

def get_discord_theme_color() -> int:
    global _THEME_CACHE
    if _THEME_CACHE is None:
        try:
            with open(_THEME_PATH, "r", encoding="utf-8") as f:
                _THEME_CACHE = json.load(f)
        except Exception:
            _THEME_CACHE = {"discord_text": "#00f0ff"}
    color_hex = _THEME_CACHE.get("discord_text") or _THEME_CACHE.get("accent2") or "#00f0ff"
    return int(color_hex.lstrip("#"), 16)

def get_discord_token() -> str:
    """Loads bot token from config/bot_token.txt or prompts user."""
    config_dir = "config"
    token_path = os.path.join(config_dir, "bot_token.txt")
    os.makedirs(config_dir, exist_ok=True)

    if os.path.exists(token_path):
        with open(token_path, "r", encoding="utf-8") as f:
            token = f.read().strip()
            if token:
                return token

    print("Discord bot token not found.")
    print("Please paste your Discord bot token (input is hidden):")
    token = getpass.getpass("Token: ").strip()
    if not token:
        print("No token entered. Exiting.")
        exit(1)
    with open(token_path, "w", encoding="utf-8") as f:
        f.write(token)
    print("Token saved to config/bot_token.txt.")
    return token

COMMAND_PREFIX = "!"
DISCORD_TOKEN = None  # Loaded lazily or on start

# Global state
permissions = PermissionManager()
server_configs: Dict[Any, dict] = {}
current_ai_provider: Optional[AIProvider] = None

def get_provider() -> AIProvider:
    global current_ai_provider
    if current_ai_provider is None:
        current_ai_provider = get_ai_provider()
    return current_ai_provider

def reload_ai_provider():
    """Reloads the active AI provider with updated app settings."""
    global current_ai_provider
    current_ai_provider = get_ai_provider()
    logger.info(f"AI Provider reloaded: {current_ai_provider.name} ({current_ai_provider.base_url})")

def update_server_config(guild_id, config: dict):
    """Updates in-memory server config immediately from GUI saves."""
    server_configs[guild_id] = config
    logger.debug(f"In-memory config updated for guild {guild_id}")

def log_to_gui(event_type: str, data: dict):
    """Pushes event to the in-memory queue and optionally appends to file."""
    entry = {
        "event": event_type,
        "data": data,
        "timestamp": time.time()
    }
    gui_event_queue.put(entry)
    try:
        log_path = os.path.join("config", "gui_log.txt")
        os.makedirs("config", exist_ok=True)
        with open(log_path, "a", encoding="utf-8") as f:
            f.write(json.dumps(entry) + "\n")
    except Exception:
        pass

def paginate_text(text: str, max_chars: int = 1850) -> List[str]:
    """
    Splits text into pages strictly under max_chars (default 1850).
    Guarantees every page length <= max_chars.
    """
    max_chars = min(max_chars, 1850)
    lines = text.splitlines(keepends=True)
    pages = []
    current = ""
    for line in lines:
        while len(line) > max_chars:
            split_at = line.rfind(" ", 0, max_chars)
            if split_at == -1 or split_at < max_chars // 2:
                split_at = max_chars
            part = line[:split_at]
            line = line[split_at:]
            if len(current) + len(part) > max_chars:
                if current:
                    pages.append(current)
                    current = ""
            current += part
        if len(current) + len(line) > max_chars:
            if current:
                pages.append(current)
                current = ""
        current += line
    if current:
        pages.append(current)
    return pages or [""]

class PaginatedView(discord.ui.View):
    def __init__(self, pages: List[str], author_id: int, timeout: int = 180):
        super().__init__(timeout=timeout)
        self.pages = pages
        self.author_id = author_id
        self.current_page = 0
        self.total_pages = len(pages)
        self.message: Optional[discord.Message] = None
        self.update_buttons()

    def update_buttons(self):
        self.clear_items()
        self.prev_button.disabled = self.current_page == 0
        self.next_button.disabled = self.current_page == self.total_pages - 1
        self.add_item(self.prev_button)
        self.add_item(self.next_button)

    @discord.ui.button(label="Previous", style=discord.ButtonStyle.primary, disabled=True)
    async def prev_button(self, interaction: discord.Interaction, button: discord.ui.Button):
        if interaction.user.id != self.author_id:
            await interaction.response.send_message("You can't control this pagination.", ephemeral=True)
            return
        if self.current_page > 0:
            self.current_page -= 1
            await self.update_message(interaction)

    @discord.ui.button(label="Next", style=discord.ButtonStyle.primary, disabled=False)
    async def next_button(self, interaction: discord.Interaction, button: discord.ui.Button):
        if interaction.user.id != self.author_id:
            await interaction.response.send_message("You can't control this pagination.", ephemeral=True)
            return
        if self.current_page < self.total_pages - 1:
            self.current_page += 1
            await self.update_message(interaction)

    async def update_message(self, interaction: discord.Interaction):
        self.update_buttons()
        content = f"Page {self.current_page + 1}/{self.total_pages}\n{self.pages[self.current_page]}"
        # Guard against Discord 2000 character hard limit
        if len(content) > 2000:
            content = content[:1997] + "..."
        await interaction.response.edit_message(content=content, view=self)

    async def on_timeout(self):
        if self.message:
            try:
                await self.message.edit(view=None)
            except Exception:
                pass

async def handle_ai_prompt(message: discord.Message, config: dict):
    """
    Sends message to active AI provider asynchronously without blocking event loop.
    Displays typing indicator while processing.
    """
    prompt = message.content.strip()
    provider = get_provider()

    # Determine default model
    default_model = config.get("default_model")
    if not default_model:
        app_cfg = load_app_config()
        if message.guild is None and app_cfg.get("dm_default_model"):
            default_model = app_cfg.get("dm_default_model")
        else:
            available = await provider.list_models()
            default_model = available[0] if available else "default"

    guild_id = message.guild.id if message.guild else None
    guild_name = message.guild.name if message.guild else "Direct Message"

    log_to_gui("prompt", {
        "guild_id": guild_id,
        "guild_name": guild_name,
        "user": str(message.author),
        "prompt": prompt
    })

    # Show Discord typing indicator during model inference
    try:
        async with message.channel.typing():
            response = await provider.send_prompt(prompt, default_model)
    except Exception as e:
        logger.error(f"Error getting AI response: {e}")
        response = f"Error processing AI request: {e}"

    log_to_gui("reply", {
        "guild_id": guild_id,
        "guild_name": guild_name,
        "user": str(message.author),
        "reply": response
    })

    max_chars = min(config.get("pagination_max_chars", 1900), 1900)
    pages = paginate_text(response, max_chars)

    if len(pages) == 1:
        text = pages[0]
        if len(text) > 2000:
            text = text[:1997] + "..."
        await message.channel.send(text)
    else:
        view = PaginatedView(pages, message.author.id)
        first_content = f"Page 1/{len(pages)}\n{pages[0]}"
        if len(first_content) > 2000:
            first_content = first_content[:1997] + "..."
        view.message = await message.channel.send(first_content, view=view)

# --- Bot Initialization ---
intents = discord.Intents.default()
intents.messages = True
intents.guilds = True
intents.message_content = True

bot = commands.Bot(command_prefix=COMMAND_PREFIX, intents=intents, help_command=None)

@bot.event
async def on_ready():
    logger.info(f"Silas Blue is online as {bot.user} (ID: {bot.user.id})")
    await bot.change_presence(activity=discord.Game(name="git.new/silasblue"))
    for guild in bot.guilds:
        config = load_config(guild.id)
        server_configs[guild.id] = config
    logger.info(f"Loaded configurations for {len(bot.guilds)} server(s).")

@bot.event
async def on_guild_join(guild):
    logger.info(f"Joined server: {guild.name} ({guild.id})")
    config = load_config(guild.id)
    server_configs[guild.id] = config

@bot.event
async def on_guild_remove(guild):
    logger.info(f"Left server: {guild.name} ({guild.id})")

@bot.event
async def on_message(message: discord.Message):
    if message.author == bot.user:
        return

    # Direct Message (DM) Policy Handling
    if message.guild is None:
        app_cfg = load_app_config()
        if not app_cfg.get("allow_dms", True):
            notice = app_cfg.get("dm_disabled_message", "")
            if notice:
                try:
                    await message.channel.send(notice)
                except Exception:
                    pass
            return
        dm_config = load_config(None)
        await handle_ai_prompt(message, dm_config)
        return

    guild_id = message.guild.id
    config = server_configs.get(guild_id)
    if not config:
        config = load_config(guild_id)
        server_configs[guild_id] = config

    # Permission check for reply
    if not permissions.can_reply(message, config):
        return

    # Random prompt probability check
    prob = config.get("random_prompt_probability", 0)
    if prob > 0:
        import random
        if random.randint(1, 100) <= prob:
            await handle_ai_prompt(message, config)
            return

    # Check for prefix or bot mention
    content = message.content
    mention_str_1 = f"<@{bot.user.id}>"
    mention_str_2 = f"<@!{bot.user.id}>"
    is_command = False

    if content.startswith(COMMAND_PREFIX):
        is_command = True
    elif content.startswith(mention_str_1) or content.startswith(mention_str_2):
        after_mention = content[len(mention_str_1):] if content.startswith(mention_str_1) else content[len(mention_str_2):]
        after_mention = after_mention.lstrip()
        first_word = after_mention.split(" ", 1)[0].lower() if after_mention else ""
        command_names = [cmd.name for cmd in bot.commands]
        if first_word in command_names:
            is_command = True
            message.content = COMMAND_PREFIX + after_mention

    if is_command:
        await bot.process_commands(message)
    else:
        if content.startswith(mention_str_1) or content.startswith(mention_str_2):
            after_mention = content[len(mention_str_1):] if content.startswith(mention_str_1) else content[len(mention_str_2):]
            after_mention = after_mention.lstrip()
            if after_mention:
                await handle_ai_prompt(message, config)
        elif bot.user in message.mentions:
            await handle_ai_prompt(message, config)

@bot.command(name="ping")
async def ping(ctx):
    await ctx.send("Pong!")

@bot.command(name="setmodel")
async def set_model(ctx, model_name: str):
    """Sets default model for this server."""
    guild_id = ctx.guild.id
    config = server_configs.get(guild_id, {})
    if not permissions.can_change_model(ctx.author, ctx.guild, config):
        await ctx.send("You do not have permission to change the model.")
        return
    config["default_model"] = model_name
    save_config(guild_id, config)
    server_configs[guild_id] = config
    log_to_gui("config_change", {
        "guild_id": guild_id,
        "user": str(ctx.author),
        "field": "default_model",
        "value": model_name
    })
    logger.info(f"Model for server {ctx.guild.name} set to {model_name} by {ctx.author}")
    await ctx.send(f"Default model set to `{model_name}`.")

@bot.command(name="config")
async def show_config(ctx):
    """Shows server configuration. Restricted to administrators."""
    if not (ctx.author.guild_permissions.administrator or ctx.author == ctx.guild.owner):
        await ctx.send("You do not have permission to view server configuration.")
        return
    guild_id = ctx.guild.id
    config = server_configs.get(guild_id, {})
    config_str = json.dumps(config, indent=2)
    pages = paginate_text(config_str, 1850)
    for p in pages:
        await ctx.send(f"```json\n{p}\n```")

@bot.command(name="models")
async def models_command(ctx):
    """Lists available models from the active AI provider."""
    guild_id = ctx.guild.id
    config = server_configs.get(guild_id, {})
    current_model = config.get("default_model", "")
    provider = get_provider()
    try:
        models = await provider.list_models()
    except Exception as e:
        await ctx.send(f"Error fetching models from {provider.name}: {e}")
        return

    if not models:
        await ctx.send(f"No models found on {provider.name} ({provider.base_url}).")
        return

    msg = f"Available Models ({provider.name}):\n"
    for m in models:
        if m == current_model:
            msg += f"- `{m}` (selected)\n"
        else:
            msg += f"- `{m}`\n"
    await ctx.send(msg)

@bot.command(name="setpagination")
async def set_pagination(ctx, max_chars: int):
    """Sets pagination character limit (500 to 1900)."""
    if not ctx.author.guild_permissions.administrator:
        await ctx.send("You do not have permission to change pagination settings.")
        return
    if max_chars < 500 or max_chars > 1900:
        await ctx.send("Please choose a value between 500 and 1900 characters.")
        return
    guild_id = ctx.guild.id
    config = server_configs.get(guild_id, {})
    config["pagination_max_chars"] = max_chars
    save_config(guild_id, config)
    server_configs[guild_id] = config
    await ctx.send(f"Pagination character limit set to {max_chars}.")

@bot.command(name="help")
async def help_command(ctx):
    guild_id = ctx.guild.id
    config = server_configs.get(guild_id, {})
    max_chars = min(config.get("pagination_max_chars", 1850), 1850)
    prefix = COMMAND_PREFIX
    provider = get_provider()
    help_text = (
        f"Silas Blue Bot Help (Backend: {provider.name})\n\n"
        f"Usage:\n"
        f"Send a prompt by mentioning the bot or using prefix {prefix} followed by message.\n"
        f"Example: {prefix}Tell me a short story\n\n"
        f"Commands:\n"
        f"{prefix}ping - Check bot response\n"
        f"{prefix}setmodel <name> - Set default model for this server\n"
        f"{prefix}models - List available models on AI backend\n"
        f"{prefix}setpagination <500-1900> - Set pagination limit\n"
        f"{prefix}config - Show server config (Admin only)\n"
        f"{prefix}help - Show this message\n"
    )
    pages = paginate_text(help_text, max_chars)
    for p in pages:
        await ctx.send(f"```\n{p}\n```")

# --- Lifecycle Management ---
_bot_thread: Optional[threading.Thread] = None
_bot_loop: Optional[asyncio.AbstractEventLoop] = None
_shutdown_event: Optional[threading.Event] = None
_bot_instance = bot  # Single canonical instance

def start_bot():
    global _bot_thread, _bot_loop, _shutdown_event, DISCORD_TOKEN
    if _bot_thread and _bot_thread.is_alive():
        logger.info("Bot is already running.")
        return

    if not DISCORD_TOKEN:
        DISCORD_TOKEN = get_discord_token()

    _shutdown_event = threading.Event()

    def run():
        global _bot_loop
        _bot_loop = asyncio.new_event_loop()
        asyncio.set_event_loop(_bot_loop)
        try:
            _bot_loop.run_until_complete(_run_bot(_shutdown_event, bot))
        finally:
            _bot_loop.run_until_complete(_bot_loop.shutdown_asyncgens())
            _bot_loop.close()

    _bot_thread = threading.Thread(target=run, daemon=True)
    _bot_thread.start()
    logger.info("Bot thread started.")

def stop_bot():
    global _shutdown_event, _bot_thread
    if _shutdown_event:
        _shutdown_event.set()
    if _bot_thread:
        _bot_thread.join(timeout=10)
    logger.info("Bot stopped.")

def restart_bot():
    stop_bot()
    time.sleep(1)
    start_bot()
    logger.info("Bot restarted.")

async def _run_bot(shutdown_event: threading.Event, bot_inst: commands.Bot):
    shutdown_asyncio = asyncio.Event()

    def check_shutdown():
        if shutdown_event.is_set():
            shutdown_asyncio.set()
        elif _bot_loop and _bot_loop.is_running():
            _bot_loop.call_later(0.5, check_shutdown)

    _bot_loop.call_soon(check_shutdown)
    bot_task = asyncio.create_task(bot_inst.start(DISCORD_TOKEN))
    await shutdown_asyncio.wait()
    await bot_inst.close()
    try:
        await bot_task
    except Exception:
        pass

def run_discord_bot():
    start_bot()