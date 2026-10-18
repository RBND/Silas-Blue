"""
Discord bot core for Silas Blue.
Handles commands, permissions, per-server config, and unified AI provider integration.
Fully asynchronous and non-blocking for Discord Gateway stability.
"""

import discord
from discord.ext import commands
import logging
import os
import base64
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
from memory import conversation_memory

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

def get_client_id_from_token(token: Optional[str] = None) -> Optional[int]:
    """Extracts the client ID from a Discord bot token by decoding its first component."""
    if token is None:
        token_path = os.path.join("config", "bot_token.txt")
        if os.path.exists(token_path):
            try:
                with open(token_path, "r", encoding="utf-8") as f:
                    token = f.read().strip()
            except Exception:
                token = None
    if not token:
        return None
    try:
        part = token.split(".")[0].strip()
        padding = "=" * (-len(part) % 4)
        decoded = base64.b64decode(part + padding).decode("utf-8")
        if decoded.isdigit():
            return int(decoded)
    except Exception as e:
        logger.debug(f"Could not extract client ID from token: {e}")
    return None

def get_bot_client_id() -> Optional[int]:
    """Returns the bot's application/client ID if known."""
    if bot.user and getattr(bot.user, "id", None):
        return bot.user.id
    if getattr(bot, "application_id", None):
        return bot.application_id
    global DISCORD_TOKEN
    return get_client_id_from_token(DISCORD_TOKEN)

def get_bot_invite_url(
    client_id: Optional[int] = None,
    administrator: bool = False,
    permissions: Optional[discord.Permissions] = None
) -> Optional[str]:
    """
    Generates a Discord OAuth2 authorization URL to invite the bot to a server.
    If client_id is not specified, it will attempt to detect it from the bot or token.
    """
    cid = client_id or get_bot_client_id()
    if not cid:
        return None

    if administrator:
        perms = discord.Permissions(administrator=True)
    elif permissions is not None:
        perms = permissions
    else:
        # Standard recommended permissions for Silas Blue
        perms = discord.Permissions(
            view_channel=True,
            send_messages=True,
            send_messages_in_threads=True,
            embed_links=True,
            attach_files=True,
            read_message_history=True,
            add_reactions=True,
            use_external_emojis=True,
        )

    return discord.utils.oauth_url(cid, permissions=perms, scopes=("bot", "applications.commands"))

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

async def handle_ai_prompt(message: discord.Message, config: dict, prompt_override: Optional[str] = None):
    """
    Sends message to active AI provider asynchronously without blocking event loop.
    Validates model against active provider and automatically falls back to currently loaded model.
    Displays typing indicator while processing.
    Integrates conversation memory history and native Discord reply context.
    """
    raw_prompt = (prompt_override if prompt_override is not None else message.content).strip()
    provider = get_provider()

    # Determine default model from active provider and configuration
    app_cfg = load_app_config()
    available = await provider.list_models()
    active_loaded = available[0] if available else "default"

    guild_id = message.guild.id if message.guild else None
    guild_name = message.guild.name if message.guild else "Direct Message"

    if message.guild is None:
        mem_key = f"dm_user_{message.author.id}"
        max_turns = int(app_cfg.get("dm_memory_turns", 6))
        ttl_minutes = int(app_cfg.get("dm_memory_ttl_minutes", 30))
        # DM handling: check dm_default_model or active_model
        dm_model = app_cfg.get("dm_default_model")
        if dm_model and (not available or dm_model in available):
            default_model = dm_model
        elif app_cfg.get("active_model") and app_cfg.get("active_model") in available:
            default_model = app_cfg.get("active_model")
        else:
            default_model = active_loaded
    else:
        mem_key = f"guild_{message.guild.id}_{message.channel.id}"
        max_turns = int(config.get("memory_turns", 6))
        ttl_minutes = int(config.get("memory_ttl_minutes", 30))
        # Guild handling:
        server_model = config.get("default_model")
        # If server has a model configured that actually exists on current provider, use it
        if server_model and (not available or server_model in available):
            default_model = server_model
        # Otherwise, if app_config has an active_model that exists on current provider, use that
        elif app_cfg.get("active_model") and app_cfg.get("active_model") in available:
            default_model = app_cfg.get("active_model")
            config["default_model"] = default_model
            save_config(guild_id, config)
        # Otherwise, fall back to the actively loaded model on the provider
        else:
            default_model = active_loaded
            if server_model != default_model and available:
                config["default_model"] = default_model
                save_config(guild_id, config)

    # Resolve replied-to message context if user replied directly to a Discord message
    replied_context = ""
    if message.reference and message.reference.message_id and not prompt_override:
        try:
            ref_msg = message.reference.resolved
            if ref_msg is None or isinstance(ref_msg, discord.DeletedReferencedMessage):
                ref_msg = await message.channel.fetch_message(message.reference.message_id)
            if ref_msg and ref_msg.content:
                author_name = getattr(ref_msg.author, "display_name", str(ref_msg.author))
                replied_context = f"[Replying to {author_name}: \"{ref_msg.content}\"]\n"
        except Exception as ref_err:
            logger.debug(f"Could not resolve referenced message: {ref_err}")

    effective_prompt = f"{replied_context}{raw_prompt}" if replied_context else raw_prompt

    # Retrieve multi-turn history from memory buffer
    history = None
    if max_turns > 0:
        history = conversation_memory.get_history(mem_key, max_turns=max_turns, ttl_minutes=ttl_minutes)

    log_to_gui("prompt", {
        "guild_id": guild_id,
        "guild_name": guild_name,
        "user": str(message.author),
        "prompt": effective_prompt
    })

    # Show Discord typing indicator during model inference
    # Server-specific system prompt takes precedence over global default
    server_prompt = (config.get("system_prompt") or "").strip() if config else ""
    global_prompt = (app_cfg.get("system_prompt") or "").strip()
    system_prompt = server_prompt if server_prompt else (global_prompt or None)

    try:
        async with message.channel.typing():
            response = await provider.send_prompt(effective_prompt, default_model, system_prompt=system_prompt, history=history)
    except Exception as e:
        logger.error(f"Error getting AI response: {e}")
        response = f"Error processing AI request: {e}"

    # Record turn in conversation memory if response was valid
    if max_turns > 0 and response and not response.startswith("Error processing AI request:") and not response.startswith("Ollama Error (") and not response.startswith("Llama.cpp Error (") and not response.startswith("OpenAI/Remote Error ("):
        conversation_memory.add_turn(mem_key, effective_prompt, response, max_turns=max_turns)

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
    log_to_gui("server_joined", {"guild_id": guild.id, "guild_name": guild.name})

@bot.event
async def on_guild_remove(guild):
    logger.info(f"Left server: {guild.name} ({guild.id})")
    log_to_gui("server_left", {"guild_id": guild.id, "guild_name": guild.name})

@bot.event
async def on_message(message: discord.Message):
    if message.author == bot.user:
        return

    # Direct Message (DM) Policy & Command Handling
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

        # Check for commands in DMs (e.g. !clear, !recall, !help)
        content = message.content.strip()
        if content.startswith(COMMAND_PREFIX):
            first_word = content[len(COMMAND_PREFIX):].split(" ", 1)[0].lower()
            command_names = [cmd.name for cmd in bot.commands]
            for cmd in bot.commands:
                command_names.extend(cmd.aliases)
            if first_word in command_names:
                await bot.process_commands(message)
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

    content = message.content
    mention_str_1 = f"<@{bot.user.id}>"
    mention_str_2 = f"<@!{bot.user.id}>"

    # Check for command or prefix-based prompt
    if content.startswith(COMMAND_PREFIX):
        first_word = content[len(COMMAND_PREFIX):].split(" ", 1)[0].lower()
        command_names = [cmd.name for cmd in bot.commands]
        for cmd in bot.commands:
            command_names.extend(cmd.aliases)
        if first_word in command_names:
            await bot.process_commands(message)
        else:
            prompt_text = content[len(COMMAND_PREFIX):].lstrip()
            if prompt_text:
                await handle_ai_prompt(message, config, prompt_override=prompt_text)
        return

    # Check for direct bot mention
    if content.startswith(mention_str_1) or content.startswith(mention_str_2):
        after_mention = content[len(mention_str_1):] if content.startswith(mention_str_1) else content[len(mention_str_2):]
        after_mention = after_mention.lstrip()
        first_word = after_mention.split(" ", 1)[0].lower() if after_mention else ""
        command_names = [cmd.name for cmd in bot.commands]
        for cmd in bot.commands:
            command_names.extend(cmd.aliases)
        if first_word in command_names:
            message.content = COMMAND_PREFIX + after_mention
            await bot.process_commands(message)
        elif after_mention:
            await handle_ai_prompt(message, config, prompt_override=after_mention)
        return

    # Check if user replied directly to a message authored by the bot
    if message.reference and message.reference.message_id:
        try:
            ref_msg = message.reference.resolved
            if ref_msg is None or isinstance(ref_msg, discord.DeletedReferencedMessage):
                ref_msg = await message.channel.fetch_message(message.reference.message_id)
            if ref_msg and ref_msg.author == bot.user:
                await handle_ai_prompt(message, config)
                return
        except Exception:
            pass

    # Check if bot was mentioned elsewhere in the message
    if bot.user in message.mentions:
        await handle_ai_prompt(message, config)

@bot.command(name="ping")
async def ping(ctx):
    await ctx.send("Pong!")

@bot.command(name="invite")
async def invite_command(ctx):
    """Generates an invite link to add Silas Blue to a Discord server."""
    invite_url = get_bot_invite_url()
    if invite_url:
        await ctx.send(f"**Invite Silas Blue to your server:**\n<{invite_url}>")
    else:
        await ctx.send("Unable to generate invite URL. Bot client ID could not be determined.")

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

@bot.command(name="setprompt", aliases=["systemprompt", "setpersona"])
async def set_prompt(ctx, *, prompt: str = ""):
    """Sets a custom system prompt / persona for this server, or resets to default if empty."""
    if not ctx.guild:
        await ctx.send("This command can only be used within a server.")
        return
    guild_id = ctx.guild.id
    config = server_configs.get(guild_id, {})
    roles = config.get("change_permission_roles", ["admin", "owner"])

    is_admin = ctx.author.guild_permissions.administrator or (ctx.author == ctx.guild.owner)
    has_role = any(r.name.lower() in [x.lower() for x in roles] for r in getattr(ctx.author, "roles", []))
    if not (is_admin or has_role):
        await ctx.send("You do not have permission to change this server's system prompt.")
        return

    prompt = prompt.strip()
    if not prompt or prompt.lower() in ("reset", "default", "none"):
        config["system_prompt"] = ""
        save_config(guild_id, config)
        server_configs[guild_id] = config
        await ctx.send("✓ Server system prompt reset to global default.")
    else:
        config["system_prompt"] = prompt
        save_config(guild_id, config)
        server_configs[guild_id] = config
        snippet = prompt[:80] + "..." if len(prompt) > 80 else prompt
        await ctx.send(f"✓ Server system prompt updated:\n> *{snippet}*")

@bot.command(name="setnick", aliases=["setname"])
async def set_nick(ctx, *, new_name: str):
    """Changes the bot's nickname in this server (Admin only)."""
    if not (ctx.author.guild_permissions.administrator or ctx.author == ctx.guild.owner):
        await ctx.send("You need Administrator permissions to change the bot's nickname.")
        return
    new_name = new_name.strip()
    if not new_name:
        await ctx.send("Please provide a new nickname.")
        return
    try:
        await ctx.guild.me.edit(nick=new_name)
        await ctx.send(f"Bot nickname updated to **{new_name}** in this server.")
    except discord.Forbidden:
        await ctx.send("I don't have permission to change my nickname. Please grant the 'Change Nickname' or 'Manage Nicknames' permission.")
    except Exception as e:
        await ctx.send(f"Error changing nickname: {e}")

@bot.command(name="setglobalname")
async def set_global_name(ctx, *, new_name: str):
    """Changes the bot's global Discord username (Bot owner only, rate-limited by Discord)."""
    if ctx.author != ctx.guild.owner and not ctx.author.guild_permissions.administrator:
        await ctx.send("You need Administrator permissions to use this command.")
        return
    new_name = new_name.strip()
    if len(new_name) < 2 or len(new_name) > 32:
        await ctx.send("Discord usernames must be between 2 and 32 characters.")
        return
    try:
        await bot.user.edit(username=new_name)
        await ctx.send(f"Global bot username changed to **{new_name}**.")
    except discord.HTTPException as e:
        await ctx.send(f"Discord rejected the username change ({e}). Note that Discord rate-limits global username changes (usually max 2 per hour). You can also change it directly on the Discord Developer Portal: https://discord.com/developers/applications")
    except Exception as e:
        await ctx.send(f"Error changing global username: {e}")

@bot.command(name="clear", aliases=["reset", "forget"])
async def clear_memory_command(ctx):
    """Clears conversation memory for the current channel or DM."""
    if ctx.guild is None:
        mem_key = f"dm_user_{ctx.author.id}"
        conversation_memory.clear(mem_key)
        await ctx.send("🧹 Your DM conversation memory has been cleared.")
    else:
        mem_key = f"guild_{ctx.guild.id}_{ctx.channel.id}"
        conversation_memory.clear(mem_key)
        await ctx.send("🧹 Conversation memory for this channel has been cleared.")

@bot.command(name="recall")
async def recall_command(ctx, target: str, *, instructions: str = ""):
    """Recalls a specific previous message by ID or jump link and analyzes/responds to it."""
    clean_target = target.strip("<>").rstrip("/")
    msg_id_str = clean_target.split("/")[-1]
    try:
        msg_id = int(msg_id_str)
    except ValueError:
        await ctx.send("⚠️ Invalid message ID or link provided.")
        return

    try:
        target_msg = await ctx.channel.fetch_message(msg_id)
    except discord.NotFound:
        await ctx.send("⚠️ Message not found in this channel.")
        return
    except discord.Forbidden:
        await ctx.send("⚠️ I don't have permission to read message history here.")
        return
    except Exception as e:
        await ctx.send(f"⚠️ Failed to fetch message: {e}")
        return

    author_name = getattr(target_msg.author, "display_name", str(target_msg.author))
    target_content = target_msg.content or "[Non-text content / attachment]"

    prompt_text = f"[Recalled message from {author_name} (ID: {target_msg.id})]:\n\"{target_content}\""
    if instructions:
        prompt_text += f"\n\nUser instructions regarding this message: {instructions}"
    else:
        prompt_text += "\n\nPlease summarize, analyze, or respond to this recalled message."

    config = server_configs.get(ctx.guild.id, {}) if ctx.guild else load_config(None)
    await handle_ai_prompt(ctx.message, config, prompt_override=prompt_text)

@bot.command(name="setmemory")
async def set_memory(ctx, turns: int):
    """Sets the number of conversation turns to remember in this server (0 to disable)."""
    if ctx.guild is None:
        await ctx.send("This command can only be used in a server.")
        return
    guild_id = ctx.guild.id
    config = server_configs.get(guild_id, {})
    roles = config.get("change_permission_roles", ["admin", "owner"])
    is_admin = ctx.author.guild_permissions.administrator or (ctx.author == ctx.guild.owner)
    has_role = any(r.name.lower() in [x.lower() for x in roles] for r in getattr(ctx.author, "roles", []))
    if not (is_admin or has_role):
        await ctx.send("You do not have permission to configure memory settings.")
        return
    if turns < 0 or turns > 50:
        await ctx.send("Please provide a number of memory turns between 0 and 50.")
        return
    config["memory_turns"] = turns
    save_config(guild_id, config)
    server_configs[guild_id] = config
    if turns == 0:
        await ctx.send("Conversation memory has been **disabled** for this server.")
    else:
        await ctx.send(f"Conversation memory set to **{turns}** turns for this server.")

@bot.command(name="setttl", aliases=["setmemoryttl"])
async def set_ttl(ctx, minutes: int):
    """Sets the memory inactivity expiration time in minutes for this server (1 to 1440)."""
    if ctx.guild is None:
        await ctx.send("This command can only be used in a server.")
        return
    guild_id = ctx.guild.id
    config = server_configs.get(guild_id, {})
    roles = config.get("change_permission_roles", ["admin", "owner"])
    is_admin = ctx.author.guild_permissions.administrator or (ctx.author == ctx.guild.owner)
    has_role = any(r.name.lower() in [x.lower() for x in roles] for r in getattr(ctx.author, "roles", []))
    if not (is_admin or has_role):
        await ctx.send("You do not have permission to configure memory settings.")
        return
    if minutes < 1 or minutes > 1440:
        await ctx.send("Please provide an inactivity TTL between 1 and 1440 minutes (24 hours).")
        return
    config["memory_ttl_minutes"] = minutes
    save_config(guild_id, config)
    server_configs[guild_id] = config
    await ctx.send(f"Conversation memory inactivity TTL set to **{minutes}** minutes for this server.")

@bot.command(name="help")
async def help_command(ctx):
    guild_id = ctx.guild.id if ctx.guild else None
    config = server_configs.get(guild_id, {}) if guild_id else load_config(None)
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
        f"{prefix}invite - Get invite link to add Silas Blue to a server\n"
        f"{prefix}setmodel <name> - Set default model for this server\n"
        f"{prefix}models - List available models on AI backend\n"
        f"{prefix}setprompt <text> - Set custom AI persona for this server (use 'reset' for default)\n"
        f"{prefix}clear / {prefix}reset - Clear channel or DM conversation memory\n"
        f"{prefix}recall <msg_id_or_link> [notes] - Recall a previous message and analyze/respond\n"
        f"{prefix}setmemory <0-50> - Set conversation memory turns for this server (0 = off)\n"
        f"{prefix}setttl <1-1440> - Set memory inactivity TTL in minutes for this server\n"
        f"{prefix}setnick <name> - Change bot nickname in this server\n"
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