//! Human-friendly duration parser and normalizer for Silas Blue.
//!
//! Parses inputs like `"3600s"`, `"60m"`, `"1hr"`, `"2h"`, `"1d"`, `"1w"`, `"1d 2h"`,
//! and keywords like `"perm"`, `"permanent"`, `"inf"`, `"infinite"`, `"never"`.
//! Guarantees zero panics, overflow checks via checked math, and strict Discord timeout limits.

use serde::{Deserialize, Serialize};
use std::time::Duration;

pub const DISCORD_MAX_TIMEOUT_SECONDS: u64 = 28 * 24 * 3600; // 28 days

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ParsedDuration {
    Permanent,
    Finite(Duration),
}

impl ParsedDuration {
    #[inline]
    pub fn is_permanent(&self) -> bool {
        matches!(self, Self::Permanent)
    }

    #[inline]
    pub fn as_seconds(&self) -> Option<u64> {
        match self {
            Self::Permanent => None,
            Self::Finite(d) => Some(d.as_secs()),
        }
    }

    #[inline]
    pub fn from_seconds(secs: Option<u64>) -> Self {
        match secs {
            Some(s) => Self::Finite(Duration::from_secs(s)),
            None => Self::Permanent,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DurationError {
    #[error("duration cannot be empty")]
    Empty,
    #[error("negative duration is not allowed")]
    Negative,
    #[error("zero duration is not allowed in this context")]
    ZeroNotAllowed,
    #[error("duration exceeds Discord timeout limit of 28 days")]
    ExceedsDiscordTimeoutLimit,
    #[error("duration value overflowed")]
    Overflow,
    #[error("unsupported duration unit '{0}', please use s, m, h, d, or w")]
    UnsupportedUnit(String),
    #[error("invalid duration format '{0}'")]
    InvalidFormat(String),
}

/// Parses a human duration string into `ParsedDuration`.
pub fn parse_duration(input: &str) -> Result<ParsedDuration, DurationError> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err(DurationError::Empty);
    }

    if trimmed.contains('-') {
        return Err(DurationError::Negative);
    }

    let lower = trimmed.to_lowercase();
    match lower.as_str() {
        "perm" | "permanent" | "inf" | "infinite" | "never" => {
            return Ok(ParsedDuration::Permanent);
        }
        "0" | "0s" | "0sec" | "0m" | "0min" | "0h" | "0d" | "0w" => {
            return Ok(ParsedDuration::Finite(Duration::ZERO));
        }
        _ => {}
    }

    // Parse tokens: collect numeric sequences and trailing unit letters
    let mut total_secs: u64 = 0;
    let mut chars = lower.chars().peekable();
    let mut tokens_found = 0;

    while chars.peek().is_some() {
        // Skip whitespace
        while let Some(&c) = chars.peek() {
            if c.is_whitespace() || c == ',' {
                let _ = chars.next();
            } else {
                break;
            }
        }

        if chars.peek().is_none() {
            break;
        }

        // Collect digits
        let mut num_str = String::new();
        while let Some(&c) = chars.peek() {
            if c.is_ascii_digit() {
                num_str.push(c);
                let _ = chars.next();
            } else {
                break;
            }
        }

        if num_str.is_empty() {
            return Err(DurationError::InvalidFormat(trimmed.to_string()));
        }

        let num: u64 = num_str
            .parse::<u64>()
            .map_err(|_| DurationError::Overflow)?;

        // Skip whitespace between number and unit
        while let Some(&c) = chars.peek() {
            if c.is_whitespace() {
                let _ = chars.next();
            } else {
                break;
            }
        }

        // Collect unit letters
        let mut unit_str = String::new();
        while let Some(&c) = chars.peek() {
            if c.is_ascii_alphabetic() {
                unit_str.push(c);
                let _ = chars.next();
            } else {
                break;
            }
        }

        let multiplier: u64 = match unit_str.as_str() {
            "" | "s" | "sec" | "secs" | "second" | "seconds" => 1,
            "m" | "min" | "mins" | "minute" | "minutes" => 60,
            "h" | "hr" | "hrs" | "hour" | "hours" => 3600,
            "d" | "day" | "days" => 86400,
            "w" | "wk" | "wks" | "week" | "weeks" => 604800,
            "mo" | "mon" | "month" | "months" | "y" | "yr" | "yrs" | "year" | "years" => {
                return Err(DurationError::UnsupportedUnit(unit_str));
            }
            _ => {
                return Err(DurationError::UnsupportedUnit(unit_str));
            }
        };

        let token_secs = num
            .checked_mul(multiplier)
            .ok_or(DurationError::Overflow)?;

        total_secs = total_secs
            .checked_add(token_secs)
            .ok_or(DurationError::Overflow)?;

        tokens_found += 1;
    }

    if tokens_found == 0 {
        return Err(DurationError::InvalidFormat(trimmed.to_string()));
    }

    Ok(ParsedDuration::Finite(Duration::from_secs(total_secs)))
}

/// Validates that a duration is valid for a Discord timeout (1s to 28 days).
pub fn validate_timeout_duration(dur: &ParsedDuration) -> Result<Duration, DurationError> {
    match dur {
        ParsedDuration::Permanent => Err(DurationError::ExceedsDiscordTimeoutLimit),
        ParsedDuration::Finite(d) => {
            let secs = d.as_secs();
            if secs == 0 {
                return Err(DurationError::ZeroNotAllowed);
            }
            if secs > DISCORD_MAX_TIMEOUT_SECONDS {
                return Err(DurationError::ExceedsDiscordTimeoutLimit);
            }
            Ok(*d)
        }
    }
}

/// Formats a `ParsedDuration` into normalized human-readable English.
pub fn format_duration(dur: &ParsedDuration) -> String {
    match dur {
        ParsedDuration::Permanent => "Permanent".to_string(),
        ParsedDuration::Finite(d) => {
            let mut secs = d.as_secs();
            if secs == 0 {
                return "0 seconds".to_string();
            }

            let weeks = secs / 604800;
            secs %= 604800;
            let days = secs / 86400;
            secs %= 86400;
            let hours = secs / 3600;
            secs %= 3600;
            let minutes = secs / 60;
            secs %= 60;

            let mut parts = Vec::new();
            if weeks > 0 {
                parts.push(format!("{weeks} {}", if weeks == 1 { "week" } else { "weeks" }));
            }
            if days > 0 {
                parts.push(format!("{days} {}", if days == 1 { "day" } else { "days" }));
            }
            if hours > 0 {
                parts.push(format!("{hours} {}", if hours == 1 { "hour" } else { "hours" }));
            }
            if minutes > 0 {
                parts.push(format!("{minutes} {}", if minutes == 1 { "minute" } else { "minutes" }));
            }
            if secs > 0 {
                parts.push(format!("{secs} {}", if secs == 1 { "second" } else { "seconds" }));
            }

            parts.join(", ")
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_permanent() {
        assert_eq!(parse_duration("perm").unwrap(), ParsedDuration::Permanent);
        assert_eq!(parse_duration("permanent").unwrap(), ParsedDuration::Permanent);
        assert_eq!(parse_duration("inf").unwrap(), ParsedDuration::Permanent);
        assert_eq!(parse_duration("infinite").unwrap(), ParsedDuration::Permanent);
        assert_eq!(parse_duration("never").unwrap(), ParsedDuration::Permanent);
        assert_eq!(parse_duration("  PERM  ").unwrap(), ParsedDuration::Permanent);
    }

    #[test]
    fn test_parse_zero() {
        assert_eq!(
            parse_duration("0").unwrap(),
            ParsedDuration::Finite(Duration::ZERO)
        );
        assert_eq!(
            parse_duration("0s").unwrap(),
            ParsedDuration::Finite(Duration::ZERO)
        );
    }

    #[test]
    fn test_parse_simple_units() {
        assert_eq!(
            parse_duration("3600s").unwrap(),
            ParsedDuration::Finite(Duration::from_secs(3600))
        );
        assert_eq!(
            parse_duration("60m").unwrap(),
            ParsedDuration::Finite(Duration::from_secs(3600))
        );
        assert_eq!(
            parse_duration("1hr").unwrap(),
            ParsedDuration::Finite(Duration::from_secs(3600))
        );
        assert_eq!(
            parse_duration("2h").unwrap(),
            ParsedDuration::Finite(Duration::from_secs(7200))
        );
        assert_eq!(
            parse_duration("1d").unwrap(),
            ParsedDuration::Finite(Duration::from_secs(86400))
        );
        assert_eq!(
            parse_duration("1w").unwrap(),
            ParsedDuration::Finite(Duration::from_secs(604800))
        );
    }

    #[test]
    fn test_parse_combinations() {
        assert_eq!(
            parse_duration("1d 2h").unwrap(),
            ParsedDuration::Finite(Duration::from_secs(86400 + 7200))
        );
        assert_eq!(
            parse_duration("2h, 30m, 15s").unwrap(),
            ParsedDuration::Finite(Duration::from_secs(7200 + 1800 + 15))
        );
    }

    #[test]
    fn test_parse_errors() {
        assert_eq!(parse_duration(""), Err(DurationError::Empty));
        assert_eq!(parse_duration("   "), Err(DurationError::Empty));
        assert_eq!(parse_duration("-5m"), Err(DurationError::Negative));
        assert!(matches!(
            parse_duration("1year"),
            Err(DurationError::UnsupportedUnit(_))
        ));
        assert!(matches!(
            parse_duration("invalid"),
            Err(DurationError::InvalidFormat(_))
        ));
    }

    #[test]
    fn test_validate_timeout() {
        let valid = ParsedDuration::Finite(Duration::from_secs(3600));
        assert_eq!(validate_timeout_duration(&valid).unwrap(), Duration::from_secs(3600));

        let zero = ParsedDuration::Finite(Duration::ZERO);
        assert_eq!(validate_timeout_duration(&zero), Err(DurationError::ZeroNotAllowed));

        let perm = ParsedDuration::Permanent;
        assert_eq!(validate_timeout_duration(&perm), Err(DurationError::ExceedsDiscordTimeoutLimit));

        let too_long = ParsedDuration::Finite(Duration::from_secs(29 * 86400));
        assert_eq!(validate_timeout_duration(&too_long), Err(DurationError::ExceedsDiscordTimeoutLimit));
    }

    #[test]
    fn test_formatting() {
        assert_eq!(format_duration(&ParsedDuration::Permanent), "Permanent");
        assert_eq!(
            format_duration(&ParsedDuration::Finite(Duration::from_secs(3600))),
            "1 hour"
        );
        assert_eq!(
            format_duration(&ParsedDuration::Finite(Duration::from_secs(86400 + 7200))),
            "1 day, 2 hours"
        );
    }
}
