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
        self.assertTrue(kwargs.get("is_forum_auto_reply", False))

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

    @patch("bot_core.handle_ai_prompt", new_callable=AsyncMock)
    async def test_forum_auto_reply_via_starter_message_event(self, mock_handle_ai):
        thread, starter_msg = self._create_mock_thread(parent_is_forum=True)
        config = {
            "forum_auto_reply_enabled": True,
            "forum_include_post_title": True,
            "forum_system_prompt": "Diagnostic bot persona"
        }
        with patch.dict(bot_core.server_configs, {self.guild_id: config}):
            with patch("bot_core.permissions.can_reply", return_value=True):
                # When on_message receives the starter message first
                await bot_core.on_message(starter_msg)
                mock_handle_ai.assert_called_once()

                # And when on_thread_create fires subsequently, it avoids double-reply
                mock_handle_ai.reset_mock()
                await bot_core.on_thread_create(thread)
                mock_handle_ai.assert_not_called()

    async def test_forum_auto_reply_pagination_delivery_modes(self):
        # 1. Verify PaginatedView allow_any_user & timeout parameters
        view = bot_core.PaginatedView(["Page 1", "Page 2"], author_id=123, timeout=None, allow_any_user=True)
        self.assertIsNone(view.timeout)
        self.assertTrue(view.allow_any_user)

        # Another user can interact when allow_any_user is True
        interaction = MagicMock()
        interaction.user.id = 999  # Different user
        interaction.response.edit_message = AsyncMock()
        await view.next_button.callback(interaction)
        interaction.response.edit_message.assert_awaited()

        # 2. Test handle_ai_prompt with is_forum_auto_reply and all_chunks
        mock_msg = MagicMock()
        mock_msg.channel.send = AsyncMock()
        mock_msg.author.id = 123456
        mock_msg.guild.id = 5555
        mock_msg.guild.name = "Test Guild"
        mock_msg.content = "How to fix iPhone charging port?"

        mock_provider = MagicMock()
        mock_provider.list_models = AsyncMock(return_value=["test-model"])
        mock_provider.generate_response = AsyncMock(return_value="Part 1 content and Part 2 content")

        cfg_all_chunks = {
            "forum_delivery_mode": "all_chunks",
            "pagination_max_chars": 1200,
            "web_search_enabled": False,
        }

        with patch("bot_core.get_provider", return_value=mock_provider):
            with patch("bot_core.conversation_memory.add_turn"):
                with patch("bot_core.log_to_gui"):
                    with patch("bot_core.paginate_text", return_value=["Part 1 content", "Part 2 content"]):
                        await bot_core.handle_ai_prompt(
                            mock_msg,
                            config=cfg_all_chunks,
                            is_forum_auto_reply=True
                        )
                        
                        self.assertEqual(mock_msg.channel.send.await_count, 2)
                        calls = mock_msg.channel.send.await_args_list
                        self.assertIn("**[Part 1/2]**", calls[0].args[0])
                        self.assertIn("**[Part 2/2]**", calls[1].args[0])

        # 3. Test handle_ai_prompt with is_forum_auto_reply and interactive
        mock_msg.channel.send.reset_mock()
        cfg_interactive = {
            "forum_delivery_mode": "interactive",
            "pagination_max_chars": 1200,
            "web_search_enabled": False,
        }

        with patch("bot_core.get_provider", return_value=mock_provider):
            with patch("bot_core.conversation_memory.add_turn"):
                with patch("bot_core.log_to_gui"):
                    with patch("bot_core.paginate_text", return_value=["Part 1 content", "Part 2 content"]):
                        with patch("bot_core.PaginatedView") as mock_view_cls:
                            await bot_core.handle_ai_prompt(
                                mock_msg,
                                config=cfg_interactive,
                                is_forum_auto_reply=True
                            )
                            mock_view_cls.assert_called_once_with(
                                ["Part 1 content", "Part 2 content"],
                                mock_msg.author.id,
                                timeout=None,
                                allow_any_user=True
                            )

if __name__ == "__main__":
    unittest.main()
