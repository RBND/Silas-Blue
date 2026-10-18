import unittest
import time
from memory import ConversationMemory

class TestConversationMemory(unittest.TestCase):
    def setUp(self):
        self.mem = ConversationMemory(default_ttl_minutes=30, max_assistant_chars=100)

    def test_dm_isolation(self):
        """Verify DM memory is strictly per-user and never cross-pollinates."""
        dm_user_1 = ConversationMemory.get_dm_key(1001)
        dm_user_2 = ConversationMemory.get_dm_key(2002)

        self.mem.add_turn(dm_user_1, "User 1 secret message", "Assistant reply to user 1")
        self.mem.add_turn(dm_user_2, "User 2 public question", "Assistant reply to user 2")

        hist_1 = self.mem.get_history(dm_user_1)
        hist_2 = self.mem.get_history(dm_user_2)

        self.assertEqual(len(hist_1), 2)
        self.assertEqual(len(hist_2), 2)
        self.assertEqual(hist_1[0]["content"], "User 1 secret message")
        self.assertEqual(hist_2[0]["content"], "User 2 public question")
        self.assertNotIn("User 1 secret message", [m["content"] for m in hist_2])
        self.assertNotIn("User 2 public question", [m["content"] for m in hist_1])

    def test_channel_isolation(self):
        """Verify channel memory is strictly per-channel."""
        ch_1 = ConversationMemory.get_channel_key(100, 1)
        ch_2 = ConversationMemory.get_channel_key(100, 2)

        self.mem.add_turn(ch_1, "Channel 1 chat", "Reply 1")
        self.mem.add_turn(ch_2, "Channel 2 chat", "Reply 2")

        hist_1 = self.mem.get_history(ch_1)
        hist_2 = self.mem.get_history(ch_2)

        self.assertEqual(hist_1[0]["content"], "Channel 1 chat")
        self.assertEqual(hist_2[0]["content"], "Channel 2 chat")

    def test_smart_compression(self):
        """Verify verbose assistant responses are truncated to save token context."""
        key = "test_compression"
        long_response = "A" * 300
        self.mem.add_turn(key, "User prompt", long_response)

        hist = self.mem.get_history(key)
        self.assertEqual(len(hist), 2)
        assistant_turn = hist[1]["content"]
        self.assertTrue(assistant_turn.endswith("... [truncated for brevity]"))
        self.assertLessEqual(len(assistant_turn), 150)

    def test_turns_sliding_window(self):
        """Verify buffer trims to requested turn limits."""
        key = "test_window"
        for i in range(10):
            self.mem.add_turn(key, f"Prompt {i}", f"Reply {i}", max_turns=4)

        hist = self.mem.get_history(key, max_turns=4)
        # 4 messages = 2 turns (user + assistant)
        self.assertEqual(len(hist), 4)
        self.assertEqual(hist[-2]["content"], "Prompt 9")
        self.assertEqual(hist[-1]["content"], "Reply 9")

    def test_inactivity_ttl_expiration(self):
        """Verify memory automatically expires after configured TTL."""
        key = "test_ttl"
        self.mem.add_turn(key, "Hello", "Hi there")

        # Manually backdate last_active to simulate 31 minutes ago
        with self.mem._lock:
            self.mem._buffers[key]["last_active"] = time.time() - (31 * 60)

        hist = self.mem.get_history(key, max_turns=6, ttl_minutes=30)
        self.assertEqual(hist, [])

    def test_manual_clear(self):
        """Verify clearing one buffer leaves others intact."""
        k1 = "session_1"
        k2 = "session_2"
        self.mem.add_turn(k1, "p1", "r1")
        self.mem.add_turn(k2, "p2", "r2")

        self.assertTrue(self.mem.clear(k1))
        self.assertEqual(self.mem.get_history(k1), [])
        self.assertEqual(len(self.mem.get_history(k2)), 2)

    def test_disabled_memory(self):
        """Verify max_turns=0 disables memory storage and retrieval."""
        key = "test_disabled"
        self.mem.add_turn(key, "Hello", "World", max_turns=0)
        self.assertEqual(self.mem.get_history(key, max_turns=0), [])

if __name__ == "__main__":
    unittest.main()
