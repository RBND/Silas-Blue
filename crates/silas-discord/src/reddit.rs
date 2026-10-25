//! Reddit OAuth2 client, feed ingestion, filtering, and Discord embed delivery service.

use parking_lot::RwLock;
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION, USER_AGENT};
use reqwest::Client;
use serde::Deserialize;
use serenity::all::{
    ChannelId, Colour, CreateEmbed, CreateEmbedAuthor, CreateEmbedFooter, CreateMessage, Http,
    Timestamp,
};
use silas_core::config::AppSettings;
use silas_core::reddit::{clean_subreddit_name, RedditPost, RedditSubscription};
use silas_core::text::truncate_chars;
use silas_storage::Storage;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tracing::{debug, error, info, warn};

const USER_AGENT_STRING: &str = "SilasBlue/4.0.7 (by /u/silas-blue; Discord bot)";

#[derive(Clone)]
struct CachedOAuthToken {
    token: String,
    expires_at: Instant,
}

/// HTTP client for querying Reddit REST endpoints with OAuth2 token caching or unauthenticated fallback.
#[derive(Clone)]
pub struct RedditClient {
    client: Client,
    cached_token: Arc<RwLock<Option<CachedOAuthToken>>>,
}

impl Default for RedditClient {
    fn default() -> Self {
        Self::new()
    }
}

impl RedditClient {
    pub fn new() -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(15))
            .build()
            .unwrap_or_else(|_| Client::new());

        Self {
            client,
            cached_token: Arc::new(RwLock::new(None)),
        }
    }

    /// Obtain a valid OAuth2 bearer token from Reddit using the `client_credentials` grant type.
    async fn get_access_token(&self, settings: &AppSettings) -> Result<String, String> {
        let client_id = settings.reddit_client_id.trim();
        let client_secret = settings.reddit_client_secret.trim();

        if client_id.is_empty() || client_secret.is_empty() {
            return Err("Reddit client credentials not configured".to_string());
        }

        // Check RAM cache
        {
            let guard = self.cached_token.read();
            if let Some(ref cached) = *guard {
                if cached.expires_at > Instant::now() {
                    return Ok(cached.token.clone());
                }
            }
        }

        let mut headers = HeaderMap::new();
        headers.insert(
            USER_AGENT,
            HeaderValue::from_static(USER_AGENT_STRING),
        );

        let res = self
            .client
            .post("https://www.reddit.com/api/v1/access_token")
            .basic_auth(client_id, Some(client_secret))
            .headers(headers)
            .form(&[("grant_type", "client_credentials")])
            .send()
            .await
            .map_err(|e| format!("Reddit OAuth request network error: {e}"))?;

        if !res.status().is_success() {
            let status = res.status();
            let body = res
                .text()
                .await
                .unwrap_or_else(|_| "[unreadable body]".to_string());
            return Err(format!("Reddit OAuth token request failed ({status}): {body}"));
        }

        #[derive(Deserialize)]
        struct TokenResponse {
            access_token: String,
            expires_in: Option<u64>,
        }

        let data: TokenResponse = res
            .json()
            .await
            .map_err(|e| format!("Failed to parse Reddit OAuth JSON: {e}"))?;

        let ttl_secs = data.expires_in.unwrap_or(3600).saturating_sub(60);
        let expires_at = Instant::now() + Duration::from_secs(ttl_secs);

        {
            let mut guard = self.cached_token.write();
            *guard = Some(CachedOAuthToken {
                token: data.access_token.clone(),
                expires_at,
            });
        }

        Ok(data.access_token)
    }

    /// Fetches the newest submissions from a subreddit.
    pub async fn fetch_new_posts(
        &self,
        subreddit: &str,
        limit: u32,
        settings: &AppSettings,
    ) -> Result<Vec<RedditPost>, String> {
        let clean_sub = clean_subreddit_name(subreddit);
        if clean_sub.is_empty() {
            return Ok(Vec::new());
        }

        let oauth_token_res = self.get_access_token(settings).await;
        let mut request = match oauth_token_res {
            Ok(ref token) => {
                let url = format!("https://oauth.reddit.com/r/{clean_sub}/new?limit={limit}");
                self.client
                    .get(&url)
                    .header(AUTHORIZATION, format!("Bearer {token}"))
            }
            Err(e) => {
                debug!("Reddit OAuth not available ({e}); falling back to public JSON endpoint");
                let url = format!("https://www.reddit.com/r/{clean_sub}/new.json?limit={limit}");
                self.client.get(&url)
            }
        };

        request = request.header(USER_AGENT, HeaderValue::from_static(USER_AGENT_STRING));

        let response = request
            .send()
            .await
            .map_err(|e| format!("Failed to fetch Reddit posts for r/{clean_sub}: {e}"))?;

        if !response.status().is_success() {
            let status = response.status();
            return Err(format!(
                "Reddit returned status {status} when fetching r/{clean_sub}"
            ));
        }

        let listing: RedditListingResponse = response
            .json()
            .await
            .map_err(|e| format!("Failed to parse Reddit listing JSON for r/{clean_sub}: {e}"))?;

        let mut posts = Vec::new();
        for child in listing.data.children {
            let d = child.data;
            let id = d.id;
            let subreddit_name = d.subreddit;
            let title = d.title;
            let author = d.author.unwrap_or_else(|| "[deleted]".to_string());
            let permalink = d.permalink.unwrap_or_default();
            let url = d.url.unwrap_or_default();
            let selftext = d.selftext.filter(|s| !s.trim().is_empty());
            let flair = d.link_flair_text.filter(|s| !s.trim().is_empty());
            let score = d.score.unwrap_or(0);
            let is_nsfw = d.over_18.unwrap_or(false);
            let created_utc = d.created_utc.unwrap_or(0.0) as i64;

            let thumbnail_url = d
                .thumbnail
                .filter(|t| t.starts_with("http://") || t.starts_with("https://"));

            let image_url = d
                .preview
                .and_then(|p| p.images)
                .and_then(|imgs| imgs.into_iter().next())
                .and_then(|img| img.source)
                .and_then(|src| src.url)
                .map(|u| html_escape::decode_html_entities(&u).to_string())
                .filter(|u| u.starts_with("http://") || u.starts_with("https://"));

            posts.push(RedditPost {
                id,
                subreddit: subreddit_name,
                title,
                author,
                permalink,
                url,
                selftext,
                flair,
                score,
                is_nsfw,
                thumbnail_url,
                image_url,
                created_utc,
            });
        }

        Ok(posts)
    }
}

// Internal Reddit JSON listing structures
#[derive(Deserialize)]
struct RedditListingResponse {
    data: RedditListingData,
}

#[derive(Deserialize)]
struct RedditListingData {
    children: Vec<RedditListingChild>,
}

#[derive(Deserialize)]
struct RedditListingChild {
    data: RedditChildData,
}

#[derive(Deserialize)]
struct RedditChildData {
    id: String,
    subreddit: String,
    title: String,
    author: Option<String>,
    permalink: Option<String>,
    url: Option<String>,
    selftext: Option<String>,
    link_flair_text: Option<String>,
    score: Option<i32>,
    over_18: Option<bool>,
    thumbnail: Option<String>,
    created_utc: Option<f64>,
    preview: Option<RedditPreviewData>,
}

#[derive(Deserialize)]
struct RedditPreviewData {
    images: Option<Vec<RedditPreviewImage>>,
}

#[derive(Deserialize)]
struct RedditPreviewImage {
    source: Option<RedditPreviewSource>,
}

#[derive(Deserialize)]
struct RedditPreviewSource {
    url: Option<String>,
}

/// Delivers a single `RedditPost` to a `RedditSubscription` destination channel as a rich embed.
pub async fn deliver_reddit_post(
    http: &Arc<Http>,
    storage: &Storage,
    post: &RedditPost,
    sub: &RedditSubscription,
) -> Result<bool, String> {
    if !post.matches_subscription(sub) {
        return Ok(false);
    }

    let post_id = post.normalized_id();
    if storage
        .has_reddit_post_been_delivered(sub.id, post_id)
        .map_err(|e| e.to_string())?
    {
        return Ok(false);
    }

    let title = truncate_chars(&post.title, 250);
    let post_url = post.canonical_url();

    let mut embed = CreateEmbed::new()
        .title(title)
        .url(post_url)
        .colour(Colour::new(0xFF4500))
        .author(CreateEmbedAuthor::new(format!("u/{}", post.author)))
        .field("Subreddit", format!("r/{}", post.normalized_subreddit()), true)
        .field("Score", format!("⬆️ {}", post.score), true)
        .footer(CreateEmbedFooter::new("Reddit"));

    if let Some(ref flair) = post.flair {
        let flair_clean = flair.trim();
        if !flair_clean.is_empty() {
            embed = embed.field("Flair", flair_clean, true);
        }
    }

    if let Some(ref selftext) = post.selftext {
        let text_clean = selftext.trim();
        if !text_clean.is_empty() {
            embed = embed.description(truncate_chars(text_clean, 350));
        }
    }

    if let Some(ref img) = post.image_url {
        embed = embed.image(img);
    } else if let Some(ref thumb) = post.thumbnail_url {
        embed = embed.thumbnail(thumb);
    }

    if post.created_utc > 0 {
        if let Ok(ts) = Timestamp::from_unix_timestamp(post.created_utc) {
            embed = embed.timestamp(ts);
        }
    }

    let channel = ChannelId::new(sub.channel_id);
    let builder = CreateMessage::new().embed(embed);

    match channel.send_message(http, builder).await {
        Ok(_) => {
            storage
                .record_reddit_post_delivered(sub.id, post_id)
                .map_err(|e| e.to_string())?;
            info!(
                "Delivered Reddit post '{}' to channel {} (sub id {})",
                post_id, sub.channel_id, sub.id
            );
            Ok(true)
        }
        Err(e) => {
            warn!(
                "Failed to send Reddit post '{}' to channel {}: {e}",
                post_id, sub.channel_id
            );
            Err(e.to_string())
        }
    }
}

/// Dispatches a post to all subscriptions matching its subreddit across all servers.
pub async fn dispatch_post_to_matching_subscriptions(
    http: &Arc<Http>,
    storage: &Storage,
    post: &RedditPost,
) -> usize {
    let clean_sub = post.normalized_subreddit();
    let subs = match storage.find_subscriptions_for_subreddit(&clean_sub) {
        Ok(s) => s,
        Err(e) => {
            error!("Failed to query subscriptions for r/{clean_sub}: {e}");
            return 0;
        }
    };

    let mut delivered_count = 0;
    for sub in subs {
        match deliver_reddit_post(http, storage, post, &sub).await {
            Ok(true) => delivered_count += 1,
            Ok(false) => {}
            Err(e) => warn!("Delivery error for sub #{}: {e}", sub.id),
        }
    }

    delivered_count
}

/// Spawns the recurring background task that polls Reddit for all registered subscriptions.
pub fn spawn_reddit_poller(
    storage: Storage,
    http_getter: Arc<dyn Fn() -> Option<Arc<Http>> + Send + Sync>,
) {
    tokio::spawn(async move {
        let client = RedditClient::new();
        info!("Reddit background poller worker started.");

        loop {
            let settings = storage.get_app_settings();
            let interval_mins = settings.reddit_poll_interval_minutes.clamp(1, 1440);
            let sleep_duration = Duration::from_secs(u64::from(interval_mins) * 60);

            tokio::time::sleep(sleep_duration).await;

            // Prune old records (older than 30 days)
            let _ = storage.prune_old_reddit_posts(30);

            let http = match http_getter() {
                Some(h) => h,
                None => {
                    debug!("Discord Gateway HTTP client not connected; skipping Reddit poll tick.");
                    continue;
                }
            };

            let subs = match storage.list_all_reddit_subscriptions() {
                Ok(s) => s,
                Err(e) => {
                    warn!("Failed to list reddit subscriptions for poller: {e}");
                    continue;
                }
            };

            if subs.is_empty() {
                continue;
            }

            // Group subscriptions by clean subreddit name
            let mut sub_map: HashMap<String, Vec<RedditSubscription>> = HashMap::new();
            for sub in subs {
                let name = clean_subreddit_name(&sub.subreddit);
                sub_map.entry(name).or_default().push(sub);
            }

            for (subreddit, target_subs) in sub_map {
                // If legacy client credentials are not configured, skip unauthenticated calls
                // to prevent repeated 403 Forbidden errors in logs now that Reddit requires Devvit push.
                if settings.reddit_client_id.trim().is_empty() || settings.reddit_client_secret.trim().is_empty() {
                    continue;
                }

                let posts_res = client.fetch_new_posts(&subreddit, 20, &settings).await;
                match posts_res {
                    Ok(mut posts) => {
                        // Sort oldest to newest so they appear in chronological order
                        posts.sort_by_key(|p| p.created_utc);

                        for post in posts {
                            for sub in &target_subs {
                                let _ = deliver_reddit_post(&http, &storage, &post, sub).await;
                            }
                        }
                    }
                    Err(e) => {
                        warn!("Poller failed to fetch posts for r/{subreddit}: {e}");
                    }
                }

                // Brief courtesy back-off between subreddits
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
        }
    });
}
