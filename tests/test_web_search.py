"""
Unit tests for Silas Blue Web Search and Auto-Search integration.
"""

import unittest
import asyncio
from unittest.mock import AsyncMock, patch, MagicMock

import web_search
from web_search import (
    WebSearchResult,
    DuckDuckGoLiteParser,
    DuckDuckGoSearchProvider,
    TavilySearchProvider,
    BraveSearchProvider,
    SearchService,
    should_auto_search,
    format_grounded_prompt
)
from permissions import PermissionManager
from utils import DEFAULT_APP_CONFIG, DEFAULT_GUILD_CONFIG, load_app_config, save_app_config

SAMPLE_DDG_HTML = """
<!DOCTYPE html>
<html>
<body>
<table>
    <tr>
        <td>
            <a class="result-link" href="//duckduckgo.com/l/?uddg=https%3A%2F%2Fdocs.python.org%2F3%2F&rut=123">
                Python 3 Documentation &amp; Guides
            </a>
        </td>
    </tr>
    <tr>
        <td class="result-snippet">
            The official home for Python 3 documentation, language reference, and standard library.
        </td>
    </tr>
    <tr>
        <td>
            <a class="result-link" href="https://example.com/page">
                Example Domain
            </a>
        </td>
    </tr>
    <tr>
        <td class="result-snippet">
            Example domain for illustrative examples in documents.
        </td>
    </tr>
</table>
</body>
</html>
"""

class TestWebSearch(unittest.TestCase):

    def test_ddg_lite_parser(self):
        parser = DuckDuckGoLiteParser()
        parser.feed(SAMPLE_DDG_HTML)
        self.assertEqual(len(parser.results), 2)

        first = parser.results[0]
        self.assertEqual(first.title, "Python 3 Documentation & Guides")
        self.assertEqual(first.url, "https://docs.python.org/3/")
        self.assertIn("official home for Python 3", first.snippet)

        second = parser.results[1]
        self.assertEqual(second.title, "Example Domain")
        self.assertEqual(second.url, "https://example.com/page")
        self.assertIn("Example domain for illustrative", second.snippet)

    def test_should_auto_search_positives(self):
        positives = [
            "What is the latest news regarding the James Webb Telescope?",
            "What is the current price of Ethereum today?",
            "Tell me about the recent SpaceX launch",
            "What is the release date for Python 3.14?",
            "Who won the 2024 World Series?",
            "Search the web for rust async tutorials",
            "Show me the breaking headlines today",
            "What are the patch notes for the new update?",
        ]
        for query in positives:
            with self.subTest(query=query):
                self.assertTrue(
                    should_auto_search(query),
                    f"Expected should_auto_search to be True for: '{query}'"
                )

    def test_should_auto_search_negatives(self):
        negatives = [
            "Hello Silas Blue!",
            "hi",
            "Write a poem about autumn leaves and rain",
            "Tell me a joke about robots",
            "Write a short fantasy story about dragons",
            "repeat after me: foo bar",
            "ping",
            "echo test",
        ]
        for query in negatives:
            with self.subTest(query=query):
                self.assertFalse(
                    should_auto_search(query),
                    f"Expected should_auto_search to be False for: '{query}'"
                )

    def test_format_grounded_prompt(self):
        results = [
            WebSearchResult(
                title="Python 3.13 Released",
                url="https://python.org/news/313",
                snippet="Python 3.13 introduces experimental free-threaded execution."
            ),
            WebSearchResult(
                title="Python Release Notes",
                url="https://docs.python.org/release",
                snippet="Detailed changes for Python 3.13."
            )
        ]

        system_prompt, citations = format_grounded_prompt(
            "What's new in Python?",
            results,
            base_system_prompt="You are Silas Blue."
        )

        self.assertIn("You are Silas Blue.", system_prompt)
        self.assertIn("REAL-TIME WEB SEARCH RESULTS:", system_prompt)
        self.assertIn("Python 3.13 Released", system_prompt)
        self.assertIn("https://python.org/news/313", system_prompt)

        self.assertIn("**Sources:**", citations)
        self.assertIn("[Python 3.13 Released](<https://python.org/news/313>)", citations)

    def test_format_grounded_prompt_empty_results(self):
        system_prompt, citations = format_grounded_prompt(
            "test query",
            [],
            base_system_prompt="Base prompt"
        )
        self.assertEqual(system_prompt, "Base prompt")
        self.assertEqual(citations, "")

    def test_search_service_provider_selection(self):
        # DuckDuckGo default
        svc_default = SearchService({"web_search_provider": "duckduckgo"})
        self.assertIsInstance(svc_default._get_provider(), DuckDuckGoSearchProvider)

        # Tavily with key
        svc_tavily = SearchService({"web_search_provider": "tavily", "tavily_api_key": "test_key"})
        self.assertIsInstance(svc_tavily._get_provider(), TavilySearchProvider)

        # Tavily without key -> fallback to DuckDuckGo
        svc_tavily_no_key = SearchService({"web_search_provider": "tavily", "tavily_api_key": ""})
        self.assertIsInstance(svc_tavily_no_key._get_provider(), DuckDuckGoSearchProvider)

        # Brave with key
        svc_brave = SearchService({"web_search_provider": "brave", "brave_api_key": "test_brave_key"})
        self.assertIsInstance(svc_brave._get_provider(), BraveSearchProvider)

    def test_search_service_caching(self):
        mock_provider = MagicMock(spec=DuckDuckGoSearchProvider)
        mock_provider.name = "MockDDG"

        sample_results = [WebSearchResult("Title 1", "https://example.com/1", "Snippet 1")]
        mock_provider.search = AsyncMock(return_value=sample_results)

        svc = SearchService()
        svc._get_provider = MagicMock(return_value=mock_provider)

        loop = asyncio.new_event_loop()
        try:
            # First call
            r1 = loop.run_until_complete(svc.search("quantum computing", max_results=3))
            self.assertEqual(len(r1), 1)
            self.assertEqual(mock_provider.search.call_count, 1)

            # Second call (identical query) -> should hit cache
            r2 = loop.run_until_complete(svc.search("quantum computing", max_results=3))
            self.assertEqual(len(r2), 1)
            self.assertEqual(mock_provider.search.call_count, 1)  # Provider was NOT called again

            # Clear cache and call again -> should call provider
            svc.clear_cache()
            r3 = loop.run_until_complete(svc.search("quantum computing", max_results=3))
            self.assertEqual(len(r3), 1)
            self.assertEqual(mock_provider.search.call_count, 2)
        finally:
            loop.close()

    def test_permissions_can_search(self):
        pm = PermissionManager()

        # DM context (guild is None)
        self.assertTrue(pm.can_search(MagicMock(), None, {}))

        # Administrator author
        admin_author = MagicMock()
        admin_author.guild_permissions.administrator = True
        self.assertTrue(pm.can_search(admin_author, MagicMock(), {}))

        # Regular user with 'everyone' allowed
        user_author = MagicMock()
        user_author.guild_permissions.administrator = False
        user_author.roles = []
        self.assertTrue(pm.can_search(user_author, MagicMock(), {"search_roles": ["everyone"]}))

        # Restricted search role
        role_searcher = MagicMock()
        role_searcher.name = "WebSearcher"
        user_with_role = MagicMock()
        user_with_role.guild_permissions.administrator = False
        user_with_role.roles = [role_searcher]

        config_restricted = {"search_roles": ["WebSearcher"]}
        self.assertTrue(pm.can_search(user_with_role, MagicMock(), config_restricted))
        self.assertFalse(pm.can_search(user_author, MagicMock(), config_restricted))

    def test_config_search_defaults(self):
        self.assertIn("web_search_enabled", DEFAULT_APP_CONFIG)
        self.assertTrue(DEFAULT_APP_CONFIG["web_search_enabled"])
        self.assertIn("web_search_auto", DEFAULT_APP_CONFIG)
        self.assertTrue(DEFAULT_APP_CONFIG["web_search_auto"])
        self.assertIn("web_search_provider", DEFAULT_APP_CONFIG)
        self.assertEqual(DEFAULT_APP_CONFIG["web_search_provider"], "duckduckgo")

        self.assertIn("web_search_enabled", DEFAULT_GUILD_CONFIG)
        self.assertTrue(DEFAULT_GUILD_CONFIG["web_search_enabled"])
        self.assertIn("web_search_auto", DEFAULT_GUILD_CONFIG)
        self.assertTrue(DEFAULT_GUILD_CONFIG["web_search_auto"])
        self.assertIn("search_roles", DEFAULT_GUILD_CONFIG)

    def tearDown(self):
        import os
        for gid in [998866, 998877]:
            p = os.path.join("config", f"{gid}.json")
            if os.path.exists(p):
                try:
                    os.remove(p)
                except Exception:
                    pass

    def test_autosearch_command(self):
        from bot_core import autosearch_command, server_configs
        ctx = MagicMock()
        ctx.guild = MagicMock()
        ctx.guild.id = 998877
        ctx.author.guild_permissions.administrator = True
        ctx.send = AsyncMock()

        server_configs[998877] = {"web_search_auto": True}

        loop = asyncio.new_event_loop()
        try:
            # Turn off
            loop.run_until_complete(autosearch_command(ctx, "off"))
            self.assertFalse(server_configs[998877]["web_search_auto"])
            ctx.send.assert_called_with("🌐 Auto-search mode is now **DISABLED** for this server.")

            # Turn on
            loop.run_until_complete(autosearch_command(ctx, "on"))
            self.assertTrue(server_configs[998877]["web_search_auto"])
            ctx.send.assert_called_with("🌐 Auto-search mode is now **ENABLED** for this server.")
        finally:
            loop.close()

    def test_set_web_search_command(self):
        from bot_core import set_web_search_command, server_configs
        ctx = MagicMock()
        ctx.guild = MagicMock()
        ctx.guild.id = 998866
        ctx.author.guild_permissions.administrator = True
        ctx.send = AsyncMock()

        server_configs[998866] = {"web_search_enabled": True}

        loop = asyncio.new_event_loop()
        try:
            # Turn off
            loop.run_until_complete(set_web_search_command(ctx, "off"))
            self.assertFalse(server_configs[998866]["web_search_enabled"])
            ctx.send.assert_called_with("🌐 Web search is now **DISABLED** for this server.")

            # Toggle (state=None)
            loop.run_until_complete(set_web_search_command(ctx, None))
            self.assertTrue(server_configs[998866]["web_search_enabled"])
            ctx.send.assert_called_with("🌐 Web search is now **ENABLED** for this server.")
        finally:
            loop.close()

if __name__ == "__main__":
    unittest.main()

