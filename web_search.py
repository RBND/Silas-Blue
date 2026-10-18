"""
Web Search module for Silas Blue.
Provides async search across DuckDuckGo (free, zero-config), Tavily, and Brave Search.
Supports Retrieval-Augmented Generation (RAG) grounding and auto-search intent detection.
"""

import aiohttp
import asyncio
import re
import time
import logging
import urllib.parse
import html
import base64
from abc import ABC, abstractmethod
from dataclasses import dataclass
from html.parser import HTMLParser
from typing import List, Optional, Dict, Any, Tuple

logger = logging.getLogger("silasblue")

@dataclass
class WebSearchResult:
    title: str
    url: str
    snippet: str

class DuckDuckGoLiteParser(HTMLParser):
    """
    Parses search results from lite.duckduckgo.com/lite/.
    Extracts real target URLs (decoding DDG redirect wrappers), titles, and snippets.
    """
    def __init__(self):
        super().__init__()
        self.results: List[WebSearchResult] = []
        self._in_link = False
        self._in_snippet = False
        self._current_item: Optional[Dict[str, str]] = None
        self._text_buffer: List[str] = []

    def handle_starttag(self, tag: str, attrs: list):
        attrs_dict = dict(attrs)
        class_name = attrs_dict.get("class", "")

        if tag == "a" and "result-link" in class_name:
            self._in_link = True
            href = attrs_dict.get("href", "")
            if "uddg=" in href:
                parsed = urllib.parse.parse_qs(urllib.parse.urlparse(href).query)
                real_url = parsed.get("uddg", [href])[0]
            else:
                real_url = href
            self._current_item = {"title": "", "url": real_url, "snippet": ""}
            self._text_buffer = []
        elif tag == "td" and "result-snippet" in class_name:
            self._in_snippet = True
            self._text_buffer = []

    def handle_endtag(self, tag: str):
        if tag == "a" and self._in_link:
            self._in_link = False
            if self._current_item:
                self._current_item["title"] = " ".join("".join(self._text_buffer).split())
            self._text_buffer = []
        elif tag == "td" and self._in_snippet:
            self._in_snippet = False
            if self._current_item:
                self._current_item["snippet"] = " ".join("".join(self._text_buffer).split())
                if self._current_item["url"] and self._current_item["title"]:
                    self.results.append(WebSearchResult(
                        title=self._current_item["title"],
                        url=self._current_item["url"],
                        snippet=self._current_item["snippet"]
                    ))
                self._current_item = None
            self._text_buffer = []

    def handle_data(self, data: str):
        if self._in_link or self._in_snippet:
            self._text_buffer.append(data)


class SearchProvider(ABC):
    @property
    @abstractmethod
    def name(self) -> str:
        """Name of the search provider."""
        pass

    @abstractmethod
    async def search(self, query: str, max_results: int = 5) -> List[WebSearchResult]:
        """Performs search and returns a list of WebSearchResult objects."""
        pass


class DuckDuckGoSearchProvider(SearchProvider):
    """
    Zero-config search provider querying DuckDuckGo Lite.
    Fast, asynchronous, and requires no API key.
    """
    SEARCH_URL = "https://lite.duckduckgo.com/lite/"
    USER_AGENT = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36"

    @property
    def name(self) -> str:
        return "DuckDuckGo"

    async def search(self, query: str, max_results: int = 5) -> List[WebSearchResult]:
        headers = {
            "User-Agent": self.USER_AGENT,
            "Accept": "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8",
            "Content-Type": "application/x-www-form-urlencoded"
        }
        data = {"q": query}
        timeout = aiohttp.ClientTimeout(total=12)

        try:
            async with aiohttp.ClientSession(headers=headers, timeout=timeout) as session:
                async with session.post(self.SEARCH_URL, data=data) as resp:
                    if resp.status != 200:
                        logger.warning(f"DuckDuckGo search returned status {resp.status} for query: {query}")
                        return []
                    html_content = await resp.text()
                    parser = DuckDuckGoLiteParser()
                    parser.feed(html_content)
                    return parser.results[:max_results]
        except Exception as e:
            logger.error(f"Error querying DuckDuckGo search: {e}")
            return []


class BingSearchProvider(SearchProvider):
    """
    Zero-config search provider querying Bing search.
    Fast, reliable, does not trigger bot blocks, and requires no API key.
    """
    SEARCH_URL = "https://www.bing.com/search"
    USER_AGENT = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36"

    @property
    def name(self) -> str:
        return "Bing"

    async def search(self, query: str, max_results: int = 5) -> List[WebSearchResult]:
        headers = {
            "User-Agent": self.USER_AGENT,
            "Accept": "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8",
            "Accept-Language": "en-US,en;q=0.9",
        }
        params = {"q": query}
        timeout = aiohttp.ClientTimeout(total=12)

        results: List[WebSearchResult] = []
        try:
            async with aiohttp.ClientSession(headers=headers, timeout=timeout) as session:
                async with session.get(self.SEARCH_URL, params=params) as resp:
                    if resp.status != 200:
                        logger.warning(f"Bing search returned status {resp.status} for query: {query}")
                        return []
                    body = await resp.text()

            items = re.findall(r'<li class="b_algo"[^>]*>(.*?)</li>', body, re.DOTALL)
            for item in items:
                t_match = re.search(r'<h2[^>]*><a[^>]+href="([^"]+)"[^>]*>(.*?)</a></h2>', item)
                p_match = re.search(r'<div class="b_caption"[^>]*>.*?<p[^>]*>(.*?)</p>', item, re.DOTALL) or re.search(r'<p[^>]*>(.*?)</p>', item, re.DOTALL)
                if t_match:
                    raw_url = html.unescape(t_match.group(1))
                    final_url = raw_url
                    # Unpack Bing tracking/redirect URLs: /ck/a?!&&p=...&u=a1<base64>...
                    if "&u=" in raw_url or "?u=" in raw_url:
                        m = re.search(r'[?&]u=a1([a-zA-Z0-9_-]+)', raw_url)
                        if m:
                            raw_b64 = m.group(1).replace('-', '+').replace('_', '/')
                            pad = '=' * (-len(raw_b64) % 4)
                            try:
                                decoded = base64.b64decode((raw_b64 + pad).encode()).decode('utf-8', errors='ignore')
                                if decoded.startswith("http://") or decoded.startswith("https://"):
                                    final_url = decoded
                            except Exception:
                                pass
                    title = re.sub(r'<[^>]+>', '', t_match.group(2)).strip()
                    title = " ".join(html.unescape(title).split())
                    snippet = ""
                    if p_match:
                        raw_snip = re.sub(r'<[^>]+>', '', p_match.group(1)).strip()
                        snippet = " ".join(html.unescape(raw_snip).split())
                    if title and final_url:
                        results.append(WebSearchResult(title=title, url=final_url, snippet=snippet))
                        if len(results) >= max_results:
                            break
            return results
        except Exception as e:
            logger.error(f"Error querying Bing search: {e}")
            return []


class TavilySearchProvider(SearchProvider):
    """
    Search provider using the Tavily AI Search API (POST https://api.tavily.com/search).
    Requires a valid tavily_api_key.
    """
    def __init__(self, api_key: str):
        self.api_key = api_key.strip()

    @property
    def name(self) -> str:
        return "Tavily"

    async def search(self, query: str, max_results: int = 5) -> List[WebSearchResult]:
        if not self.api_key:
            logger.warning("Tavily API key is empty.")
            return []

        url = "https://api.tavily.com/search"
        payload = {
            "api_key": self.api_key,
            "query": query,
            "max_results": max_results,
            "search_depth": "basic",
            "include_answer": False
        }
        timeout = aiohttp.ClientTimeout(total=15)

        try:
            async with aiohttp.ClientSession(timeout=timeout) as session:
                async with session.post(url, json=payload) as resp:
                    if resp.status != 200:
                        err_text = await resp.text()
                        logger.warning(f"Tavily search error {resp.status}: {err_text}")
                        return []
                    data = await resp.json()
                    raw_results = data.get("results", [])
                    results = []
                    for item in raw_results[:max_results]:
                        title = item.get("title", "")
                        link = item.get("url", "")
                        snippet = item.get("content", "")
                        if title and link:
                            results.append(WebSearchResult(title=title, url=link, snippet=snippet))
                    return results
        except Exception as e:
            logger.error(f"Error querying Tavily search: {e}")
            return []


class BraveSearchProvider(SearchProvider):
    """
    Search provider using the Brave Search API (GET https://api.search.brave.com/res/v1/web/search).
    Requires a valid brave_api_key.
    """
    def __init__(self, api_key: str):
        self.api_key = api_key.strip()

    @property
    def name(self) -> str:
        return "Brave"

    async def search(self, query: str, max_results: int = 5) -> List[WebSearchResult]:
        if not self.api_key:
            logger.warning("Brave API key is empty.")
            return []

        url = "https://api.search.brave.com/res/v1/web/search"
        headers = {
            "Accept": "application/json",
            "X-Subscription-Token": self.api_key
        }
        params = {"q": query, "count": str(max_results)}
        timeout = aiohttp.ClientTimeout(total=15)

        try:
            async with aiohttp.ClientSession(headers=headers, timeout=timeout) as session:
                async with session.get(url, params=params) as resp:
                    if resp.status != 200:
                        err_text = await resp.text()
                        logger.warning(f"Brave search error {resp.status}: {err_text}")
                        return []
                    data = await resp.json()
                    web_items = data.get("web", {}).get("results", [])
                    results = []
                    for item in web_items[:max_results]:
                        title = item.get("title", "")
                        link = item.get("url", "")
                        snippet = item.get("description", "")
                        if title and link:
                            results.append(WebSearchResult(title=title, url=link, snippet=snippet))
                    return results
        except Exception as e:
            logger.error(f"Error querying Brave search: {e}")
            return []


class SearchService:
    """
    Coordinator service for web search with caching and provider fallback.
    """
    def __init__(self, config: Optional[Dict[str, Any]] = None):
        self.config = config or {}
        self._cache: Dict[str, Tuple[float, List[WebSearchResult]]] = {}
        self._cache_ttl = 300  # 5 minutes cache TTL

    def _get_provider(self) -> SearchProvider:
        provider_name = (self.config.get("web_search_provider") or "bing").lower()
        if provider_name == "duckduckgo":
            return DuckDuckGoSearchProvider()
        elif provider_name == "tavily":
            key = self.config.get("tavily_api_key", "")
            if key:
                return TavilySearchProvider(key)
            logger.info("Tavily API key not configured, falling back to Bing.")
        elif provider_name == "brave":
            key = self.config.get("brave_api_key", "")
            if key:
                return BraveSearchProvider(key)
            logger.info("Brave API key not configured, falling back to Bing.")
        return BingSearchProvider()

    async def search(self, query: str, max_results: Optional[int] = None) -> List[WebSearchResult]:
        query = query.strip()
        if not query:
            return []

        limit = max_results or int(self.config.get("web_search_max_results", 5))
        limit = max(1, min(limit, 10))

        cache_key = f"{query.lower()}_{limit}"
        now = time.time()
        if cache_key in self._cache:
            ts, cached_results = self._cache[cache_key]
            if now - ts < self._cache_ttl:
                return cached_results

        provider = self._get_provider()
        results = await provider.search(query, max_results=limit)

        # Resilient fallback if primary provider returned empty results or failed
        if not results:
            if not isinstance(provider, BingSearchProvider):
                logger.info(f"{provider.name} returned 0 results or failed. Falling back to Bing.")
                fallback_prov = BingSearchProvider()
                results = await fallback_prov.search(query, max_results=limit)
            else:
                logger.info("Bing returned 0 results. Trying DuckDuckGo fallback.")
                fallback_prov = DuckDuckGoSearchProvider()
                results = await fallback_prov.search(query, max_results=limit)

        if results:
            self._cache[cache_key] = (now, results)

        return results

    def clear_cache(self):
        self._cache.clear()


# --- Auto-Search Intent Detection & Query Formulation ---

TEMPORAL_PATTERNS = [
    r"\b(right now|as of (?:today|now)|at the moment|happening now|currently happening)\b",
    r"\b(?:latest|current|recent|newest|upcoming)\s+(?:news|updates?|version|release|patch|price|score|events?|info|information|specs?|status|data|stats?|trends?|developments?)\b",
    r"\b(?:today|tonight|yesterday|tomorrow)(?:'s)?\s+(?:news|weather|scores?|games?|stock|market|events?|headlines?|updates?|matches?|results?)\b",
    r"\b(?:news|weather|scores?|price|stock|events?|matches?|results?)\s+(?:today|tonight|yesterday|tomorrow|this week|this month|this year)\b",
    r"\b(?:who won|what happened|results? for|schedule for|winner of|champion of)\s+(?:today|tonight|yesterday|this week|202[4-9]|203[0-9])\b",
    r"\b(?:released?|announced?|unveiled?|launched?)\s+(?:today|yesterday|recently|in 202[4-9]|in 203[0-9])\b",
    r"\b202[4-9]\s+(?:news|release|specs|features|election|championship|winner|schedule|update|games?|movies?)\b",
    r"^(?:what|who|when|where|how|which)\b.*\b202[4-9]\b.*\?",
]

LOOKUP_PATTERNS = [
    r"^(?:what|who|when|where|how|which)\b\s+(?:is|was|are|were|happened|occurred|announced|released|won|leads|plays)\b.*(?:today|tonight|recently|current|latest|this week|202[4-9]|203[0-9])",
    r"\b(news|headline|headlines|breaking news|patch notes|changelog)\b",
    r"\b(release date|released|specs|specifications|price of|price for|weather in|score of|who won)\b",
    r"\b(search the web|search for|google|look up|lookup|find info on|browse for|check online)\b",
    r"\b(part number|part numbers|model number|model numbers|sku|skus|datasheet|datasheets|pinout|pinouts)\b",
    r"\b(benchmark|benchmarks|cost of|how much is|where to buy|msrp|in stock)\b",
    r"\b(to search|can you search|can you look up|can you find|give me a part number|give me a model|give me the model|give me the part)\b",
    r"\brecent\s+[A-Za-z0-9]+\s+(?:launch|flight|mission|test|event|game|update)\b",
    r"\bmost\s+recent\b",
]

NON_SEARCH_PATTERNS = [
    r"^(hi|hello|hey|greetings|howdy|yo|good morning|good evening|good afternoon)\b",
    r"\b(write a poem|write a song|write a story|tell me a joke|compose a)\b",
    r"^(?:write|create|generate|code|debug)\s+(?:a\s+)?(?:python|javascript|script|code|program|function|class|algorithm)\b",
    r"\b(repeat after me|translate this|ignore previous instructions)\b",
    r"^(ping|test|echo)\b",
    r"\b(?:shut\s*down|turn\s+off|power\s+off|reboot|restart)\s+(?:your|the|this)?\s*(?:computer|pc|system|server|machine|host|bot)\b",
    r"^now\s+(?:that|then|listen|look|what|why|how|tell|let|i|we|you)\b",
    r"\bnow\s+that\s+you\b",
    r"\b(?:from\s+now\s+on|for\s+now|until\s+now|by\s+now|just\s+now)\b",
    r"^(?:why|how|what)\s+(?:did\s+you|are\s+you|were\s+those|made\s+you)\s+(?:show|give|send|link|search|say|mention)\b",
    r"\bwhy\s+did\s+you\s+(?:show|give|send|link|search|say)\b",
    r"^(?:how\s+are\s+you|how\s+do\s+you\s+feel|how's\s+it\s+going|what\s+are\s+you\s+doing|what\s+can\s+you\s+do)\b",
    r"^(?:who\s+are\s+you|what\s+is\s+your\s+name|introduce\s+yourself|tell\s+me\s+about\s+yourself)\b",
]

GENERIC_LOOKUP_WORDS = {
    "part", "number", "part number", "model", "model number", "sku", "skus",
    "datasheet", "specs", "specifications", "price", "cost", "link", "url",
    "pinout", "search", "where", "buy", "online", "details", "info", "information",
    "a", "an", "the", "me", "to", "for", "give", "need", "you"
}

FILLER_PREFIXES = [
    r"^(?:no\s*,?\s*)?(?:please\s+)?(?:can\s+you\s+|could\s+you\s+|i\s+need\s+you\s+to\s+|i\s+need\s+|give\s+me\s+|tell\s+me\s+|show\s+me\s+|what\s+is\s+|what\s+are\s+)+",
    r"^[!?/]*(?:search\s+(?:the\s+web\s+for|the\s+internet\s+for|for|online\s+for)?|google|look\s+up|find\s+info\s+on|browse\s+for)\s+",
]

def clean_query_text(text: str) -> str:
    cleaned = text.strip()
    for pat in FILLER_PREFIXES:
        cleaned = re.sub(pat, "", cleaned, flags=re.IGNORECASE).strip()
    cleaned = re.sub(r"\s+to\s+(?:search|look\s+up|google|find)\.?$", "", cleaned, flags=re.IGNORECASE).strip()
    cleaned = re.sub(r"^[?!.,;:]+|[?!.,;:]+$", "", cleaned).strip()
    return cleaned

def extract_context_keywords(context_text: str) -> str:
    """Extracts high-signal subject keywords from context text."""
    if not context_text:
        return ""
    stripped = re.sub(r"^\[Replying to [^:]+:\s*\"?", "", context_text)
    stripped = re.sub(r"\"?\]\s*$", "", stripped)
    stripped = re.sub(r"\bPage\s+\d+(?:/\d+)?\b", "", stripped, flags=re.IGNORECASE)
    stripped = stripped.replace("\\n", " ").replace("\n", " ")
    stripped = re.sub(r"[*_`#~>\[\]\(\)|]", " ", stripped)

    tokens = re.findall(r"\b[A-Za-z0-9]+(?:[-_./+][A-Za-z0-9]+)*\b", stripped)

    stop_words = {
        "page", "the", "and", "or", "for", "with", "this", "that", "you", "your",
        "silas", "blue", "replying", "response", "summary", "checklist", "step",
        "recommended", "solution", "upgrade", "table", "need", "give", "can", "will",
        "about", "from", "are", "not", "have", "been", "would", "like", "using",
        "what", "when", "where", "which", "there", "here", "also", "just", "more"
    }

    seen = set()
    subject_tokens = []
    for tok in tokens:
        low = tok.lower()
        if low not in stop_words and len(tok) > 1 and not tok.isdigit():
            if any(c.isdigit() for c in tok) or "-" in tok or "+" in tok or tok.isupper() or (tok[0].isupper() and len(tok) > 3):
                if low not in seen:
                    seen.add(low)
                    subject_tokens.append(tok)
                    if len(subject_tokens) >= 8:
                        break

    return " ".join(subject_tokens)

def build_search_query(
    raw_prompt: str,
    replied_context: str = "",
    history: Optional[List[Dict[str, str]]] = None
) -> str:
    """
    Constructs an optimized, grounded search query from a user prompt,
    enriching vague follow-up prompts with contextual subjects when available.
    """
    cleaned = clean_query_text(raw_prompt)
    words = set(re.findall(r"\b\w+\b", cleaned.lower()))
    is_vague = (len(words) <= 3 or words.issubset(GENERIC_LOOKUP_WORDS))

    if not is_vague:
        return cleaned

    context_subject = ""
    if replied_context:
        context_subject = extract_context_keywords(replied_context)

    if not context_subject and history:
        for item in reversed(history):
            if item.get("role") == "user":
                prev_text = item.get("content", "")
                context_subject = extract_context_keywords(prev_text)
                if context_subject:
                    break
        if not context_subject:
            for item in reversed(history):
                if item.get("role") == "assistant":
                    context_subject = extract_context_keywords(item.get("content", ""))
                    if context_subject:
                        break

    if context_subject:
        cleaned_suffix = re.sub(r'^(?:a|an|the)\s+', '', cleaned, flags=re.IGNORECASE).strip()
        return f"{context_subject} {cleaned_suffix}".strip()
    return cleaned or raw_prompt

def should_auto_search(prompt: str) -> bool:
    """
    Evaluates whether a user prompt has search intent or requests real-time / factual web knowledge.
    Returns True if auto-search should be invoked.
    """
    text = prompt.strip().lower()
    if len(text) < 4:
        return False

    # Negative exclusions
    for pat in NON_SEARCH_PATTERNS:
        if re.search(pat, text):
            return False

    # Explicit lookup requests
    for pat in LOOKUP_PATTERNS:
        if re.search(pat, text):
            return True

    # Temporal indicator check
    for pat in TEMPORAL_PATTERNS:
        if re.search(pat, text):
            return True

    return False


# --- Grounded Prompt Formatter ---

def format_grounded_prompt(
    user_query: str,
    results: List[WebSearchResult],
    base_system_prompt: Optional[str] = None
) -> Tuple[str, str]:
    """
    Constructs an augmented system prompt incorporating search results (RAG grounding)
    and produces a formatted source citation footer for Discord.
    
    Returns:
        (augmented_system_prompt, citation_footer)
    """
    if not results:
        return base_system_prompt or "", ""

    snippets_text = []
    citations = []

    for idx, r in enumerate(results, start=1):
        snippets_text.append(f"[{idx}] Title: {r.title}\nURL: {r.url}\nExcerpt: {r.snippet}")
        clean_title = r.title.replace("[", "").replace("]", "")
        citations.append(f"• [{clean_title}](<{r.url}>)")

    grounding_context = (
        "REAL-TIME WEB SEARCH RESULTS:\n"
        "------------------------------------\n"
        + "\n\n".join(snippets_text) + "\n"
        "------------------------------------\n"
        "GROUNDING INSTRUCTIONS:\n"
        "- The above search results provide current, factual information relevant to the user's question.\n"
        "- Synthesize a comprehensive, direct, and well-structured answer based on these real-time web results.\n"
        "- When citing specific facts or quotes, refer to the corresponding source index or title.\n"
        "- If the search results do not contain enough information, acknowledge what is known and clarify the limitation.\n"
        "- If the search results are irrelevant to the user's request, disregard them and respond directly without citing them."
    )

    if base_system_prompt:
        augmented_system = f"{base_system_prompt}\n\n{grounding_context}"
    else:
        augmented_system = grounding_context

    citation_footer = "\n\n**Sources:**\n" + "\n".join(citations[:5])

    return augmented_system, citation_footer
