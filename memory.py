"""
Silas Blue - Conversation Memory Engine
Provides secure, isolated sliding-window conversation history with smart compression
and configurable inactivity timeouts (TTL).
"""

import time
import threading
from typing import List, Dict, Optional, Tuple

class ConversationMemory:
    """
    Manages multi-turn conversation memory for Discord channels and direct messages (DMs).
    
    Security & Privacy Guarantees:
    - Server channels are keyed by `guild_{guild_id}_{channel_id}` ensuring strict channel isolation.
    - DMs are keyed strictly by `dm_user_{author_id}` ensuring zero cross-user context leakage.
    - Automatic inactivity timeout flushes memory after a configurable period of silence.
    - Smart compression truncates verbose historical AI replies to conserve model context tokens.
    """

    def __init__(self, default_ttl_minutes: int = 30, max_assistant_chars: int = 1500):
        self._default_ttl = default_ttl_minutes * 60
        self._max_assistant_chars = max_assistant_chars
        self._lock = threading.Lock()
        # buffer structure: key -> { "last_active": float, "messages": List[Dict[str, str]] }
        self._buffers: Dict[str, Dict] = {}

    @staticmethod
    def get_channel_key(guild_id: int, channel_id: int) -> str:
        """Returns the isolated key for a guild text channel."""
        return f"guild_{guild_id}_{channel_id}"

    @staticmethod
    def get_dm_key(user_id: int) -> str:
        """Returns the strictly user-bound key for Direct Messages."""
        return f"dm_user_{user_id}"

    def get_history(self, key: str, max_turns: int = 6, ttl_minutes: Optional[int] = None) -> List[Dict[str, str]]:
        """
        Retrieves the conversation history for a given context key.
        Flushes buffer if inactivity TTL has expired.
        """
        if max_turns <= 0:
            return []

        ttl_seconds = (ttl_minutes * 60) if ttl_minutes is not None else self._default_ttl

        with self._lock:
            buf = self._buffers.get(key)
            if not buf:
                return []

            now = time.time()
            if ttl_seconds > 0 and (now - buf.get("last_active", 0)) > ttl_seconds:
                # Expired due to inactivity
                del self._buffers[key]
                return []

            # Update activity timestamp
            buf["last_active"] = now

            # Return a copy of up to max_turns messages
            msgs = buf.get("messages", [])
            return [dict(m) for m in msgs[-max_turns:]]

    def add_turn(self, key: str, user_prompt: str, assistant_response: str, max_turns: int = 10):
        """
        Adds a user prompt and assistant response turn to the conversation history.
        Applies smart compression to the historical assistant response while preserving
        source citations for contextual grounding across multiple turns.
        """
        if max_turns <= 0:
            return

        compressed_assistant = assistant_response.strip()
        if len(compressed_assistant) > self._max_assistant_chars:
            if "\n\n**Sources:**" in compressed_assistant:
                body, sources_part = compressed_assistant.split("\n\n**Sources:**", 1)
                sources_suffix = "\n\n**Sources:**" + sources_part
                budget_for_body = max(100, self._max_assistant_chars - len(sources_suffix) - 30)
                if len(body) > budget_for_body:
                    body = body[:budget_for_body].rstrip() + "... [truncated for brevity]"
                compressed_assistant = body + sources_suffix
            else:
                compressed_assistant = compressed_assistant[:self._max_assistant_chars].rstrip() + "... [truncated for brevity]"

        with self._lock:
            now = time.time()
            # Periodically sweep stale buffers if buffer count grows large
            if len(self._buffers) > 100 and self._default_ttl > 0:
                max_idle = max(self._default_ttl * 2, 86400)
                expired_keys = [k for k, v in self._buffers.items() if (now - v.get("last_active", 0)) > max_idle]
                for k in expired_keys:
                    del self._buffers[k]

            if key not in self._buffers:
                self._buffers[key] = {
                    "last_active": now,
                    "messages": []
                }

            buf = self._buffers[key]
            buf["last_active"] = now

            buf["messages"].append({"role": "user", "content": user_prompt.strip()})
            buf["messages"].append({"role": "assistant", "content": compressed_assistant})

            # Trim buffer to max bounds (allow twice max_turns for user+assistant pairs)
            keep_limit = max_turns * 2
            if len(buf["messages"]) > keep_limit:
                buf["messages"] = buf["messages"][-keep_limit:]

    def clear(self, key: str) -> bool:
        """Flushes memory for the specified key. Returns True if memory was cleared."""
        with self._lock:
            if key in self._buffers:
                del self._buffers[key]
                return True
            return False

    def clear_all(self):
        """Flushes all stored conversation buffers."""
        with self._lock:
            self._buffers.clear()

    def get_stats(self) -> Dict[str, int]:
        """Returns statistics on active conversation sessions."""
        with self._lock:
            total_sessions = len(self._buffers)
            dm_sessions = sum(1 for k in self._buffers if k.startswith("dm_user_"))
            guild_sessions = total_sessions - dm_sessions
            total_messages = sum(len(b.get("messages", [])) for b in self._buffers.values())
            return {
                "total_sessions": total_sessions,
                "dm_sessions": dm_sessions,
                "guild_sessions": guild_sessions,
                "total_stored_messages": total_messages
            }

# Canonical singleton instance for the Silas Blue process
conversation_memory = ConversationMemory()
