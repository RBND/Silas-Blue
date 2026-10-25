//! Subreddit feed subscription models and unified Reddit post schema.

use serde::{Deserialize, Serialize};

/// Represents an active subreddit monitor tied to a specific Discord channel in a guild.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RedditSubscription {
    pub id: i64,
    pub guild_id: u64,
    pub channel_id: u64,
    pub subreddit: String,
    pub filter_nsfw: bool,
    pub min_score: i32,
    pub flair_filter: Option<String>,
    pub keyword_filter: Option<String>,
    pub created_at: i64,
    pub last_seen_post_id: Option<String>,
}

/// Request parameters to subscribe a Discord channel to a subreddit.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewRedditSubscription {
    pub guild_id: u64,
    pub channel_id: u64,
    pub subreddit: String,
    #[serde(default = "default_filter_nsfw")]
    pub filter_nsfw: bool,
    #[serde(default)]
    pub min_score: i32,
    #[serde(default)]
    pub flair_filter: Option<String>,
    #[serde(default)]
    pub keyword_filter: Option<String>,
    #[serde(default)]
    pub backfill_count: u32,
}

fn default_filter_nsfw() -> bool {
    true
}

/// Unified post representation supporting both Devvit webhooks and the Reddit REST/OAuth API.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RedditPost {
    pub id: String,
    pub subreddit: String,
    pub title: String,
    #[serde(default = "default_unknown_author")]
    pub author: String,
    #[serde(default)]
    pub permalink: String,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub selftext: Option<String>,
    #[serde(default)]
    pub flair: Option<String>,
    #[serde(default)]
    pub score: i32,
    #[serde(default)]
    pub is_nsfw: bool,
    #[serde(default)]
    pub thumbnail_url: Option<String>,
    #[serde(default)]
    pub image_url: Option<String>,
    #[serde(default)]
    pub created_utc: i64,
}

fn default_unknown_author() -> String {
    "[deleted]".to_string()
}

impl RedditPost {
    /// Normalized post ID without any Reddit type prefixes (e.g. `t3_`).
    pub fn normalized_id(&self) -> &str {
        if let Some(stripped) = self.id.strip_prefix("t3_") {
            stripped
        } else {
            &self.id
        }
    }

    /// Normalized lowercase subreddit name without `r/` prefix.
    pub fn normalized_subreddit(&self) -> String {
        let trimmed = self.subreddit.trim();
        let without_prefix = if let Some(stripped) = trimmed.strip_prefix("r/") {
            stripped
        } else if let Some(stripped) = trimmed.strip_prefix("/r/") {
            stripped
        } else {
            trimmed
        };
        without_prefix.to_ascii_lowercase()
    }

    /// Full canonical Reddit permalink URL.
    pub fn canonical_url(&self) -> String {
        let p = self.permalink.trim();
        if p.starts_with("http://") || p.starts_with("https://") {
            p.to_string()
        } else if p.starts_with('/') {
            format!("https://www.reddit.com{p}")
        } else if !p.is_empty() {
            format!("https://www.reddit.com/{p}")
        } else {
            let u = self.url.trim();
            if u.is_empty() {
                format!(
                    "https://www.reddit.com/r/{}/comments/{}",
                    self.normalized_subreddit(),
                    self.normalized_id()
                )
            } else {
                u.to_string()
            }
        }
    }

    /// Check if post matches subscription filter rules.
    pub fn matches_subscription(&self, sub: &RedditSubscription) -> bool {
        // 1. Subreddit match check
        let sub_target = clean_subreddit_name(&sub.subreddit);
        if self.normalized_subreddit() != sub_target {
            return false;
        }

        // 2. NSFW filter
        if sub.filter_nsfw && self.is_nsfw {
            return false;
        }

        // 3. Minimum score filter
        if sub.min_score > 0 && self.score < sub.min_score {
            return false;
        }

        // 4. Flair filter (comma-separated allowed list)
        if let Some(ref allowed_flairs_raw) = sub.flair_filter {
            let filter_str = allowed_flairs_raw.trim();
            if !filter_str.is_empty() {
                let post_flair = self.flair.as_deref().unwrap_or("").trim().to_lowercase();
                let mut matched = false;
                for allowed in filter_str.split(',') {
                    let cleaned = allowed.trim().to_lowercase();
                    if !cleaned.is_empty() && cleaned == post_flair {
                        matched = true;
                        break;
                    }
                }
                if !matched {
                    return false;
                }
            }
        }

        // 5. Keyword filter (comma-separated required keywords; matches title or selftext)
        if let Some(ref keywords_raw) = sub.keyword_filter {
            let filter_str = keywords_raw.trim();
            if !filter_str.is_empty() {
                let title_low = self.title.to_lowercase();
                let body_low = self.selftext.as_deref().unwrap_or("").to_lowercase();
                let mut matched = false;
                for kw in filter_str.split(',') {
                    let kw_clean = kw.trim().to_lowercase();
                    if !kw_clean.is_empty()
                        && (title_low.contains(&kw_clean) || body_low.contains(&kw_clean))
                    {
                        matched = true;
                        break;
                    }
                }
                if !matched {
                    return false;
                }
            }
        }

        true
    }
}

/// Clean a subreddit string by trimming whitespace, slashes, and `r/`.
pub fn clean_subreddit_name(input: &str) -> String {
    let mut s = input.trim();
    if let Some(stripped) = s.strip_prefix('/') {
        s = stripped;
    }
    if let Some(stripped) = s.strip_prefix("r/") {
        s = stripped;
    }
    if let Some(stripped) = s.strip_prefix("R/") {
        s = stripped;
    }
    if let Some(stripped) = s.strip_suffix('/') {
        s = stripped;
    }
    s.trim().to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_clean_subreddit_name() {
        assert_eq!(clean_subreddit_name("rust"), "rust");
        assert_eq!(clean_subreddit_name("r/rust"), "rust");
        assert_eq!(clean_subreddit_name("/r/rust/"), "rust");
        assert_eq!(clean_subreddit_name("R/Rust"), "rust");
        assert_eq!(clean_subreddit_name("  r/learnprogramming/  "), "learnprogramming");
    }

    #[test]
    fn test_reddit_post_matches_subscription() {
        let sub = RedditSubscription {
            id: 1,
            guild_id: 100,
            channel_id: 200,
            subreddit: "rust".to_string(),
            filter_nsfw: true,
            min_score: 5,
            flair_filter: Some("News,Release".to_string()),
            keyword_filter: Some("async,tokio".to_string()),
            created_at: 1000,
            last_seen_post_id: None,
        };

        // Matching post
        let valid_post = RedditPost {
            id: "t3_abc123".to_string(),
            subreddit: "r/rust".to_string(),
            title: "New Tokio Release available!".to_string(),
            author: "alice".to_string(),
            permalink: "/r/rust/comments/abc123/".to_string(),
            url: "https://reddit.com/r/rust/comments/abc123/".to_string(),
            selftext: Some("Check out the new async features.".to_string()),
            flair: Some("Release".to_string()),
            score: 10,
            is_nsfw: false,
            thumbnail_url: None,
            image_url: None,
            created_utc: 1500,
        };
        assert!(valid_post.matches_subscription(&sub));

        // NSFW rejected
        let mut nsfw_post = valid_post.clone();
        nsfw_post.is_nsfw = true;
        assert!(!nsfw_post.matches_subscription(&sub));

        // Low score rejected
        let mut low_score = valid_post.clone();
        low_score.score = 2;
        assert!(!low_score.matches_subscription(&sub));

        // Flair mismatch rejected
        let mut wrong_flair = valid_post.clone();
        wrong_flair.flair = Some("Question".to_string());
        assert!(!wrong_flair.matches_subscription(&sub));

        // Keyword mismatch rejected
        let mut wrong_kw = valid_post.clone();
        wrong_kw.title = "A general discussion".to_string();
        wrong_kw.selftext = Some("Nothing special here.".to_string());
        assert!(!wrong_kw.matches_subscription(&sub));
    }
}
