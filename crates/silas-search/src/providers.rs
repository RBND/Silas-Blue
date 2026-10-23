//! Search providers (`Bing`, `DuckDuckGo`, `Tavily`, `Brave`) with pre-compiled
//! `LazyLock<scraper::Selector>` DOM parsers and a 512 KB response body cap.

use std::sync::LazyLock;

use async_trait::async_trait;
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use base64::Engine;
use futures_util::StreamExt;
use regex::Regex;
use reqwest::header::{HeaderMap, HeaderValue, ACCEPT, ACCEPT_LANGUAGE, USER_AGENT};
use scraper::{Html, Selector};
use serde::Deserialize;
use serde_json::json;
use tracing::debug;
use url::Url;

use silas_ai::fast_http_client;

use crate::{grounding::sanitize_citation_url, SearchEngine, WebSearchResult};

/// Maximum HTML response size (512 KB) read from Bing or DuckDuckGo to prevent OOM.
const MAX_HTML_BODY_BYTES: usize = 512 * 1024;

const BROWSER_USER_AGENT: &str =
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/126.0.0.0 Safari/537.36";

#[allow(clippy::expect_used)]
static BING_ITEM_SEL: LazyLock<Selector> =
    LazyLock::new(|| Selector::parse("li.b_algo").expect("valid selector"));
#[allow(clippy::expect_used)]
static BING_TITLE_A_SEL: LazyLock<Selector> =
    LazyLock::new(|| Selector::parse("h2 a").expect("valid selector"));
#[allow(clippy::expect_used)]
static BING_SNIPPET_SEL: LazyLock<Selector> =
    LazyLock::new(|| Selector::parse(".b_caption p, p").expect("valid selector"));
#[allow(clippy::expect_used)]
static BING_U_PARAM_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"[?&]u=a1([a-zA-Z0-9_-]+)").expect("valid regex"));

#[allow(clippy::expect_used)]
static DDG_LINK_SEL: LazyLock<Selector> =
    LazyLock::new(|| Selector::parse("a.result-link").expect("valid selector"));
#[allow(clippy::expect_used)]
static DDG_SNIPPET_SEL: LazyLock<Selector> =
    LazyLock::new(|| Selector::parse("td.result-snippet").expect("valid selector"));

async fn read_bounded_text(resp: reqwest::Response) -> Option<String> {
    if !resp.status().is_success() {
        return None;
    }
    let mut stream = resp.bytes_stream();
    let mut buf = Vec::with_capacity(64 * 1024);
    while let Some(Ok(chunk)) = stream.next().await {
        if buf.len().saturating_add(chunk.len()) > MAX_HTML_BODY_BYTES {
            let take = MAX_HTML_BODY_BYTES.saturating_sub(buf.len());
            if let Some(sub) = chunk.get(..take) {
                buf.extend_from_slice(sub);
            }
            break;
        }
        buf.extend_from_slice(&chunk);
    }
    Some(String::from_utf8_lossy(&buf).into_owned())
}

/// Unpacks Bing tracking redirect URLs (`/ck/a?!&&p=...&u=a1<base64>...`).
pub fn unpack_bing_redirect_url(raw_url: &str) -> String {
    let unescaped = html_escape::decode_html_entities(raw_url);
    if let Some(caps) = BING_U_PARAM_RE.captures(&unescaped) {
        if let Some(m) = caps.get(1) {
            let b64_str = m.as_str();
            if let Ok(bytes) = URL_SAFE_NO_PAD.decode(b64_str).or_else(|_| {
                let mut std_b64 = b64_str.replace('-', "+").replace('_', "/");
                while std_b64.len() % 4 != 0 {
                    std_b64.push('=');
                }
                STANDARD.decode(std_b64)
            }) {
                let decoded = String::from_utf8_lossy(&bytes);
                if decoded.starts_with("http://") || decoded.starts_with("https://") {
                    return decoded.into_owned();
                }
            }
        }
    }
    unescaped.into_owned()
}

/// Parses Bing HTML search results using pre-compiled `scraper` selectors.
pub fn parse_bing_html(html_body: &str, max_results: usize) -> Vec<WebSearchResult> {
    let doc = Html::parse_document(html_body);
    let mut results = Vec::with_capacity(max_results);

    for item in doc.select(&BING_ITEM_SEL) {
        let Some(a_el) = item.select(&BING_TITLE_A_SEL).next() else {
            continue;
        };
        let Some(raw_href) = a_el.value().attr("href") else {
            continue;
        };
        let unpacked = unpack_bing_redirect_url(raw_href);
        let Some(final_url) = sanitize_citation_url(&unpacked) else {
            continue;
        };

        let raw_title: String = a_el.text().collect::<Vec<_>>().join(" ");
        let title = raw_title.split_whitespace().collect::<Vec<_>>().join(" ");
        if title.is_empty() {
            continue;
        }

        let snippet = item
            .select(&BING_SNIPPET_SEL)
            .next()
            .map(|p| {
                p.text()
                    .collect::<Vec<_>>()
                    .join(" ")
                    .split_whitespace()
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .unwrap_or_default();

        results.push(WebSearchResult {
            title,
            url: final_url,
            snippet,
        });
        if results.len() >= max_results {
            break;
        }
    }
    results
}

/// Unpacks DuckDuckGo Lite redirect URLs (`//duckduckgo.com/l/?uddg=<url_encoded>`).
pub fn unpack_ddg_redirect_url(raw_href: &str) -> String {
    let full = if raw_href.starts_with("//") {
        format!("https:{raw_href}")
    } else {
        raw_href.to_owned()
    };
    if let Ok(parsed) = Url::parse(&full) {
        for (k, v) in parsed.query_pairs() {
            if k == "uddg" && (v.starts_with("http://") || v.starts_with("https://")) {
                return v.into_owned();
            }
        }
    }
    raw_href.to_owned()
}

/// Parses DuckDuckGo Lite HTML search results.
pub fn parse_ddg_lite_html(html_body: &str, max_results: usize) -> Vec<WebSearchResult> {
    let doc = Html::parse_document(html_body);
    let links: Vec<_> = doc.select(&DDG_LINK_SEL).collect();
    let snippets: Vec<_> = doc.select(&DDG_SNIPPET_SEL).collect();

    let mut results = Vec::with_capacity(max_results);
    for (idx, a_el) in links.into_iter().enumerate() {
        let Some(raw_href) = a_el.value().attr("href") else {
            continue;
        };
        let unpacked = unpack_ddg_redirect_url(raw_href);
        let Some(final_url) = sanitize_citation_url(&unpacked) else {
            continue;
        };
        let title = a_el
            .text()
            .collect::<Vec<_>>()
            .join(" ")
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        if title.is_empty() {
            continue;
        }
        let snippet = snippets
            .get(idx)
            .map(|td| {
                td.text()
                    .collect::<Vec<_>>()
                    .join(" ")
                    .split_whitespace()
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .unwrap_or_default();

        results.push(WebSearchResult {
            title,
            url: final_url,
            snippet,
        });
        if results.len() >= max_results {
            break;
        }
    }
    results
}

pub struct BingSearchProvider;

#[async_trait]
impl SearchEngine for BingSearchProvider {
    fn name(&self) -> &str {
        "Bing"
    }

    async fn search(&self, query: &str, max_results: usize) -> Vec<WebSearchResult> {
        let mut headers = HeaderMap::new();
        headers.insert(USER_AGENT, HeaderValue::from_static(BROWSER_USER_AGENT));
        headers.insert(
            ACCEPT,
            HeaderValue::from_static("text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8"),
        );
        headers.insert(ACCEPT_LANGUAGE, HeaderValue::from_static("en-US,en;q=0.9"));

        let Ok(resp) = fast_http_client()
            .get("https://www.bing.com/search")
            .headers(headers)
            .query(&[("q", query)])
            .send()
            .await
        else {
            return Vec::new();
        };

        let Some(body) = read_bounded_text(resp).await else {
            return Vec::new();
        };
        parse_bing_html(&body, max_results)
    }
}

pub struct DuckDuckGoSearchProvider;

#[async_trait]
impl SearchEngine for DuckDuckGoSearchProvider {
    fn name(&self) -> &str {
        "DuckDuckGo"
    }

    async fn search(&self, query: &str, max_results: usize) -> Vec<WebSearchResult> {
        let mut headers = HeaderMap::new();
        headers.insert(USER_AGENT, HeaderValue::from_static(BROWSER_USER_AGENT));
        headers.insert(
            ACCEPT,
            HeaderValue::from_static("text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8"),
        );

        let Ok(resp) = fast_http_client()
            .post("https://lite.duckduckgo.com/lite/")
            .headers(headers)
            .form(&[("q", query)])
            .send()
            .await
        else {
            return Vec::new();
        };

        let Some(body) = read_bounded_text(resp).await else {
            return Vec::new();
        };
        parse_ddg_lite_html(&body, max_results)
    }
}

pub struct TavilySearchProvider {
    api_key: String,
}

impl TavilySearchProvider {
    pub fn new(api_key: &str) -> Self {
        Self {
            api_key: api_key.trim().to_owned(),
        }
    }
}

#[derive(Deserialize)]
struct TavilyResponse {
    #[serde(default)]
    results: Vec<TavilyItem>,
}

#[derive(Deserialize)]
struct TavilyItem {
    #[serde(default)]
    title: String,
    #[serde(default)]
    url: String,
    #[serde(default)]
    content: String,
}

#[async_trait]
impl SearchEngine for TavilySearchProvider {
    fn name(&self) -> &str {
        "Tavily"
    }

    async fn search(&self, query: &str, max_results: usize) -> Vec<WebSearchResult> {
        if self.api_key.is_empty() {
            return Vec::new();
        }
        let payload = json!({
            "api_key": self.api_key,
            "query": query,
            "max_results": max_results,
            "search_depth": "basic",
            "include_answer": false
        });

        let Ok(resp) = fast_http_client()
            .post("https://api.tavily.com/search")
            .json(&payload)
            .send()
            .await
        else {
            return Vec::new();
        };
        if !resp.status().is_success() {
            debug!(status = resp.status().as_u16(), "Tavily search non-200");
            return Vec::new();
        }
        let Ok(data) = resp.json::<TavilyResponse>().await else {
            return Vec::new();
        };

        data.results
            .into_iter()
            .filter_map(|item| {
                let url = sanitize_citation_url(&item.url)?;
                if item.title.trim().is_empty() {
                    return None;
                }
                Some(WebSearchResult {
                    title: item.title,
                    url,
                    snippet: item.content,
                })
            })
            .take(max_results)
            .collect()
    }
}

pub struct BraveSearchProvider {
    api_key: String,
}

impl BraveSearchProvider {
    pub fn new(api_key: &str) -> Self {
        Self {
            api_key: api_key.trim().to_owned(),
        }
    }
}

#[derive(Deserialize)]
struct BraveResponse {
    web: Option<BraveWebSection>,
}

#[derive(Deserialize)]
struct BraveWebSection {
    #[serde(default)]
    results: Vec<BraveItem>,
}

#[derive(Deserialize)]
struct BraveItem {
    #[serde(default)]
    title: String,
    #[serde(default)]
    url: String,
    #[serde(default)]
    description: String,
}

#[async_trait]
impl SearchEngine for BraveSearchProvider {
    fn name(&self) -> &str {
        "Brave"
    }

    async fn search(&self, query: &str, max_results: usize) -> Vec<WebSearchResult> {
        if self.api_key.is_empty() {
            return Vec::new();
        }
        let mut headers = HeaderMap::new();
        headers.insert(ACCEPT, HeaderValue::from_static("application/json"));
        if let Ok(tok) = HeaderValue::from_str(&self.api_key) {
            headers.insert("X-Subscription-Token", tok);
        } else {
            return Vec::new();
        }

        let Ok(resp) = fast_http_client()
            .get("https://api.search.brave.com/res/v1/web/search")
            .headers(headers)
            .query(&[("q", query), ("count", &max_results.to_string())])
            .send()
            .await
        else {
            return Vec::new();
        };
        if !resp.status().is_success() {
            return Vec::new();
        }
        let Ok(data) = resp.json::<BraveResponse>().await else {
            return Vec::new();
        };

        data.web
            .map_or_else(Vec::new, |w| w.results)
            .into_iter()
            .filter_map(|item| {
                let url = sanitize_citation_url(&item.url)?;
                if item.title.trim().is_empty() {
                    return None;
                }
                Some(WebSearchResult {
                    title: item.title,
                    url,
                    snippet: item.description,
                })
            })
            .take(max_results)
            .collect()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
mod tests {
    use super::*;

    #[test]
    fn test_bing_redirect_unpack_and_html_parsing() {
        // Base64 for "https://www.rust-lang.org/" is "aHR0cHM6Ly93d3cucnVzdC1sYW5nLm9yZy8"
        let html = r#"
        <ul>
          <li class="b_algo">
            <h2><a href="https://www.bing.com/ck/a?!&&p=123&u=a1aHR0cHM6Ly93d3cucnVzdC1sYW5nLm9yZy8&ntb=1">Rust Programming Language</a></h2>
            <div class="b_caption"><p>A language empowering everyone to build reliable and efficient software.</p></div>
          </li>
        </ul>
        "#;
        let res = parse_bing_html(html, 5);
        assert_eq!(res.len(), 1);
        assert_eq!(res[0].title, "Rust Programming Language");
        assert_eq!(res[0].url, "https://www.rust-lang.org/");
        assert!(res[0].snippet.contains("reliable and efficient"));
    }

    #[test]
    fn test_ddg_lite_html_parsing() {
        let html = r#"
        <table>
          <tr><td><a class="result-link" href="//duckduckgo.com/l/?uddg=https%3A%2F%2Ftokio.rs%2F">Tokio - Async Rust</a></td></tr>
          <tr><td class="result-snippet">Tokio is an asynchronous runtime for the Rust programming language.</td></tr>
        </table>
        "#;
        let res = parse_ddg_lite_html(html, 5);
        assert_eq!(res.len(), 1);
        assert_eq!(res[0].title, "Tokio - Async Rust");
        assert_eq!(res[0].url, "https://tokio.rs/");
    }
}
