import unittest
from unittest.mock import AsyncMock, MagicMock, patch
import discord
from utils import load_config, save_config, validate_and_sanitize_guild_config
import bot_core

class TestForumAutoReply(unittest.IsolatedAsyncioTestCase):
    def setUp(self):
        bot_core._handled_forum_thread_ids.clear()
        self.guild_id = 987654321012345678
        self.forum_channel_id = 111222333444555666
        self.thread_id = 222333444555666777

    def _create_mock_thread(self, parent_is_forum=True, applied_tags=None, author_is_bot=False):
        thread = MagicMock(spec=discord.Thread)
        thread.id = self.thread_id
        thread.name = "How do I fix Docker containers?"
        thread.guild = MagicMock()
        thread.guild.id = self.guild_id
        thread.guild.name = "Test Guild"

        if parent_is_forum:
            parent = MagicMock(spec=discord.ForumChannel)
            parent.id = self.forum_channel_id
            parent.type = discord.ChannelType.forum
            thread.parent = parent
            thread.parent_id = self.forum_channel_id
        else:
            parent = MagicMock(spec=discord.TextChannel)
            parent.id = self.forum_channel_id
            parent.type = discord.ChannelType.text
            thread.parent = parent
            thread.parent_id = self.forum_channel_id

        # Tags
        tags = []
        if applied_tags:
            for tid, tname in applied_tags:
                tag_mock = MagicMock()
                tag_mock.id = tid
                tag_mock.name = tname
                tags.append(tag_mock)
        thread.applied_tags = tags

        # Starter message
        starter_msg = MagicMock(spec=discord.Message)
        starter_msg.id = self.thread_id
        starter_msg.content = "My container exits immediately after running."
        starter_msg.author = MagicMock()
        starter_msg.author.id = 333444555666777888
        starter_msg.author.bot = author_is_bot
        starter_msg.guild = thread.guild
        starter_msg.channel = thread
        thread.starter_message = starter_msg
        thread.fetch_message = AsyncMock(return_value=starter_msg)

        return thread, starter_msg

    @patch("bot_core.handle_ai_prompt", new_callable=AsyncMock)
    async def test_forum_auto_reply_disabled_by_default(self, mock_handle_ai):
        thread, _ = self._create_mock_thread()
        config = {
            "forum_auto_reply_enabled": False
        }
        with patch.dict(bot_core.server_configs, {self.guild_id: config}):
            await bot_core.on_thread_create(thread)

        mock_handle_ai.assert_not_called()

    @patch("bot_core.handle_ai_prompt", new_callable=AsyncMock)
    async def test_forum_auto_reply_ignores_regular_threads(self, mock_handle_ai):
        thread, _ = self._create_mock_thread(parent_is_forum=False)
        config = {
            "forum_auto_reply_enabled": True
        }
        with patch.dict(bot_core.server_configs, {self.guild_id: config}):
            await bot_core.on_thread_create(thread)

        mock_handle_ai.assert_not_called()

    @patch("bot_core.handle_ai_prompt", new_callable=AsyncMock)
    async def test_forum_auto_reply_fires_on_enabled_forum(self, mock_handle_ai):
        thread, starter_msg = self._create_mock_thread(parent_is_forum=True)
        config = {
            "forum_auto_reply_enabled": True,
            "forum_include_post_title": True,
            "forum_system_prompt": "You are a tech support AI assistant."
        }
        with patch.dict(bot_core.server_configs, {self.guild_id: config}):
            with patch("bot_core.permissions.can_reply", return_value=True):
                await bot_core.on_thread_create(thread)

        mock_handle_ai.assert_called_once()
        args, kwargs = mock_handle_ai.call_args
        self.assertEqual(args[0], starter_msg)
        self.assertIn("[Forum Post Title: How do I fix Docker containers?]", kwargs["prompt_override"])
        self.assertIn("My container exits immediately after running.", kwargs["prompt_override"])
        self.assertEqual(kwargs["system_prompt_override"], "You are a tech support AI assistant.")

    @patch("bot_core.handle_ai_prompt", new_callable=AsyncMock)
    async def test_channel_whitelist_filtering(self, mock_handle_ai):
        thread, _ = self._create_mock_thread(parent_is_forum=True)
        # Configure whitelist for another channel
        config = {
            "forum_auto_reply_enabled": True,
            "forum_monitored_channels": ["999888777666555444"]
        }
        with patch.dict(bot_core.server_configs, {self.guild_id: config}):
            await bot_core.on_thread_create(thread)
        mock_handle_ai.assert_not_called()

        # Now configure whitelist matching the forum channel
        config["forum_monitored_channels"] = [str(self.forum_channel_id)]
        with patch.dict(bot_core.server_configs, {self.guild_id: config}):
            with patch("bot_core.permissions.can_reply", return_value=True):
                await bot_core.on_thread_create(thread)
        mock_handle_ai.assert_called_once()

    @patch("bot_core.handle_ai_prompt", new_callable=AsyncMock)
    async def test_tag_filtering_whitelist_and_blacklist(self, mock_handle_ai):
        # 1. Blacklist test
        thread_ignored, _ = self._create_mock_thread(
            parent_is_forum=True,
            applied_tags=[(1, "Resolved"), (2, "General")]
        )
        config = {
            "forum_auto_reply_enabled": True,
            "forum_ignore_tags": ["resolved"]
        }
        with patch.dict(bot_core.server_configs, {self.guild_id: config}):
            await bot_core.on_thread_create(thread_ignored)
        mock_handle_ai.assert_not_called()

        # 2. Whitelist test: post does NOT have required tag
        thread_no_req, _ = self._create_mock_thread(
            parent_is_forum=True,
            applied_tags=[(2, "General")]
        )
        thread_no_req.id = 555666777888999000
        config_whitelist = {
            "forum_auto_reply_enabled": True,
            "forum_auto_reply_tags": ["help", "support"]
        }
        with patch.dict(bot_core.server_configs, {self.guild_id: config_whitelist}):
            await bot_core.on_thread_create(thread_no_req)
        mock_handle_ai.assert_not_called()

        # 3. Whitelist test: post HAS required tag
        thread_with_req, _ = self._create_mock_thread(
            parent_is_forum=True,
            applied_tags=[(3, "Help")]
        )
        thread_with_req.id = 666777888999000111
        with patch.dict(bot_core.server_configs, {self.guild_id: config_whitelist}):
            with patch("bot_core.permissions.can_reply", return_value=True):
                await bot_core.on_thread_create(thread_with_req)
        mock_handle_ai.assert_called_once()

    @patch("bot_core.handle_ai_prompt", new_callable=AsyncMock)
    async def test_deduplication_prevents_duplicate_replies(self, mock_handle_ai):
        thread, _ = self._create_mock_thread(parent_is_forum=True)
        config = {
            "forum_auto_reply_enabled": True
        }
        with patch.dict(bot_core.server_configs, {self.guild_id: config}):
            with patch("bot_core.permissions.can_reply", return_value=True):
                # First time: called
                await bot_core.on_thread_create(thread)
                # Second time: should be blocked by de-duplication
                await bot_core.on_thread_create(thread)

        self.assertEqual(mock_handle_ai.call_count, 1)

    @patch("bot_core.handle_ai_prompt", new_callable=AsyncMock)
    async def test_ignores_bot_author(self, mock_handle_ai):
        thread, _ = self._create_mock_thread(parent_is_forum=True, author_is_bot=True)
        config = {
            "forum_auto_reply_enabled": True
        }
        with patch.dict(bot_core.server_configs, {self.guild_id: config}):
            await bot_core.on_thread_create(thread)

        mock_handle_ai.assert_not_called()

    async def test_set_forum_command(self):
        ctx = MagicMock()
        ctx.guild = MagicMock()
        ctx.guild.id = self.guild_id
        ctx.guild.name = "Test Guild"
        ctx.author = MagicMock()
        ctx.send = AsyncMock()

        config = {
            "forum_auto_reply_enabled": False,
            "forum_include_post_title": True,
            "forum_system_prompt": ""
        }

        with patch.dict(bot_core.server_configs, {self.guild_id: config}):
            with patch("bot_core.permissions.can_change_permissions", return_value=True):
                with patch("bot_core.save_config"):
                    # 1. Enable
                    await bot_core.set_forum(ctx, action="on")
                    self.assertTrue(config["forum_auto_reply_enabled"])
                    ctx.send.assert_awaited()

                    # 2. Set Prompt
                    await bot_core.set_forum(ctx, action="prompt", arg="Custom forum assistant prompt")
                    self.assertEqual(config["forum_system_prompt"], "Custom forum assistant prompt")

                    # 3. Status check
                    await bot_core.set_forum(ctx, action="status")
                    last_msg = ctx.send.call_args[0][0]
                    self.assertIn("Discord Forum Auto-Reply Settings", last_msg)
                    self.assertIn("Enabled", last_msg)

                    # 4. Disable
                    await bot_core.set_forum(ctx, action="off")
                    self.assertFalse(config["forum_auto_reply_enabled"])

if __name__ == "__main__":
    unittest.main()
