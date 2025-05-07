"""
Discord bot core for Silas Blue.
Handles commands, permissions, per-server config, and Ollama integration.
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

from ollama_api import OllamaClient
from permissions import PermissionManager
from utils import load_config, save_config, get_config_path

logger = logging.getLogger("silasblue")

# Utility to get the discord_text color from the theme
_THEME_CACHE = None
_THEME_PATH = os.path.join("themes", "retrowave.json")  # You can make this configurable if needed

def get_discord_theme_color():
    global _THEME_CACHE
    if _THEME_CACHE is None:
        try:
            with open(_THEME_PATH, "r", encoding="utf-8") as f:
                theme = json.load(f)
            _THEME_CACHE = theme
        except Exception:
            _THEME_CACHE = {"discord_text": "#00f0ff"}
    color_hex = _THEME_CACHE.get("discord_text") or _THEME_CACHE.get("accent2") or "#00f0ff"
    return int(color_hex.lstrip("#"), 16)

def get_discord_token():
    """
    Loads the Discord bot token from config/bot_token.txt.
    If not found, prompts the user to enter it, then saves it for future use.
    """
    config_dir = "config"
    token_path = os.path.join(config_dir, "bot_token.txt")
    os.makedirs(config_dir, exist_ok=True)

    # Try to load from file
    if os.path.exists(token_path):
        with open(token_path, "r") as f:
            token = f.read().strip()
            if token:
                return token

    # Prompt user for token
    print("Discord bot token not found.")
    print("Please paste your Discord bot token (input is hidden):")
    token = getpass.getpass("Token: ").strip()
    if not token:
        print("No token entered. Exiting.")
        exit(1)
    # Save for future use
    with open(token_path, "w") as f:
        f.write(token)
    print("Token saved to config/bot_token.txt.")
    return token

# Use the new token loading logic
DISCORD_TOKEN = get_discord_token()
COMMAND_PREFIX = "!"
intents = discord.Intents.default()
intents.messages = True
intents.guilds = True
intents.message_content = True

bot = commands.Bot(command_prefix=COMMAND_PREFIX, intents=intents, help_command=None)
ollama = OllamaClient()
permissions = PermissionManager()

server_configs = {}

class PaginatedView(discord.ui.View):
    def __init__(self, pages, author_id, timeout=180):
        super().__init__(timeout=timeout)
        self.pages = pages
        self.author_id = author_id
        self.current_page = 0
        self.total_pages = len(pages)
        self.message = None
        self.update_buttons()

    def update_buttons(self):
        self.clear_items()
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

    async def update_message(self, interaction):
        # Update button states
        self.prev_button.disabled = self.current_page == 0
        self.next_button.disabled = self.current_page == self.total_pages - 1
        content = f"Page {self.current_page+1}/{self.total_pages}\n{self.pages[self.current_page]}"
        await interaction.response.edit_message(content=content, view=self)

    async def on_timeout(self):
        if self.message:
            await self.message.edit(view=None)

@bot.event
async def on_ready():
    logging.info(f"Silas Blue is online as {bot.user} (ID: {bot.user.id})")
    logger.info("Bot started and ready.")
    # Set status to always show the new URL as Playing
    await bot.change_presence(activity=discord.Game(name="git.new/silasblue"))
    # Load configs for all guilds
    for guild in bot.guilds:
        config = load_config(guild.id)
        server_configs[guild.id] = config
    logging.info("Loaded all server configs.")

@bot.event
async def on_guild_join(guild):
    logging.info(f"Joined new guild: {guild.name} (ID: {guild.id})")
    logger.info(f"Joined server: {guild.name} ({guild.id})")
    config = load_config(guild.id)
    server_configs[guild.id] = config

@bot.event
async def on_guild_remove(guild):
    logger.info(f"Left server: {guild.name} ({guild.id})")

@bot.event
async def on_message(message):
    if message.author == bot.user:
        return

    guild_id = message.guild.id if message.guild else None
    config = server_configs.get(guild_id, {})

    # Permission: Should the bot reply to this user/message?
    if not permissions.can_reply(message, config):
        return

    # Random prompt probability
    prob = config.get("random_prompt_probability", 0)
    import random
    if prob > 0 and random.randint(1, 100) <= prob:
        await handle_ollama_prompt(message, config, ollama)
        return

    # Check if message starts with prefix or bot mention
    is_command = False
    content = message.content
    mention_str_1 = f"<@{bot.user.id}>"
    mention_str_2 = f"<@!{bot.user.id}>"

    if content.startswith(COMMAND_PREFIX):
        is_command = True
    elif content.startswith(mention_str_1) or content.startswith(mention_str_2):
        # Remove the mention and any leading whitespace
        after_mention = content[len(mention_str_1):] if content.startswith(mention_str_1) else content[len(mention_str_2):]
        after_mention = after_mention.lstrip()
        # Get the first word after the mention
        first_word = after_mention.split(' ', 1)[0].lower() if after_mention else ""
        # List of command names (lowercase)
        command_names = [cmd.name for cmd in bot.commands]
        if first_word in command_names:
            is_command = True
            # Reconstruct the message as if it started with the prefix for command processing
            message.content = COMMAND_PREFIX + after_mention

    if is_command:
        await bot.process_commands(message)
    else:
        # If the message starts with a mention, treat the rest as a prompt
        if content.startswith(mention_str_1) or content.startswith(mention_str_2):
            after_mention = content[len(mention_str_1):] if content.startswith(mention_str_1) else content[len(mention_str_2):]
            after_mention = after_mention.lstrip()
            if after_mention:  # Only send prompt if there's text after the mention
                await handle_ollama_prompt(message, config, ollama)
        # Optionally: treat any message that mentions the bot anywhere as a prompt
        elif bot.user in message.mentions:
            await handle_ollama_prompt(message, config, ollama)

def paginate_text(text, max_chars):
    """
    Splits text into pages of up to max_chars, preserving line breaks. If a single line is too long, it is split as needed.
    """
    lines = text.splitlines(keepends=True)
    pages = []
    current = ''
    for line in lines:
        while len(line) > max_chars:
            # Find a split point (preferably at a space)
            split_at = line.rfind(' ', 0, max_chars)
            if split_at == -1 or split_at < max_chars // 2:
                split_at = max_chars  # No good space, just hard split
            part = line[:split_at]
            if not part.endswith('\n'):
                part += '\n'
            if len(current) + len(part) > max_chars:
                if current:
                    pages.append(current)
                    current = ''
            current += part
            line = line[split_at:]
        if len(current) + len(line) > max_chars:
            if current:
                pages.append(current)
                current = ''
        current += line
    if current:
        pages.append(current)
    return pages

def log_to_gui(event_type, data):
    """
    Appends a log entry for the GUI.
    event_type: 'config_change', 'prompt', 'reply'
    data: dict with relevant info
    """
    log_path = os.path.join("config", "gui_log.txt")
    os.makedirs("config", exist_ok=True)
    entry = {
        "event": event_type,
        "data": data,
        "timestamp": time.time()
    }
    with open(log_path, "a", encoding="utf-8") as f:
        f.write(json.dumps(entry) + "\n")

async def handle_ollama_prompt(message, config, ollama):
    """
    Sends a message to Ollama and replies with the result.
    """
    prompt = message.content
    model = config.get("default_model", "llama2")
    log_to_gui("prompt", {
        "guild_id": message.guild.id if message.guild else None,
        "user": str(message.author),
        "prompt": prompt
    })
    response = ollama.send_prompt(prompt, model)
    log_to_gui("reply", {
        "guild_id": message.guild.id if message.guild else None,
        "user": str(message.author),
        "reply": response
    })
    max_chars = config.get("pagination_max_chars", 2000)
    pages = paginate_text(response, max_chars)
    if len(pages) == 1:
        await message.channel.send(pages[0])
    else:
        view = PaginatedView(pages, message.author.id)
        view.message = await message.channel.send(f"Page 1/{len(pages)}\n{pages[0]}", view=view)

@bot.command(name="ping")
async def ping(ctx):
    await ctx.send("Pong!")

@bot.command(name="setmodel")
async def set_model(ctx, model_name: str):
    """
    Set the default Ollama model for this server.
    """
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
    logger.info(f"Default model for server {ctx.guild.name} ({ctx.guild.id}) set to {model_name} by {ctx.author}")
    await ctx.send(f"Default model set to `{model_name}`.")

@bot.command(name="config")
async def show_config(ctx):
    """
    Show the current server config (raw).
    """
    guild_id = ctx.guild.id
    config = server_configs.get(guild_id, {})
    config_lines = json.dumps(config, indent=2).splitlines()
    config_str = '\n'.join(config_lines)
    await ctx.send(f"Current Server Config:\n{config_str}")

@bot.command(name="models")
async def models_command(ctx):
    """
    List all available Ollama models and show the current selected model.
    """
    guild_id = ctx.guild.id
    config = server_configs.get(guild_id, {})
    current_model = config.get("default_model", "llama2")
    try:
        models = ollama.list_models()
    except Exception as e:
        await ctx.send(f"Error fetching models: {e}")
        return
    if not models:
        await ctx.send("No models found on Ollama server.")
        return
    msg = "Available Models:\n"
    for m in models:
        if m == current_model:
            msg += f"- {m} (selected)\n"
        else:
            msg += f"- {m}\n"
    await ctx.send(msg)

@bot.command(name="setpagination")
async def set_pagination(ctx, max_chars: int):
    """
    Set the pagination character limit for this server.
    """
    guild_id = ctx.guild.id
    config = server_configs.get(guild_id, {})
    if not ctx.author.guild_permissions.administrator:
        await ctx.send("You do not have permission to change pagination settings.")
        return
    if max_chars < 500 or max_chars > 2000:
        await ctx.send("Please choose a value between 500 and 2000.")
        return
    config["pagination_max_chars"] = max_chars
    save_config(guild_id, config)
    server_configs[guild_id] = config
    log_to_gui("config_change", {
        "guild_id": guild_id,
        "user": str(ctx.author),
        "field": "pagination_max_chars",
        "value": max_chars
    })
    await ctx.send(f"Pagination character limit set to {max_chars}.")

@bot.command(name="help")
async def help_command(ctx):
    guild_id = ctx.guild.id
    config = server_configs.get(guild_id, {})
    max_chars = config.get("pagination_max_chars", 2000)
    prefix = COMMAND_PREFIX
    help_text = (
        f"Silas Blue Bot Help\n"
        f"\n"
        f"General Usage:\n"
        f"Send a prompt by mentioning the bot or using the prefix {prefix} followed by your message.\n"
        f"Example: {prefix}What is the weather today? or @{bot.user.name} What is the weather today?\n"
        f"\n"
        f"Commands:\n"
        f"\n"
        f"{prefix}ping\n"
        f"Check if the bot is online. Replies with 'Pong!'.\n"
        f"\n"
        f"{prefix}setmodel <model_name>\n"
        f"Set the default Ollama model for this server.\n"
        f"Example: {prefix}setmodel llama2\n"
        f"\n"
        f"{prefix}models\n"
        f"List all available Ollama models and show which is currently selected.\n"
        f"\n"
        f"{prefix}config\n"
        f"Show the current server configuration in raw JSON.\n"
        f"\n"
        f"{prefix}help\n"
        f"Show this help message.\n"
        f"\n"
        f"Notes:\n"
        f"The bot will reply to prompts if you have permission and the server config allows it.\n"
        f"Pagination is used for long responses, with a max of {max_chars} characters per message.\n"
    )
    # Add 8 for the code block markers (```)
    codeblock_overhead = 8
    pages = paginate_text(help_text, max_chars - codeblock_overhead)
    if len(pages) == 1:
        await ctx.send(f"```\n{pages[0]}\n```")
    else:
        view = PaginatedView([f"```\n{p}\n```" for p in pages], ctx.author.id)
        view.message = await ctx.send(f"Page 1/{len(pages)}\n```\n{pages[0]}\n```", view=view)

# --- Bot control logic ---
_bot_thread = None
_bot_loop = None
_shutdown_event = None
_bot_instance = None

def create_bot():
    intents = discord.Intents.default()
    intents.messages = True
    intents.guilds = True
    intents.message_content = True

    bot = commands.Bot(command_prefix=COMMAND_PREFIX, intents=intents, help_command=None)
    ollama = OllamaClient()
    permissions = PermissionManager()
    server_configs = {}

    @bot.event
    async def on_ready():
        logging.info(f"Silas Blue is online as {bot.user} (ID: {bot.user.id})")
        logger.info("Bot started and ready.")
        # Set status to always show the new URL as Playing
        await bot.change_presence(activity=discord.Game(name="git.new/silasblue"))
        # Load configs for all guilds
        for guild in bot.guilds:
            config = load_config(guild.id)
            server_configs[guild.id] = config
        logging.info("Loaded all server configs.")

    @bot.event
    async def on_guild_join(guild):
        logging.info(f"Joined new guild: {guild.name} (ID: {guild.id})")
        logger.info(f"Joined server: {guild.name} ({guild.id})")
        config = load_config(guild.id)
        server_configs[guild.id] = config

    @bot.event
    async def on_guild_remove(guild):
        logger.info(f"Left server: {guild.name} ({guild.id})")

    @bot.event
    async def on_message(message):
        if message.author == bot.user:
            return

        guild_id = message.guild.id if message.guild else None
        config = server_configs.get(guild_id, {})

        # Permission: Should the bot reply to this user/message?
        if not permissions.can_reply(message, config):
            return

        # Random prompt probability
        prob = config.get("random_prompt_probability", 0)
        import random
        if prob > 0 and random.randint(1, 100) <= prob:
            await handle_ollama_prompt(message, config, ollama)
            return

        # Check if message starts with prefix or bot mention
        is_command = False
        content = message.content
        mention_str_1 = f"<@{bot.user.id}>"
        mention_str_2 = f"<@!{bot.user.id}>"

        if content.startswith(COMMAND_PREFIX):
            is_command = True
        elif content.startswith(mention_str_1) or content.startswith(mention_str_2):
            # Remove the mention and any leading whitespace
            after_mention = content[len(mention_str_1):] if content.startswith(mention_str_1) else content[len(mention_str_2):]
            after_mention = after_mention.lstrip()
            # Get the first word after the mention
            first_word = after_mention.split(' ', 1)[0].lower() if after_mention else ""
            # List of command names (lowercase)
            command_names = [cmd.name for cmd in bot.commands]
            if first_word in command_names:
                is_command = True
                # Reconstruct the message as if it started with the prefix for command processing
                message.content = COMMAND_PREFIX + after_mention

        if is_command:
            await bot.process_commands(message)
        else:
            # If the message starts with a mention, treat the rest as a prompt
            if content.startswith(mention_str_1) or content.startswith(mention_str_2):
                after_mention = content[len(mention_str_1):] if content.startswith(mention_str_1) else content[len(mention_str_2):]
                after_mention = after_mention.lstrip()
                if after_mention:  # Only send prompt if there's text after the mention
                    await handle_ollama_prompt(message, config, ollama)
            # Optionally: treat any message that mentions the bot anywhere as a prompt
            elif bot.user in message.mentions:
                await handle_ollama_prompt(message, config, ollama)

    @bot.command(name="ping")
    async def ping(ctx):
        await ctx.send("Pong!")

    @bot.command(name="setmodel")
    async def set_model(ctx, model_name: str):
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
        logger.info(f"Default model for server {ctx.guild.name} ({ctx.guild.id}) set to {model_name} by {ctx.author}")
        await ctx.send(f"Default model set to `{model_name}`.")

    @bot.command(name="config")
    async def show_config(ctx):
        guild_id = ctx.guild.id
        config = server_configs.get(guild_id, {})
        config_lines = json.dumps(config, indent=2).splitlines()
        config_str = '\n'.join(config_lines)
        await ctx.send(f"Current Server Config:\n{config_str}")

    @bot.command(name="models")
    async def models_command(ctx):
        guild_id = ctx.guild.id
        config = server_configs.get(guild_id, {})
        current_model = config.get("default_model", "llama2")
        try:
            models = ollama.list_models()
        except Exception as e:
            await ctx.send(f"Error fetching models: {e}")
            return
        if not models:
            await ctx.send("No models found on Ollama server.")
            return
        msg = "Available Models:\n"
        for m in models:
            if m == current_model:
                msg += f"- {m} (selected)\n"
            else:
                msg += f"- {m}\n"
        await ctx.send(msg)

    @bot.command(name="setpagination")
    async def set_pagination(ctx, max_chars: int):
        guild_id = ctx.guild.id
        config = server_configs.get(guild_id, {})
        if not ctx.author.guild_permissions.administrator:
            await ctx.send("You do not have permission to change pagination settings.")
            return
        if max_chars < 500 or max_chars > 2000:
            await ctx.send("Please choose a value between 500 and 2000.")
            return
        config["pagination_max_chars"] = max_chars
        save_config(guild_id, config)
        server_configs[guild_id] = config
        log_to_gui("config_change", {
            "guild_id": guild_id,
            "user": str(ctx.author),
            "field": "pagination_max_chars",
            "value": max_chars
        })
        await ctx.send(f"Pagination character limit set to {max_chars}.")

    @bot.command(name="help")
    async def help_command(ctx):
        guild_id = ctx.guild.id
        config = server_configs.get(guild_id, {})
        max_chars = config.get("pagination_max_chars", 2000)
        prefix = COMMAND_PREFIX
        help_text = (
            f"Silas Blue Bot Help\n"
            f"\n"
            f"General Usage:\n"
            f"Send a prompt by mentioning the bot or using the prefix {prefix} followed by your message.\n"
            f"Example: {prefix}What is the weather today? or @{bot.user.name} What is the weather today?\n"
            f"\n"
            f"Commands:\n"
            f"\n"
            f"{prefix}ping\n"
            f"Check if the bot is online. Replies with 'Pong!'.\n"
            f"\n"
            f"{prefix}setmodel <model_name>\n"
            f"Set the default Ollama model for this server.\n"
            f"Example: {prefix}setmodel llama2\n"
            f"\n"
            f"{prefix}models\n"
            f"List all available Ollama models and show which is currently selected.\n"
            f"\n"
            f"{prefix}config\n"
            f"Show the current server configuration in raw JSON.\n"
            f"\n"
            f"{prefix}help\n"
            f"Show this help message.\n"
            f"\n"
            f"Notes:\n"
            f"The bot will reply to prompts if you have permission and the server config allows it.\n"
            f"Pagination is used for long responses, with a max of {max_chars} characters per message.\n"
        )
        # Add 8 for the code block markers (```)
        codeblock_overhead = 8
        pages = paginate_text(help_text, max_chars - codeblock_overhead)
        if len(pages) == 1:
            await ctx.send(f"```\n{pages[0]}\n```")
        else:
            view = PaginatedView([f"```\n{p}\n```" for p in pages], ctx.author.id)
            view.message = await ctx.send(f"Page 1/{len(pages)}\n```\n{pages[0]}\n```", view=view)

    return bot

def start_bot():
    global _bot_thread, _bot_loop, _shutdown_event, _bot_instance
    if _bot_thread and _bot_thread.is_alive():
        logger.info("Bot is already running.")
        return
    _shutdown_event = threading.Event()
    def run():
        import asyncio
        global _bot_loop, _bot_instance
        _bot_loop = asyncio.new_event_loop()
        asyncio.set_event_loop(_bot_loop)
        _bot_instance = create_bot()
        try:
            _bot_loop.run_until_complete(_run_bot(_shutdown_event, _bot_instance))
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
        _bot_thread.join(timeout=10)  # Wait for the thread to finish
    logger.info("Bot shutdown requested.")

def restart_bot():
    stop_bot()
    time.sleep(1)  # Give time for shutdown
    start_bot()
    logger.info("Bot restart requested.")

async def _run_bot(shutdown_event, bot_instance):
    import asyncio
    shutdown_asyncio_event = asyncio.Event()
    def set_shutdown():
        shutdown_asyncio_event.set()
    # Link threading event to asyncio event
    def check_shutdown():
        if shutdown_event.is_set():
            set_shutdown()
        else:
            _bot_loop.call_later(0.5, check_shutdown)
    _bot_loop.call_soon(check_shutdown)
    bot_task = asyncio.create_task(bot_instance.start(DISCORD_TOKEN))
    await shutdown_asyncio_event.wait()
    await bot_instance.close()
    try:
        await bot_task
    except Exception:
        pass

# For compatibility with SilasBlue.py

def run_discord_bot():
    start_bot() 