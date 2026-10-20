"""
Discord bot core for Silas Blue.
Handles commands, permissions, per-server config, and unified AI provider integration.
Fully asynchronous and non-blocking for Discord Gateway stability.
"""

import discord
from discord.ext import commands
import logging
import os
import re
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
from token_limiter import token_limiter
from utils import load_config, save_config, get_config_path, load_app_config, save_app_config
from memory import conversation_memory
from updater import BotUpdater, auto_update_background_loop
from web_search import (
    SearchService,
    WebSearchResult,
    should_auto_search,
    build_search_query,
    format_grounded_prompt
)

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
current_search_service: Optional[SearchService] = None

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

def get_search_service() -> SearchService:
    global current_search_service
    if current_search_service is None:
        app_cfg = load_app_config()
        current_search_service = SearchService(app_cfg)
    return current_search_service

def reload_search_service():
    """Reloads search service with updated configuration."""
    global current_search_service
    app_cfg = load_app_config()
    current_search_service = SearchService(app_cfg)
    logger.info(f"Web search service reloaded (Provider: {app_cfg.get('web_search_provider', 'duckduckgo')})")

def reload_ai_provider():
    """Reloads the active AI provider with updated app settings."""
    global current_ai_provider
    current_ai_provider = get_ai_provider()
    reload_search_service()
    logger.info(f"AI Provider reloaded: {current_ai_provider.name} ({current_ai_provider.base_url})")

def update_server_config(guild_id, config: dict):
    """Updates in-memory server config immediately from GUI / Web saves."""
    try:
        server_configs[int(guild_id)] = config
    except (ValueError, TypeError):
        pass
    server_configs[str(guild_id)] = config
    logger.debug(f"In-memory config updated for guild {guild_id}")

def log_to_gui(event_type: str, data: dict):
    """Pushes event to the in-memory thread-safe queue without blocking event loop with disk I/O."""
    entry = {
        "event": event_type,
        "data": data,
        "timestamp": time.time()
    }
    gui_event_queue.put(entry)

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
    def __init__(self, pages: List[str], author_id: int, timeout: Optional[int] = 180, allow_any_user: bool = False):
        super().__init__(timeout=timeout)
        self.pages = pages
        self.author_id = author_id
        self.allow_any_user = allow_any_user
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
        if not self.allow_any_user and interaction.user.id != self.author_id:
            await interaction.response.send_message("You can't control this pagination.", ephemeral=True)
            return
        if self.current_page > 0:
            self.current_page -= 1
            await self.update_message(interaction)

    @discord.ui.button(label="Next", style=discord.ButtonStyle.primary, disabled=False)
    async def next_button(self, interaction: discord.Interaction, button: discord.ui.Button):
        if not self.allow_any_user and interaction.user.id != self.author_id:
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

async def handle_ai_prompt(
    message: discord.Message,
    config: dict,
    prompt_override: Optional[str] = None,
    force_web_search: bool = False,
    system_prompt_override: Optional[str] = None,
    is_forum_auto_reply: bool = False
):
    """
    Sends message to active AI provider asynchronously without blocking event loop.
    Validates model against active provider and automatically falls back to currently loaded model.
    Displays typing indicator while processing.
    Integrates conversation memory history, native Discord reply context, and RAG web search grounding.
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

    # Web search configuration check & intent detection
    guild_search_enabled = config.get("web_search_enabled", True) if message.guild else True
    app_search_enabled = app_cfg.get("web_search_enabled", True)
    search_enabled = guild_search_enabled and app_search_enabled

    guild_auto_search = config.get("web_search_auto", True) if message.guild else True
    app_auto_search = app_cfg.get("web_search_auto", True)
    auto_search_enabled = search_enabled and guild_auto_search and app_auto_search

    can_search = permissions.can_search(message.author, message.guild, config)
    has_auto_search = auto_search_enabled and should_auto_search(raw_prompt)
    do_web_search = can_search and (force_web_search or has_auto_search)

    search_results = []
    citations_footer = ""

    # Base system prompt
    if system_prompt_override is not None:
        server_prompt = system_prompt_override.strip()
    else:
        server_prompt = (config.get("system_prompt") or "").strip() if config else ""
    global_prompt = (app_cfg.get("system_prompt") or "").strip()
    if server_prompt and global_prompt:
        base_sys_prompt = f"{global_prompt}\n\n### SERVER-SPECIFIC PERSONA & INSTRUCTIONS\n{server_prompt}"
    else:
        base_sys_prompt = server_prompt if server_prompt else (global_prompt or None)

    # Check role-based token quotas
    quota_result = None
    if message.guild:
        quota_result = token_limiter.check_prompt_allowed(guild_id, message.author, config)
        if not quota_result.allowed:
            reset_display = f"<t:{quota_result.reset_epoch}:R>" if quota_result.reset_epoch else "the next reset"
            overage_policy = config.get("token_quotas", {}).get("overage_policy", {})
            overage_note = ""
            if not overage_policy.get("allow_partial_overage", True):
                overage_note = " *(Strict limit: partial overage disabled)*"
            try:
                await message.channel.send(
                    f"⚠️ **Token Quota Reached**: You have used **{quota_result.used:,} / {quota_result.limit:,} tokens** "
                    f"for your role **@{quota_result.role_name}**{overage_note}.\n"
                    f"Your quota resets {reset_display}."
                )
            except Exception as send_err:
                logger.warning(f"Could not send token limit notice: {send_err}")
            return

    try:
        async with message.channel.typing():
            # Perform web search if needed
            if do_web_search:
                try:
                    search_service = get_search_service()
                    search_query = build_search_query(raw_prompt, replied_context=replied_context, history=history)
                    search_results = await search_service.search(search_query)
                    if search_results:
                        log_to_gui("web_search", {
                            "guild_id": guild_id,
                            "guild_name": guild_name,
                            "user": str(message.author),
                            "query": search_query,
                            "results_count": len(search_results)
                        })
                        system_prompt, citations_footer = format_grounded_prompt(
                            search_query, search_results, base_system_prompt=base_sys_prompt
                        )
                    else:
                        system_prompt = base_sys_prompt
                except Exception as s_err:
                    logger.warning(f"Web search error, proceeding with standard AI response: {s_err}")
                    system_prompt = base_sys_prompt
            else:
                system_prompt = base_sys_prompt

            max_tokens_clamp = quota_result.effective_max_tokens if (quota_result and quota_result.effective_max_tokens) else None
            response = await provider.send_prompt(
                effective_prompt,
                default_model,
                system_prompt=system_prompt,
                history=history,
                max_tokens=max_tokens_clamp
            )
    except Exception as e:
        logger.error(f"Error getting AI response: {e}")
        response = f"Error processing AI request: {e}"

    is_error_response = (
        not response
        or response.startswith("Error processing AI request:")
        or response.startswith("Ollama Error (")
        or response.startswith("Llama.cpp Error (")
        or response.startswith("OpenAI/Remote Error (")
    )

    if not is_error_response:
        # Record token consumption in token limiter
        if message.guild and quota_result:
            usage = getattr(provider, "last_usage", {})
            try:
                tot_tokens = int(usage.get("total_tokens", 0)) if isinstance(usage, dict) else 0
                p_tokens = int(usage.get("prompt_tokens", 0)) if isinstance(usage, dict) else 0
                c_tokens = int(usage.get("completion_tokens", 0)) if isinstance(usage, dict) else 0
            except (ValueError, TypeError):
                tot_tokens, p_tokens, c_tokens = 0, 0, 0

            if tot_tokens > 0:
                token_limiter.record_tokens(
                    guild_id,
                    message.author.id,
                    tokens=tot_tokens,
                    prompt_tokens=p_tokens,
                    completion_tokens=c_tokens,
                    model=default_model
                )

        if citations_footer:
            should_append_citations = True
            if not force_web_search:
                refusal_check = re.search(
                    r"\b(?:i cannot|i can't|i am unable to|i am not able to|do not have the ability|don't have the ability|do not have permission|don't have permission)\s+to\s+(?:shut down|shutdown|turn off|power off|restart|reboot|execute|run|access)\b",
                    response,
                    flags=re.IGNORECASE
                )
                if refusal_check:
                    should_append_citations = False
            if should_append_citations:
                response = response.strip() + citations_footer
    elif force_web_search and search_results:
        # Fallback if model fails during explicit search command
        fallback_lines = ["⚠️ *AI service is currently unavailable, but here are the top search results for your query:*\n"]
        for idx, r in enumerate(search_results, 1):
            clean_title = r.title.replace("[", "").replace("]", "")
            fallback_lines.append(f"**{idx}. [{clean_title}](<{r.url}>)**\n> {r.snippet}\n")
        response = "\n".join(fallback_lines)

    # Record turn in conversation memory if response was valid
    if max_turns > 0 and not is_error_response:
        conversation_memory.add_turn(mem_key, effective_prompt, response, max_turns=max_turns)

    log_to_gui("reply", {
        "guild_id": guild_id,
        "guild_name": guild_name,
        "user": str(message.author),
        "reply": response
    })

    max_chars = min(config.get("pagination_max_chars", 1900), 1900)
    pages = paginate_text(response, max_chars)

    if len(pages) == 1 or (is_forum_auto_reply and config.get("forum_delivery_mode", "all_chunks") == "disabled"):
        text = pages[0]
        if len(text) > 2000:
            text = text[:1997] + "..."
        await message.channel.send(text)
    elif is_forum_auto_reply and config.get("forum_delivery_mode", "all_chunks") == "all_chunks":
        # Send all chunks sequentially with Part indicators so the full answer is permanent
        total = len(pages)
        for idx, page_content in enumerate(pages, 1):
            chunk_text = f"**[Part {idx}/{total}]**\n{page_content}"
            if len(chunk_text) > 2000:
                chunk_text = chunk_text[:1997] + "..."
            await message.channel.send(chunk_text)
            if idx < total:
                await asyncio.sleep(0.5)
    else:
        # Interactive pagination buttons (persistent timeout and open to any thread participant if forum auto-reply)
        view_timeout = None if is_forum_auto_reply else 180
        view = PaginatedView(pages, message.author.id, timeout=view_timeout, allow_any_user=is_forum_auto_reply)
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

async def dispatch_post_update_announcement(bot_client: commands.Bot, update_data: dict):
    """Sends confirmation embed to Discord after a successful update restart."""
    app_cfg = load_app_config()
    notify_ctx = update_data.get("notify_context", {})
    prev_ver = update_data.get("previous_version", "unknown")
    new_ver = update_data.get("new_version", "unknown")
    prev_commit = update_data.get("previous_commit", "")[:7]
    new_commit = update_data.get("new_commit", "")[:7]
    channel_name = update_data.get("channel", "stable").title()
    changelog = update_data.get("changelog", [])

    embed = discord.Embed(
        title="✨ Silas Blue Updated Successfully!",
        description=(
            f"Silas Blue has been updated and restarted cleanly.\n\n"
            f"• **Version:** `{prev_ver}` → `{new_ver}`\n"
            f"• **Commit:** `{prev_commit}` → `{new_commit}`\n"
            f"• **Channel:** `{channel_name}`\n"
        ),
        color=get_discord_theme_color()
    )
    if changelog:
        cl_text = "\n".join(f"• {c}" for c in changelog[:6])
        embed.add_field(name="Recent Changes", value=cl_text, inline=False)

    sent = False

    # 1. Send to the channel where /update was run
    invoked_channel_id = notify_ctx.get("channel_id")
    if invoked_channel_id:
        try:
            target_chan = bot_client.get_channel(int(invoked_channel_id))
            if target_chan:
                await target_chan.send(embed=embed)
                sent = True
        except Exception as e:
            logger.debug(f"Could not send to invoked channel: {e}")

    # 2. Send to configured notify channel if different
    cfg_channel_id = app_cfg.get("update_notify_channel_id")
    if cfg_channel_id and str(cfg_channel_id) != str(invoked_channel_id):
        try:
            target_chan = bot_client.get_channel(int(cfg_channel_id))
            if target_chan:
                await target_chan.send(embed=embed)
                sent = True
        except Exception as e:
            logger.debug(f"Could not send to notify channel: {e}")

    # 3. Send to bot owner DM if configured or if no channel was notified
    if app_cfg.get("update_notify_owner_dm", True) and not sent:
        try:
            app_info = await bot_client.application_info()
            if app_info.owner:
                await app_info.owner.send(embed=embed)
        except Exception as e:
            logger.debug(f"Could not send update DM to bot owner: {e}")

async def dispatch_update_alert(bot_client: commands.Bot, update_info: dict):
    """Sends notification when a new update is detected by the background scheduler."""
    app_cfg = load_app_config()
    embed = discord.Embed(
        title="🔔 Silas Blue Update Available",
        description=(
            f"A new update was detected on the **{update_info.get('channel', 'stable').title()}** channel!\n\n"
            f"• **Current:** `{update_info.get('current_version')}` (`{update_info.get('current_commit')}`)\n"
            f"• **Available:** `{update_info.get('latest_version')}` (`{update_info.get('latest_commit')}`)\n"
        ),
        color=get_discord_theme_color()
    )
    if update_info.get("changelog"):
        cl = "\n".join(f"• {c}" for c in update_info["changelog"][:5])
        embed.add_field(name="Highlights", value=cl, inline=False)

    embed.set_footer(text="Run /update apply or !update apply to update now.")

    cfg_channel_id = app_cfg.get("update_notify_channel_id")
    if cfg_channel_id:
        try:
            target_chan = bot_client.get_channel(int(cfg_channel_id))
            if target_chan:
                await target_chan.send(embed=embed)
                return
        except Exception:
            pass

    if app_cfg.get("update_notify_owner_dm", True):
        try:
            app_info = await bot_client.application_info()
            if app_info.owner:
                await app_info.owner.send(embed=embed)
        except Exception:
            pass

# Bind alert callback to bot instance
bot.dispatch_update_alert = lambda info: dispatch_update_alert(bot, info)

@bot.event
async def on_ready():
    logger.info(f"Silas Blue is online as {bot.user} (ID: {bot.user.id})")
    await bot.change_presence(activity=discord.Game(name="git.new/silasblue"))
    for guild in bot.guilds:
        config = load_config(guild.id)
        server_configs[guild.id] = config
    logger.info(f"Loaded configurations for {len(bot.guilds)} server(s).")

    # Verify post-update health check and dispatch notification
    try:
        post_update_info = BotUpdater.verify_boot_health()
        if post_update_info:
            logger.info("Bot boot verified following update. Dispatching update announcement...")
            await dispatch_post_update_announcement(bot, post_update_info)
    except Exception as e:
        logger.error(f"Error handling post-update verification: {e}")

    # Synchronize Discord Slash Commands
    try:
        await bot.tree.sync()
        logger.info("Synchronized Discord slash commands.")
    except Exception as e:
        logger.warning(f"Failed to sync slash commands: {e}")

    # Launch background auto-update loop
    bot.loop.create_task(auto_update_background_loop(bot))

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

# Thread IDs processed by forum auto-reply or on_message to prevent duplicate answers
_handled_forum_thread_ids: set = set()

async def process_forum_auto_reply(thread: discord.Thread, starter_msg: Optional[discord.Message] = None):
    """
    Core handler for auto-replying to newly created posts in Discord Forum channels.
    Can be invoked from either on_thread_create or on_message (for starter posts).
    """
    if not thread.guild:
        return

    # Check if thread was already handled
    if thread.id in _handled_forum_thread_ids:
        return

    guild_id = thread.guild.id
    config = server_configs.get(guild_id) or server_configs.get(str(guild_id))
    if not config:
        config = load_config(guild_id)
        server_configs[guild_id] = config

    if not config.get("forum_auto_reply_enabled", False):
        return

    # Resolve parent channel
    parent = thread.parent
    if parent is None and getattr(thread, "parent_id", None):
        parent = thread.guild.get_channel(thread.parent_id)
        if parent is None:
            try:
                parent = await thread.guild.fetch_channel(thread.parent_id)
            except Exception as e:
                logger.debug(f"Could not fetch parent channel for thread {thread.id}: {e}")
                return

    # Verify parent is a ForumChannel
    is_forum = isinstance(parent, getattr(discord, "ForumChannel", ())) or getattr(parent, "type", None) == getattr(discord.ChannelType, "forum", None)
    if not is_forum:
        return

    # Channel whitelist check
    monitored_channels = [str(c).strip() for c in config.get("forum_monitored_channels", []) if str(c).strip()]
    if monitored_channels and str(parent.id) not in monitored_channels:
        logger.info(f"Forum thread '{thread.name}' ({thread.id}) in channel {parent.id} skipped (not in monitored channel whitelist: {monitored_channels})")
        return

    # Tag filter check
    applied_tags = getattr(thread, "applied_tags", []) or []
    applied_names = {t.name.casefold().strip() for t in applied_tags if hasattr(t, "name")}
    applied_ids = {str(t.id).strip() for t in applied_tags if hasattr(t, "id")}

    # 1. Ignored tags check (blacklist)
    ignore_tags = {str(t).casefold().strip() for t in config.get("forum_ignore_tags", []) if str(t).strip()}
    if ignore_tags and ((applied_names & ignore_tags) or (applied_ids & ignore_tags)):
        logger.info(f"Forum post '{thread.name}' skipped due to ignored tag filter.")
        return

    # 2. Required tags check (whitelist)
    req_tags = {str(t).casefold().strip() for t in config.get("forum_auto_reply_tags", []) if str(t).strip()}
    if req_tags and not ((applied_names & req_tags) or (applied_ids & req_tags)):
        logger.info(f"Forum post '{thread.name}' skipped: does not match required tag filter.")
        return

    # Mark as handled to prevent duplicate reply
    _handled_forum_thread_ids.add(thread.id)
    if len(_handled_forum_thread_ids) > 10000:
        _handled_forum_thread_ids.clear()
        _handled_forum_thread_ids.add(thread.id)

    # Fetch starter message
    if starter_msg is None:
        starter_msg = getattr(thread, "starter_message", None)
    if starter_msg is None:
        await asyncio.sleep(0.5)
        try:
            starter_msg = await thread.fetch_message(thread.id)
        except Exception:
            pass
    if starter_msg is None:
        try:
            async for msg in thread.history(limit=1, oldest_first=True):
                starter_msg = msg
                break
        except Exception as e:
            logger.debug(f"Could not retrieve starter message for thread {thread.id}: {e}")

    if not starter_msg:
        logger.warning(f"Could not retrieve starter message for forum thread '{thread.name}' ({thread.id})")
        return

    # Do not reply to bots or self
    if starter_msg.author == bot.user or getattr(starter_msg.author, "bot", False):
        return

    # Permission check
    if not permissions.can_reply(starter_msg, config):
        logger.info(f"User {starter_msg.author} lacks permission for forum auto-reply in {thread.guild.name}")
        return

    # Build prompt
    title = (thread.name or "").strip()
    body = (starter_msg.content or "").strip()
    include_title = config.get("forum_include_post_title", True)
    if include_title and title:
        prompt_override = f"[Forum Post Title: {title}]\n{body}" if body else f"[Forum Post Title: {title}]"
    else:
        prompt_override = body if body else title

    forum_sys_prompt = (config.get("forum_system_prompt") or "").strip()

    logger.info(f"Auto-replying to forum post '{title}' ({thread.id}) in guild '{thread.guild.name}' (#{getattr(parent, 'name', parent.id)})")
    log_to_gui("forum_post_replied", {
        "guild_id": guild_id,
        "guild_name": thread.guild.name,
        "thread_id": thread.id,
        "post_title": title,
        "author": str(starter_msg.author)
    })

    await handle_ai_prompt(
        starter_msg,
        config,
        prompt_override=prompt_override,
        system_prompt_override=forum_sys_prompt if forum_sys_prompt else None,
        is_forum_auto_reply=True
    )

@bot.event
async def on_thread_create(thread: discord.Thread):
    """
    Event listener triggered when a new thread is created in Discord.
    Monitors for new posts created in Forum channels and auto-replies if enabled.
    """
    await process_forum_auto_reply(thread)

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

    # Check if message is a new forum post starter message
    if isinstance(message.channel, discord.Thread) and message.id == message.channel.id:
        parent = message.channel.parent
        if parent is None and getattr(message.channel, "parent_id", None) and message.guild:
            parent = message.guild.get_channel(message.channel.parent_id)
        is_forum = isinstance(parent, getattr(discord, "ForumChannel", ())) or getattr(parent, "type", None) == getattr(discord.ChannelType, "forum", None)
        if is_forum:
            if message.channel.id not in _handled_forum_thread_ids:
                await process_forum_auto_reply(message.channel, starter_msg=message)
            return

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

@bot.command(name="quota", aliases=["tokens", "myquota"])
async def check_user_quota(ctx):
    """Displays your token quota and remaining allowance in this server."""
    if ctx.guild is None:
        await ctx.send("Token quotas are only applicable inside Discord servers.")
        return

    config = server_configs.get(ctx.guild.id) or load_config(ctx.guild.id)
    quotas_cfg = config.get("token_quotas", {})
    if not quotas_cfg.get("enabled", False):
        await ctx.send("ℹ️ Token quotas are currently disabled on this server.")
        return

    quota = token_limiter.check_prompt_allowed(ctx.guild.id, ctx.author, config)
    if quota.limit == -1:
        await ctx.send(f"✨ You have **unlimited tokens** ({quota.role_name}).")
        return

    reset_rel = f"<t:{quota.reset_epoch}:R>" if quota.reset_epoch else "the next reset"
    reset_abs = f"<t:{quota.reset_epoch}:f>" if quota.reset_epoch else "N/A"
    freq = quotas_cfg.get("reset_frequency", "daily").title()
    overage_cfg = quotas_cfg.get("overage_policy", {})
    allow_overage = "Yes" if overage_cfg.get("allow_partial_overage", True) else "No"
    max_ov_val = overage_cfg.get('max_overage_tokens', 1000)
    max_overage = "Unlimited single turn" if max_ov_val == -1 else f"{max_ov_val:,} tokens"

    embed = discord.Embed(
        title="📊 Your Token Quota Status",
        description=f"Server: **{ctx.guild.name}**",
        color=get_discord_theme_color()
    )
    embed.add_field(name="Role Tier", value=f"@{quota.role_name}", inline=True)
    embed.add_field(name="Reset Cycle", value=freq, inline=True)
    embed.add_field(name="Tokens Used", value=f"{quota.used:,} / {quota.limit:,}", inline=True)
    embed.add_field(name="Tokens Remaining", value=f"{quota.remaining:,}", inline=True)
    embed.add_field(name="Partial Overage", value=f"{allow_overage} ({max_overage})", inline=True)
    embed.add_field(name="Next Reset", value=f"{reset_rel} ({reset_abs})", inline=False)
    await ctx.send(embed=embed)

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
    if not permissions.can_change_permissions(ctx.author, ctx.guild, config):
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

@bot.command(name="setforum", aliases=["forumreply", "forum"])
async def set_forum(ctx, action: Optional[str] = None, *, arg: Optional[str] = None):
    """
    Configures Discord Forum Auto-Reply settings for this server (Admin only).
    Usage:
      !setforum on / off             - Enable or disable automatic replies in forums
      !setforum status               - View current forum auto-reply configuration
      !setforum prompt <text>        - Set custom persona/instructions for forum post replies
      !setforum resetprompt          - Clear custom forum system prompt
      !setforum title on / off       - Toggle including post title in the prompt
    """
    if not ctx.guild:
        await ctx.send("Forum settings can only be configured within a server.")
        return

    guild_id = ctx.guild.id
    config = server_configs.get(guild_id, {})
    if not permissions.can_change_permissions(ctx.author, ctx.guild, config):
        await ctx.send("You do not have permission to modify server forum settings.")
        return

    act = (action or "").lower().strip()

    if act in ("on", "enable", "true", "1"):
        config["forum_auto_reply_enabled"] = True
        save_config(guild_id, config)
        server_configs[guild_id] = config
        await ctx.send("✅ **Forum Auto-Reply Enabled**: Silas Blue will now monitor new forum posts and reply.")
    elif act in ("off", "disable", "false", "0"):
        config["forum_auto_reply_enabled"] = False
        save_config(guild_id, config)
        server_configs[guild_id] = config
        await ctx.send("⏸️ **Forum Auto-Reply Disabled**.")
    elif act in ("title", "posttitle"):
        val = (arg or "").lower().strip()
        enable_title = val in ("on", "true", "1", "yes")
        config["forum_include_post_title"] = enable_title
        save_config(guild_id, config)
        server_configs[guild_id] = config
        await ctx.send(f"✅ Forum post title inclusion set to: **{enable_title}**.")
    elif act in ("prompt", "systemprompt", "setprompt"):
        if not arg or not arg.strip():
            await ctx.send("Please provide the custom system prompt text, e.g. `!setforum prompt You are a helpful forum assistant.`")
            return
        config["forum_system_prompt"] = arg.strip()[:4000]
        save_config(guild_id, config)
        server_configs[guild_id] = config
        await ctx.send(f"✅ **Forum System Prompt updated**:\n```{config['forum_system_prompt']}```")
    elif act in ("resetprompt", "clearprompt"):
        config["forum_system_prompt"] = ""
        save_config(guild_id, config)
        server_configs[guild_id] = config
        await ctx.send("✅ **Forum System Prompt cleared** (will use server default system prompt).")
    elif act == "status" or not act:
        enabled = config.get("forum_auto_reply_enabled", False)
        chans = config.get("forum_monitored_channels", [])
        chans_display = ", ".join(f"<#{c}>" for c in chans) if chans else "All Forum Channels"
        tags = config.get("forum_auto_reply_tags", [])
        tags_display = ", ".join(f"`{t}`" for t in tags) if tags else "All Tags (No whitelist filter)"
        ignore_tags = config.get("forum_ignore_tags", [])
        ignore_display = ", ".join(f"`{t}`" for t in ignore_tags) if ignore_tags else "None"
        include_title = config.get("forum_include_post_title", True)
        f_prompt = config.get("forum_system_prompt") or "(Uses standard server system prompt)"

        status_msg = (
            f"📋 **Discord Forum Auto-Reply Settings**\n"
            f"• **Status**: {'🟢 **Enabled**' if enabled else '🔴 **Disabled**'}\n"
            f"• **Monitored Forums**: {chans_display}\n"
            f"• **Required Tags**: {tags_display}\n"
            f"• **Ignored Tags**: {ignore_display}\n"
            f"• **Include Post Title**: {'Yes' if include_title else 'No'}\n"
            f"• **Custom Forum Prompt**: {f_prompt[:300] + ('...' if len(f_prompt) > 300 else '')}\n\n"
            f"*Use `!setforum on/off`, `!setforum prompt <text>`, or the Desktop/Web GUI to configure.*"
        )
        await ctx.send(status_msg)
    else:
        await ctx.send("Unknown action. Available: `!setforum [on|off|status|prompt <text>|resetprompt|title on/off]`")

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
    is_owner = await bot.is_owner(ctx.author)
    if not is_owner:
        await ctx.send("You must be the Bot Application Owner to change the bot's global username.")
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
    if not permissions.can_change_permissions(ctx.author, ctx.guild, config):
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
    if not permissions.can_change_permissions(ctx.author, ctx.guild, config):
        await ctx.send("You do not have permission to configure memory settings.")
        return
    if minutes < 1 or minutes > 1440:
        await ctx.send("Please provide an inactivity TTL between 1 and 1440 minutes (24 hours).")
        return
    config["memory_ttl_minutes"] = minutes
    save_config(guild_id, config)
    server_configs[guild_id] = config
    await ctx.send(f"Conversation memory inactivity TTL set to **{minutes}** minutes for this server.")

@bot.command(name="search", aliases=["web", "ddg"])
async def search_command(ctx, *, query: str = ""):
    """Searches the web and generates an AI-grounded response citing sources."""
    query = query.strip()
    if not query:
        await ctx.send("Please provide a query to search. Example: `!search latest Mars rover discoveries`")
        return
    config = server_configs.get(ctx.guild.id, {}) if ctx.guild else load_config(None)
    if not permissions.can_search(ctx.author, ctx.guild, config):
        await ctx.send("You do not have permission to use web search.")
        return
    await handle_ai_prompt(ctx.message, config, prompt_override=query, force_web_search=True)

@bot.command(name="links", aliases=["rawsearch"])
async def links_command(ctx, *, query: str = ""):
    """Fetches raw web search links and snippets without calling the AI."""
    query = query.strip()
    if not query:
        await ctx.send("Please provide a query to search. Example: `!links Python documentation`")
        return
    config = server_configs.get(ctx.guild.id, {}) if ctx.guild else load_config(None)
    if not permissions.can_search(ctx.author, ctx.guild, config):
        await ctx.send("You do not have permission to use web search.")
        return

    search_service = get_search_service()
    async with ctx.channel.typing():
        results = await search_service.search(query)

    if not results:
        await ctx.send(f"No search results found for: `{query}`")
        return

    lines = [f"🔍 **Web Search Results for:** *{query}*\n"]
    for idx, r in enumerate(results, 1):
        clean_title = r.title.replace("[", "").replace("]", "")
        lines.append(f"**{idx}. [{clean_title}](<{r.url}>)**\n> {r.snippet}\n")

    output = "\n".join(lines)
    max_chars = min(config.get("pagination_max_chars", 1900), 1900)
    pages = paginate_text(output, max_chars)
    if len(pages) == 1:
        await ctx.send(pages[0])
    else:
        view = PaginatedView(pages, ctx.author.id)
        view.message = await ctx.send(f"Page 1/{len(pages)}\n{pages[0]}", view=view)

@bot.command(name="autosearch", aliases=["setautosearch"])
async def autosearch_command(ctx, state: Optional[str] = None):
    """Toggles or sets auto-search mode for this server (Admin only)."""
    if ctx.guild is None:
        await ctx.send("This command can only be used in a server.")
        return
    guild_id = ctx.guild.id
    config = server_configs.get(guild_id, {})
    if not permissions.can_change_permissions(ctx.author, ctx.guild, config):
        await ctx.send("You do not have permission to configure search settings.")
        return

    if state is None:
        current = config.get("web_search_auto", True)
        new_state = not current
    elif state.lower() in ("on", "true", "enable", "1"):
        new_state = True
    elif state.lower() in ("off", "false", "disable", "0"):
        new_state = False
    else:
        await ctx.send("Usage: `!autosearch [on|off]`")
        return

    config["web_search_auto"] = new_state
    save_config(guild_id, config)
    server_configs[guild_id] = config
    status_str = "ENABLED" if new_state else "DISABLED"
    await ctx.send(f"🌐 Auto-search mode is now **{status_str}** for this server.")

@bot.command(name="setwebsearch", aliases=["websearch"])
async def set_web_search_command(ctx, state: Optional[str] = None):
    """Enables or disables web search features for this server (Admin only)."""
    if ctx.guild is None:
        await ctx.send("This command can only be used in a server.")
        return
    guild_id = ctx.guild.id
    config = server_configs.get(guild_id, {})
    if not permissions.can_change_permissions(ctx.author, ctx.guild, config):
        await ctx.send("You do not have permission to configure search settings.")
        return

    if state is None:
        current = config.get("web_search_enabled", True)
        new_state = not current
    elif state.lower() in ("on", "true", "enable", "1"):
        new_state = True
    elif state.lower() in ("off", "false", "disable", "0"):
        new_state = False
    else:
        await ctx.send("Usage: `!setwebsearch [on|off]`")
        return

    config["web_search_enabled"] = new_state
    save_config(guild_id, config)
    server_configs[guild_id] = config
    status_str = "ENABLED" if new_state else "DISABLED"
    await ctx.send(f"🌐 Web search is now **{status_str}** for this server.")

@bot.hybrid_command(name="update", description="Check, configure, or apply updates to Silas Blue.")
@discord.app_commands.describe(
    action="Action: 'check' (default), 'apply', 'channel', or 'status'",
    channel="Release channel: 'stable' or 'latest'"
)
async def update_command(ctx: commands.Context, action: str = "check", channel: Optional[str] = None):
    """
    Manages Silas Blue updates.
    Usage:
      !update / /update - Check for updates
      !update check [channel] - Check specific channel
      !update apply [channel] - Pull update, backup, and restart (Owner/Admin)
      !update channel <stable|latest> - Set active release channel
      !update status - Show current version and update configuration
    """
    action = action.lower().strip()
    app_cfg = load_app_config()
    current_chan = (channel or app_cfg.get("update_channel", "stable")).lower().strip()

    # Permission check for mutating actions
    is_owner = await bot.is_owner(ctx.author)
    is_admin = bool(ctx.guild and (ctx.author == ctx.guild.owner or getattr(ctx.author.guild_permissions, "administrator", False)))

    if action in ("apply", "channel") and not (is_owner or is_admin):
        await ctx.send("❌ You must be the Bot Owner or a Server Administrator to perform this action.")
        return

    # Defer interaction if slash command to avoid Discord 3-second timeout
    if ctx.interaction:
        await ctx.defer()

    if action in ("check", "status"):
        msg = await ctx.send("🔍 Checking for updates...")
        res = await asyncio.to_thread(BotUpdater.check_for_updates, current_chan)
        
        has_update = res.get("update_available", False)
        embed = discord.Embed(
            title="🔄 Silas Blue Update Status",
            color=get_discord_theme_color()
        )
        embed.add_field(name="Current Version", value=f"`{res.get('current_version')}` (`{res.get('current_commit')}`)", inline=True)
        embed.add_field(name="Latest Available", value=f"`{res.get('latest_version')}` (`{res.get('latest_commit')}`)", inline=True)
        embed.add_field(name="Release Channel", value=f"`{res.get('channel', 'stable').title()}`", inline=True)

        if has_update:
            embed.description = "🎉 **A newer version of Silas Blue is available!**\nUse `/update apply` or `!update apply` to install and restart."
            if res.get("commits_behind", 0) > 0:
                embed.add_field(name="Commits Behind", value=str(res["commits_behind"]), inline=True)
            if res.get("changelog"):
                cl = "\n".join(f"• {c}" for c in res["changelog"][:6])
                embed.add_field(name="Changelog", value=cl, inline=False)
        else:
            embed.description = "✅ **Silas Blue is completely up to date!**"

        if res.get("error"):
            embed.add_field(name="Notice", value=f"⚠️ {res['error']}", inline=False)

        await msg.edit(content=None, embed=embed)

    elif action == "channel":
        if channel not in ("stable", "latest"):
            await ctx.send("⚠️ Please specify a valid channel: `stable` (GitHub releases) or `latest` (main branch).")
            return
        app_cfg["update_channel"] = channel
        save_app_config(app_cfg)
        await ctx.send(f"✅ Active update channel switched to **{channel.title()}**.")

    elif action == "apply":
        init_embed = discord.Embed(
            title="⏳ Updating Silas Blue...",
            description=(
                f"1. Creating safety snapshot of `config/` and `state/`\n"
                f"2. Pulling updates from channel: **{current_chan.title()}**\n"
                f"3. Installing updated dependencies if needed\n"
                f"4. Restarting process..."
            ),
            color=get_discord_theme_color()
        )
        status_msg = await ctx.send(embed=init_embed)

        notify_ctx = {
            "channel_id": ctx.channel.id,
            "user_id": ctx.author.id,
            "guild_id": ctx.guild.id if ctx.guild else None
        }

        success, message = await asyncio.to_thread(
            BotUpdater.apply_update,
            channel=current_chan,
            initiator=str(ctx.author),
            notify_context=notify_ctx
        )

        if not success:
            err_embed = discord.Embed(
                title="❌ Update Failed",
                description=f"An error occurred while applying the update:\n```{message}```",
                color=0xFF0000
            )
            await status_msg.edit(embed=err_embed)
            return

        done_embed = discord.Embed(
            title="🚀 Update Applied!",
            description=(
                f"Update successfully staged.\n"
                f"**Restarting Silas Blue now...**\n"
                f"You will be notified once the bot is back online."
            ),
            color=0x00FF88
        )
        await status_msg.edit(embed=done_embed)
        await asyncio.sleep(1.5)

        # Trigger process/container restart
        BotUpdater.trigger_restart()

    else:
        await ctx.send("❓ Unknown update action. Available actions: `check`, `apply`, `channel`, `status`.")

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
        f"{prefix}update [check|apply|channel] - Check for or apply updates\n"
        f"{prefix}setmodel <name> - Set default model for this server\n"
        f"{prefix}models - List available models on AI backend\n"
        f"{prefix}search <query> - Search web and answer with live AI citations\n"
        f"{prefix}links <query> - Search web and return direct links and snippets\n"
        f"{prefix}autosearch [on|off] - Toggle auto-search for questions needing live info\n"
        f"{prefix}setwebsearch [on|off] - Enable/disable web search for this server\n"
        f"{prefix}setprompt <text> - Set custom AI persona for this server (use 'reset' for default)\n"
        f"{prefix}setforum [on|off|status|prompt] - Manage forum auto-reply monitoring (Admin only)\n"
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