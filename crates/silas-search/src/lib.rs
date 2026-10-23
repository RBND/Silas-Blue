//! `silas-search` — Async web search coordinator with LRU-bounded 5-minute TTL cache,
//! automatic provider failover (`Configured -> Bing -> DuckDuckGo`), intent classification,
//! and XML-sandboxed RAG prompt grounding.

pub mod grounding;
pub mod intent;
pub mod providers;

use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use indexmap::IndexMap;
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use tracing::info;

use silas_core::config::AppSettings;

use crate::providers::{
    BingSearchProvider, BraveSearchProvider, DuckDuckGoSearchProvider, TavilySearchProvider,
};

/// Hard cap on cached search queries to prevent memory growth under spam.
const MAX_SEARCH_CACHE_ENTRIES: usize = 256;
const CACHE_TTL: Duration = Duration::from_secs(300);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WebSearchResult {
    pub title: String,
    pub url: String,
    pub snippet: String,
}

#[async_trait]
pub trait SearchEngine: Send + Sync {
    fn name(&self) -> &str;
    async fn search(&self, query: &str, max_results: usize) -> Vec<WebSearchResult>;
}

struct CachedSearchEntry {
    created_at: Instant,
    results: Vec<WebSearchResult>,
}

#[derive(Clone, Default)]
pub struct SearchCoordinator {
    cache: Arc<RwLock<IndexMap<String, CachedSearchEntry>>>,
}

impl SearchCoordinator {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn clear_cache(&self) {
        self.cache.write().clear();
    }

    fn select_primary_provider(settings: &AppSettings) -> Box<dyn SearchEngine> {
        let prov = settings.web_search_provider.trim().to_ascii_lowercase();
        match prov.as_str() {
            "duckduckgo" => Box::new(DuckDuckGoSearchProvider),
            "tavily" if !settings.tavily_api_key.trim().is_empty() => {
                Box::new(TavilySearchProvider::new(&settings.tavily_api_key))
            }
            "brave" if !settings.brave_api_key.trim().is_empty() => {
                Box::new(BraveSearchProvider::new(&settings.brave_api_key))
            }
            _ => Box::new(BingSearchProvider),
        }
    }

    /// Executes a web search with 5-minute TTL LRU caching and automatic provider failover.
    pub async fn search(
        &self,
        query: &str,
        max_results: Option<usize>,
        settings: &AppSettings,
    ) -> Vec<WebSearchResult> {
        let clean_q = query.trim();
        if clean_q.is_empty() {
            return Vec::new();
        }
        let limit = max_results
            .unwrap_or(settings.web_search_max_results as usize)
            .clamp(1, 10);
        let cache_key = format!("{}_{limit}", clean_q.to_lowercase());

        {
            let mut map = self.cache.write();
            if let Some(entry) = map.get(&cache_key) {
                if entry.created_at.elapsed() < CACHE_TTL {
                    return entry.results.clone();
                }
                map.shift_remove(&cache_key);
            }
        }

        let primary = Self::select_primary_provider(settings);
        let primary_name = primary.name().to_owned();
        let mut results = primary.search(clean_q, limit).await;

        // Resilient cross-provider failover
        if results.is_empty() {
            if primary_name != "Bing" {
                info!(
                    primary = %primary_name,
                    "Primary search provider returned 0 results; failing over to Bing"
                );
                results = BingSearchProvider.search(clean_q, limit).await;
            }
            if results.is_empty() && primary_name != "DuckDuckGo" {
                info!("Trying DuckDuckGo search fallback");
                results = DuckDuckGoSearchProvider.search(clean_q, limit).await;
            }
        }

        if !results.is_empty() {
            let mut map = self.cache.write();
            if map.len() >= MAX_SEARCH_CACHE_ENTRIES {
                map.retain(|_, v| v.created_at.elapsed() < CACHE_TTL);
                if map.len() >= MAX_SEARCH_CACHE_ENTRIES {
                    map.shift_remove_index(0);
                }
            }
            map.insert(
                cache_key,
                CachedSearchEntry {
                    created_at: Instant::now(),
                    results: results.clone(),
                },
            );
        }

        results
    }
}
