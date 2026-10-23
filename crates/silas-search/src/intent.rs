//! Precision auto-search intent detection and context-aware query synthesis
//! powered by single-pass pre-compiled `LazyLock<RegexSet>` DFAs.

use std::collections::HashSet;
use std::sync::LazyLock;

use regex::{Regex, RegexSet};
use silas_core::memory::ChatMessage;
use silas_core::text::char_count;

#[allow(clippy::expect_used)]
static NON_SEARCH_SET: LazyLock<RegexSet> = LazyLock::new(|| {
    RegexSet::new([
        r"(?i)^(hi|hello|hey|greetings|howdy|yo|good morning|good evening|good afternoon)\b",
        r"(?i)\b(write a poem|write a song|write a story|tell me a joke|compose a)\b",
        r"(?i)^(?:write|create|generate|code|debug)\s+(?:a\s+)?(?:rust|python|javascript|script|code|program|function|class|algorithm)\b",
        r"(?i)\b(repeat after me|translate this|ignore previous instructions)\b",
        r"(?i)^(ping|test|echo)\b",
        r"(?i)\b(?:shut\s*down|turn\s+off|power\s+off|reboot|restart)\s+(?:your|the|this)?\s*(?:computer|pc|system|server|machine|host|bot)\b",
        r"(?i)^now\s+(?:that|then|listen|look|what|why|how|tell|let|i|we|you)\b",
        r"(?i)\bnow\s+that\s+you\b",
        r"(?i)\b(?:from\s+now\s+on|for\s+now|until\s+now|by\s+now|just\s+now)\b",
        r"(?i)^(?:why|how|what)\s+(?:did\s+you|are\s+you|were\s+those|made\s+you)\s+(?:show|give|send|link|search|say|mention)\b",
        r"(?i)\bwhy\s+did\s+you\s+(?:show|give|send|link|search|say)\b",
        r"(?i)^(?:how\s+are\s+you|how\s+do\s+you\s+feel|how's\s+it\s+going|what\s+are\s+you\s+doing|what\s+can\s+you\s+do)\b",
        r"(?i)^(?:who\s+are\s+you|what\s+is\s+your\s+name|introduce\s+yourself|tell\s+me\s+about\s+yourself)\b",
    ])
    .expect("valid NON_SEARCH_SET regexes")
});

#[allow(clippy::expect_used)]
static LOOKUP_SET: LazyLock<RegexSet> = LazyLock::new(|| {
    RegexSet::new([
        r"(?i)^(?:what|who|when|where|how|which)\b\s+(?:is|was|are|were|happened|occurred|announced|released|won|leads|plays)\b.*(?:today|tonight|recently|current|latest|this week|202[4-9]|203[0-9])",
        r"(?i)\b(news|headline|headlines|breaking news|patch notes|changelog)\b",
        r"(?i)\b(release date|released|specs|specifications|price of|price for|weather in|score of|who won)\b",
        r"(?i)\b(search the web|search for|google|look up|lookup|find info on|browse for|check online)\b",
        r"(?i)\b(part number|part numbers|model number|model numbers|sku|skus|datasheet|datasheets|pinout|pinouts)\b",
        r"(?i)\b(benchmark|benchmarks|cost of|how much is|where to buy|msrp|in stock)\b",
        r"(?i)\b(to search|can you search|can you look up|can you find|give me a part number|give me a model|give me the model|give me the part)\b",
        r"(?i)\brecent\s+[A-Za-z0-9]+\s+(?:launch|flight|mission|test|event|game|update)\b",
        r"(?i)\bmost\s+recent\b",
    ])
    .expect("valid LOOKUP_SET regexes")
});

#[allow(clippy::expect_used)]
static TEMPORAL_SET: LazyLock<RegexSet> = LazyLock::new(|| {
    RegexSet::new([
        r"(?i)\b(right now|as of (?:today|now)|at the moment|happening now|currently happening)\b",
        r"(?i)\b(?:latest|current|recent|newest|upcoming)\s+(?:news|updates?|version|release|patch|price|score|events?|info|information|specs?|status|data|stats?|trends?|developments?)\b",
        r"(?i)\b(?:today|tonight|yesterday|tomorrow)(?:'s)?\s+(?:news|weather|scores?|games?|stock|market|events?|headlines?|updates?|matches?|results?)\b",
        r"(?i)\b(?:news|weather|scores?|price|stock|events?|matches?|results?)\s+(?:today|tonight|yesterday|tomorrow|this week|this month|this year)\b",
        r"(?i)\b(?:who won|what happened|results? for|schedule for|winner of|champion of)\s+(?:today|tonight|yesterday|this week|202[4-9]|203[0-9])\b",
        r"(?i)\b(?:released?|announced?|unveiled?|launched?)\s+(?:today|yesterday|recently|in 202[4-9]|in 203[0-9])\b",
        r"(?i)\b202[4-9]\s+(?:news|release|specs|features|election|championship|winner|schedule|update|games?|movies?)\b",
        r"(?i)^(?:what|who|when|where|how|which)\b.*\b202[4-9]\b.*\?",
    ])
    .expect("valid TEMPORAL_SET regexes")
});

#[allow(clippy::expect_used)]
static FILLER_PREFIX_1: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)^(?:no\s*,?\s*)?(?:please\s+)?(?:can\s+you\s+|could\s+you\s+|i\s+need\s+you\s+to\s+|i\s+need\s+|give\s+me\s+|tell\s+me\s+|show\s+me\s+|what\s+is\s+|what\s+are\s+)+")
        .expect("valid filler prefix 1")
});

#[allow(clippy::expect_used)]
static FILLER_PREFIX_2: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)^[!?/]*(?:search\s+(?:the\s+web\s+for|the\s+internet\s+for|for|online\s+for)?|google|look\s+up|find\s+info\s+on|browse\s+for)\s+")
        .expect("valid filler prefix 2")
});

#[allow(clippy::expect_used)]
static FILLER_SUFFIX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\s+to\s+(?:search|look\s+up|google|find)\.?$").expect("valid filler suffix")
});

#[allow(clippy::expect_used)]
static WORD_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\b\w+\b").expect("valid word regex"));

#[allow(clippy::expect_used)]
static SUBJECT_TOKEN_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\b[A-Za-z0-9]+(?:[-_./+][A-Za-z0-9]+)*\b").expect("valid subject token regex")
});

#[allow(clippy::expect_used)]
static ARTICLE_PREFIX_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)^(?:a|an|the)\s+").expect("valid article prefix regex"));

static GENERIC_LOOKUP_WORDS: LazyLock<HashSet<&'static str>> = LazyLock::new(|| {
    HashSet::from([
        "part",
        "number",
        "model",
        "sku",
        "skus",
        "datasheet",
        "specs",
        "specifications",
        "price",
        "cost",
        "link",
        "url",
        "pinout",
        "search",
        "where",
        "buy",
        "online",
        "details",
        "info",
        "information",
        "a",
        "an",
        "the",
        "me",
        "to",
        "for",
        "give",
        "need",
        "you",
    ])
});

static STOP_WORDS: LazyLock<HashSet<&'static str>> = LazyLock::new(|| {
    HashSet::from([
        "page",
        "the",
        "and",
        "or",
        "for",
        "with",
        "this",
        "that",
        "you",
        "your",
        "silas",
        "blue",
        "replying",
        "response",
        "summary",
        "checklist",
        "step",
        "recommended",
        "solution",
        "upgrade",
        "table",
        "need",
        "give",
        "can",
        "will",
        "about",
        "from",
        "are",
        "not",
        "have",
        "been",
        "would",
        "like",
        "using",
        "what",
        "when",
        "where",
        "which",
        "there",
        "here",
        "also",
        "just",
        "more",
    ])
});

/// Evaluates whether a user prompt has real-time or factual web lookup intent.
pub fn should_auto_search(prompt: &str) -> bool {
    let trimmed = prompt.trim();
    if char_count(trimmed) < 4 {
        return false;
    }
    if NON_SEARCH_SET.is_match(trimmed) {
        return false;
    }
    LOOKUP_SET.is_match(trimmed) || TEMPORAL_SET.is_match(trimmed)
}

pub fn clean_query_text(text: &str) -> String {
    let mut cleaned = text.trim().to_owned();
    cleaned = FILLER_PREFIX_1.replace(&cleaned, "").trim().to_owned();
    cleaned = FILLER_PREFIX_2.replace(&cleaned, "").trim().to_owned();
    cleaned = FILLER_SUFFIX.replace(&cleaned, "").trim().to_owned();
    cleaned
        .trim_matches(&['?', '!', '.', ',', ';', ':'][..])
        .trim()
        .to_owned()
}

pub fn extract_context_keywords(context_text: &str) -> String {
    if context_text.trim().is_empty() {
        return String::new();
    }
    let mut seen = HashSet::new();
    let mut subject_tokens = Vec::with_capacity(8);

    for m in SUBJECT_TOKEN_RE.find_iter(context_text) {
        let tok = m.as_str();
        let low = tok.to_ascii_lowercase();
        if STOP_WORDS.contains(low.as_str()) || char_count(tok) <= 1 || tok.chars().all(|c| c.is_ascii_digit()) {
            continue;
        }
        let has_digit = tok.chars().any(|c| c.is_ascii_digit());
        let has_symbol = tok.contains('-') || tok.contains('+');
        let is_all_upper = tok.chars().all(|c| !c.is_alphabetic() || c.is_uppercase());
        let is_title = tok.chars().next().is_some_and(char::is_uppercase) && char_count(tok) > 3;

        if (has_digit || has_symbol || is_all_upper || is_title) && seen.insert(low) {
            subject_tokens.push(tok.to_owned());
            if subject_tokens.len() >= 8 {
                break;
            }
        }
    }
    subject_tokens.join(" ")
}

/// Constructs an optimized search query, enriching vague follow-up prompts
/// (e.g. `"give me a part number"`) with contextual subject tokens from the conversation.
pub fn build_search_query(
    raw_prompt: &str,
    replied_context: &str,
    history: &[ChatMessage],
) -> String {
    let cleaned = clean_query_text(raw_prompt);
    let words: Vec<String> = WORD_RE
        .find_iter(&cleaned)
        .map(|m| m.as_str().to_ascii_lowercase())
        .collect();

    let is_vague = words.len() <= 3
        || words
            .iter()
            .all(|w| GENERIC_LOOKUP_WORDS.contains(w.as_str()));

    if !is_vague {
        return if cleaned.is_empty() {
            raw_prompt.trim().to_owned()
        } else {
            cleaned
        };
    }

    let mut context_subject = extract_context_keywords(replied_context);
    if context_subject.is_empty() {
        for msg in history.iter().rev() {
            if msg.role == "user" {
                context_subject = extract_context_keywords(&msg.content);
                if !context_subject.is_empty() {
                    break;
                }
            }
        }
    }
    if context_subject.is_empty() {
        for msg in history.iter().rev() {
            if msg.role == "assistant" {
                context_subject = extract_context_keywords(&msg.content);
                if !context_subject.is_empty() {
                    break;
                }
            }
        }
    }

    if !context_subject.is_empty() {
        let suffix = ARTICLE_PREFIX_RE.replace(&cleaned, "").trim().to_owned();
        return format!("{context_subject} {suffix}").trim().to_owned();
    }

    if cleaned.is_empty() {
        raw_prompt.trim().to_owned()
    } else {
        cleaned
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn test_intent_classifier_rejects_false_positives_and_matches_real_queries() {
        assert!(!should_auto_search("Hello there!"));
        assert!(!should_auto_search("Now that you mention it, I agree."));
        assert!(!should_auto_search("From now on, speak like a knight."));
        assert!(!should_auto_search("Shutdown your computer right now"));
        assert!(!should_auto_search("Write a python function to sort a list"));

        assert!(should_auto_search("What is the latest news on Rust 2026?"));
        assert!(should_auto_search("Give me the part number for Mellanox ConnectX-3"));
        assert!(should_auto_search("What is the current price of RTX 5090?"));
    }

    #[test]
    fn test_context_query_enrichment() {
        let q = build_search_query(
            "Give me a part number to search",
            "We recommend the Mellanox ConnectX-3 10GbE SFP+ PCIe 3.0 card.",
            &[],
        );
        assert!(q.contains("Mellanox"));
        assert!(q.contains("ConnectX-3"));
        assert!(q.contains("part number"));
    }
}
