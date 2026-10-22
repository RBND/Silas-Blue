import unittest
from bot_core import paginate_text

class TestPagination(unittest.TestCase):
    def test_short_text(self):
        text = "Hello world!"
        pages = paginate_text(text, 1900)
        self.assertEqual(len(pages), 1)
        self.assertEqual(pages[0], "Hello world!")

    def test_long_text_split(self):
        # 5000 characters of text
        text = "This is a word " * 400
        pages = paginate_text(text, 1850)
        self.assertGreater(len(pages), 1)
        for page in pages:
            self.assertLessEqual(len(page), 1850)
            # Verify combined length with Discord header is strictly < 2000
            header = f"Page 99/99\n{page}"
            self.assertLessEqual(len(header), 2000)

    def test_unbroken_long_line(self):
        # Single continuous line of 4000 characters without spaces
        text = "A" * 4000
        pages = paginate_text(text, 1850)
        self.assertGreater(len(pages), 1)
        for page in pages:
            self.assertLessEqual(len(page), 1850)


class TestNonPaginatedFollowUpDelivery(unittest.IsolatedAsyncioTestCase):
    async def test_pagination_disabled_sends_followup_messages_with_cap(self):
        from unittest.mock import AsyncMock, MagicMock, patch
        import bot_core

        mock_msg = MagicMock()
        mock_msg.channel.send = AsyncMock()
        mock_msg.author.id = 123456
        mock_msg.guild.id = 7777
        mock_msg.guild.name = "Test Guild"
        mock_msg.content = "Explain quantum computing in detail"
        mock_msg.reference = None

        long_reply = ("Paragraph line of text for testing follow-up messages.\n") * 80  # ~4400 chars
        mock_provider = MagicMock()
        mock_provider.send_prompt = AsyncMock(return_value=long_reply)
        mock_provider.last_usage = {"prompt_tokens": 10, "completion_tokens": 100, "total_tokens": 110}

        cfg = {
            "default_model": "test-model",
            "pagination_enabled": False,
            "non_paginated_max_chars": 2500,
            "web_search_enabled": False,
            "memory_turns": 0
        }

        with patch("bot_core.get_provider", return_value=mock_provider), \
             patch("bot_core.log_to_gui"), \
             patch("asyncio.sleep", new_callable=AsyncMock):
            await bot_core.handle_ai_prompt(mock_msg, cfg)

        # With 2500 char cap split into <=1900 char follow-up messages, should send 2 messages without view
        self.assertEqual(mock_msg.channel.send.await_count, 2)
        sent_total = sum(len(call.args[0]) for call in mock_msg.channel.send.await_args_list)
        self.assertLessEqual(sent_total, 2500)
        for call in mock_msg.channel.send.await_args_list:
            self.assertNotIn("view", call.kwargs)


if __name__ == "__main__":
    unittest.main()
