//! Human-readable duration parsing and serialization.
//!
//! Provides utilities for converting between `std::time::Duration` and human-readable
//! string formats for use in YAML configuration files.
//!
//! # Supported Formats
//!
//! - Seconds: "30s", "5s"
//! - Minutes: "5m", "90m"
//! - Hours: "2h", "24h"
//! - Days: "7d", "30d"
//! - Combined: "1h30m", "2h30m45s", "1d12h"
//!
//! # Examples
//!
//! ```
//! use petit::core::duration::parse_duration;
//! use std::time::Duration;
//!
//! assert_eq!(parse_duration("30s").unwrap(), Duration::from_secs(30));
//! assert_eq!(parse_duration("5m").unwrap(), Duration::from_secs(300));
//! assert_eq!(parse_duration("1h").unwrap(), Duration::from_secs(3600));
//! assert_eq!(parse_duration("1h30m").unwrap(), Duration::from_secs(5400));
//! assert_eq!(parse_duration("2d").unwrap(), Duration::from_secs(172800));
//! ```

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::time::Duration;
use thiserror::Error;

/// Errors that can occur when parsing durations.
#[derive(Debug, Error, PartialEq)]
pub enum DurationParseError {
    /// Empty duration string.
    #[error("empty duration string")]
    Empty,

    /// Invalid format.
    #[error("invalid duration format: {0}")]
    InvalidFormat(String),

    /// Invalid numeric value.
    #[error("invalid numeric value: {0}")]
    InvalidNumber(String),

    /// Unknown unit.
    #[error("unknown time unit: {0}")]
    UnknownUnit(String),
}

/// Parse a human-readable duration string into a `Duration`.
///
/// Supports the following units:
/// - `s` - seconds
/// - `m` - minutes
/// - `h` - hours
/// - `d` - days
///
/// Units can be combined (e.g., "1h30m", "2d12h30m").
///
/// # Examples
///
/// ```
/// use petit::core::duration::parse_duration;
/// use std::time::Duration;
///
/// assert_eq!(parse_duration("30s").unwrap(), Duration::from_secs(30));
/// assert_eq!(parse_duration("1h30m").unwrap(), Duration::from_secs(5400));
/// ```
pub fn parse_duration(s: &str) -> Result<Duration, DurationParseError> {
    if s.is_empty() {
        return Err(DurationParseError::Empty);
    }

    let mut total_secs = 0u64;
    let mut current_num = String::new();

    for ch in s.chars() {
        if ch.is_ascii_digit() {
            current_num.push(ch);
        } else if ch.is_alphabetic() {
            if current_num.is_empty() {
                return Err(DurationParseError::InvalidFormat(
                    "unit without preceding number".into(),
                ));
            }

            let num: u64 = current_num
                .parse()
                .map_err(|_| DurationParseError::InvalidNumber(current_num.clone()))?;

            let multiplier = match ch {
                's' => 1,
                'm' => 60,
                'h' => 3600,
                'd' => 86400,
                _ => return Err(DurationParseError::UnknownUnit(ch.to_string())),
            };

            total_secs += num * multiplier;
            current_num.clear();
        } else if ch.is_whitespace() {
            // Skip whitespace
            continue;
        } else {
            return Err(DurationParseError::InvalidFormat(format!(
                "unexpected character: {}",
                ch
            )));
        }
    }

    if !current_num.is_empty() {
        return Err(DurationParseError::InvalidFormat(
            "number without unit".into(),
        ));
    }

    Ok(Duration::from_secs(total_secs))
}

/// Format a `Duration` as a human-readable string.
///
/// Returns the most compact representation possible, using the largest
/// units that evenly divide the duration.
///
/// # Examples
///
/// ```
/// use petit::core::duration::format_duration;
/// use std::time::Duration;
///
/// assert_eq!(format_duration(Duration::from_secs(30)), "30s");
/// assert_eq!(format_duration(Duration::from_secs(300)), "5m");
/// assert_eq!(format_duration(Duration::from_secs(5400)), "1h30m");
/// ```
pub fn format_duration(duration: Duration) -> String {
    let total_secs = duration.as_secs();

    if total_secs == 0 {
        return "0s".to_string();
    }

    let mut result = String::new();
    let mut remaining = total_secs;

    // Days
    if remaining >= 86400 {
        let days = remaining / 86400;
        result.push_str(&format!("{}d", days));
        remaining %= 86400;
    }

    // Hours
    if remaining >= 3600 {
        let hours = remaining / 3600;
        result.push_str(&format!("{}h", hours));
        remaining %= 3600;
    }

    // Minutes
    if remaining >= 60 {
        let minutes = remaining / 60;
        result.push_str(&format!("{}m", minutes));
        remaining %= 60;
    }

    // Seconds
    if remaining > 0 {
        result.push_str(&format!("{}s", remaining));
    }

    result
}

/// Serialize a `Duration` as a human-readable string.
///
/// This function is intended for use with serde's `serialize_with` attribute.
pub fn serialize<S>(duration: &Duration, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    format_duration(*duration).serialize(serializer)
}

/// Deserialize a `Duration` from either a human-readable string or a number of seconds.
///
/// This function is intended for use with serde's `deserialize_with` attribute.
/// It accepts both string formats (e.g., "1h30m") and numeric seconds for backwards compatibility.
pub fn deserialize<'de, D>(deserializer: D) -> Result<Duration, D::Error>
where
    D: Deserializer<'de>,
{
    use serde::de::Error;

    #[derive(Deserialize)]
    #[serde(untagged)]
    enum DurationInput {
        String(String),
        Seconds(u64),
    }

    match DurationInput::deserialize(deserializer)? {
        DurationInput::String(s) => {
            parse_duration(&s).map_err(|e| D::Error::custom(format!("invalid duration: {}", e)))
        }
        DurationInput::Seconds(secs) => Ok(Duration::from_secs(secs)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_seconds() {
        assert_eq!(parse_duration("0s").unwrap(), Duration::from_secs(0));
        assert_eq!(parse_duration("30s").unwrap(), Duration::from_secs(30));
        assert_eq!(parse_duration("5400s").unwrap(), Duration::from_secs(5400));
    }

    #[test]
    fn test_parse_minutes() {
        assert_eq!(parse_duration("1m").unwrap(), Duration::from_secs(60));
        assert_eq!(parse_duration("5m").unwrap(), Duration::from_secs(300));
        assert_eq!(parse_duration("90m").unwrap(), Duration::from_secs(5400));
    }

    #[test]
    fn test_parse_hours() {
        assert_eq!(parse_duration("1h").unwrap(), Duration::from_secs(3600));
        assert_eq!(parse_duration("2h").unwrap(), Duration::from_secs(7200));
        assert_eq!(parse_duration("24h").unwrap(), Duration::from_secs(86400));
    }

    #[test]
    fn test_parse_days() {
        assert_eq!(parse_duration("1d").unwrap(), Duration::from_secs(86400));
        assert_eq!(parse_duration("7d").unwrap(), Duration::from_secs(604800));
        assert_eq!(parse_duration("30d").unwrap(), Duration::from_secs(2592000));
    }

    #[test]
    fn test_parse_combined() {
        assert_eq!(parse_duration("1h30m").unwrap(), Duration::from_secs(5400));
        assert_eq!(
            parse_duration("2h30m45s").unwrap(),
            Duration::from_secs(9045)
        );
        assert_eq!(
            parse_duration("1d12h").unwrap(),
            Duration::from_secs(129600)
        );
        assert_eq!(
            parse_duration("1d2h3m4s").unwrap(),
            Duration::from_secs(93784)
        );
    }

    #[test]
    fn test_parse_with_whitespace() {
        assert_eq!(parse_duration("1h 30m").unwrap(), Duration::from_secs(5400));
        assert_eq!(
            parse_duration("2d 12h 30m").unwrap(),
            Duration::from_secs(217800)
        );
    }

    #[test]
    fn test_parse_empty() {
        assert_eq!(parse_duration(""), Err(DurationParseError::Empty));
    }

    #[test]
    fn test_parse_invalid_format() {
        assert!(parse_duration("h").is_err());
        assert!(parse_duration("30").is_err());
        assert!(parse_duration("30x").is_err());
        assert!(parse_duration("1h30").is_err());
    }

    #[test]
    fn test_format_seconds() {
        assert_eq!(format_duration(Duration::from_secs(0)), "0s");
        assert_eq!(format_duration(Duration::from_secs(30)), "30s");
        assert_eq!(format_duration(Duration::from_secs(59)), "59s");
    }

    #[test]
    fn test_format_minutes() {
        assert_eq!(format_duration(Duration::from_secs(60)), "1m");
        assert_eq!(format_duration(Duration::from_secs(300)), "5m");
        assert_eq!(format_duration(Duration::from_secs(3540)), "59m");
    }

    #[test]
    fn test_format_hours() {
        assert_eq!(format_duration(Duration::from_secs(3600)), "1h");
        assert_eq!(format_duration(Duration::from_secs(7200)), "2h");
    }

    #[test]
    fn test_format_days() {
        assert_eq!(format_duration(Duration::from_secs(86400)), "1d");
        assert_eq!(format_duration(Duration::from_secs(604800)), "7d");
    }

    #[test]
    fn test_format_combined() {
        assert_eq!(format_duration(Duration::from_secs(5400)), "1h30m");
        assert_eq!(format_duration(Duration::from_secs(9045)), "2h30m45s");
        assert_eq!(format_duration(Duration::from_secs(129600)), "1d12h");
        assert_eq!(format_duration(Duration::from_secs(93784)), "1d2h3m4s");
    }

    #[test]
    fn test_roundtrip() {
        let durations = vec![
            Duration::from_secs(30),
            Duration::from_secs(300),
            Duration::from_secs(3600),
            Duration::from_secs(5400),
            Duration::from_secs(86400),
            Duration::from_secs(93784),
        ];

        for duration in durations {
            let formatted = format_duration(duration);
            let parsed = parse_duration(&formatted).unwrap();
            assert_eq!(parsed, duration);
        }
    }

    #[test]
    fn test_serde_json_string() {
        use serde::{Deserialize, Serialize};

        #[derive(Debug, PartialEq, Serialize, Deserialize)]
        struct Test {
            #[serde(with = "super")]
            duration: Duration,
        }

        // Serialize
        let test = Test {
            duration: Duration::from_secs(5400),
        };
        let json = serde_json::to_string(&test).unwrap();
        assert_eq!(json, r#"{"duration":"1h30m"}"#);

        // Deserialize from string
        let parsed: Test = serde_json::from_str(r#"{"duration":"1h30m"}"#).unwrap();
        assert_eq!(parsed.duration, Duration::from_secs(5400));
    }

    #[test]
    fn test_serde_json_number() {
        use serde::Deserialize;

        #[derive(Debug, PartialEq, Deserialize)]
        struct Test {
            #[serde(with = "super")]
            duration: Duration,
        }

        // Deserialize from number (backwards compatibility)
        let parsed: Test = serde_json::from_str(r#"{"duration":5400}"#).unwrap();
        assert_eq!(parsed.duration, Duration::from_secs(5400));
    }

    #[test]
    fn test_serde_yaml() {
        use serde::{Deserialize, Serialize};

        #[derive(Debug, PartialEq, Serialize, Deserialize)]
        struct Test {
            #[serde(with = "super")]
            duration: Duration,
        }

        // Serialize
        let test = Test {
            duration: Duration::from_secs(5400),
        };
        let yaml = serde_yaml::to_string(&test).unwrap();
        assert!(yaml.contains("1h30m"));

        // Deserialize from string
        let yaml = "duration: 1h30m\n";
        let parsed: Test = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(parsed.duration, Duration::from_secs(5400));

        // Deserialize from number (backwards compatibility)
        let yaml = "duration: 5400\n";
        let parsed: Test = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(parsed.duration, Duration::from_secs(5400));
    }

    #[test]
    fn test_serde_invalid() {
        use serde::Deserialize;

        #[derive(Debug, Deserialize)]
        #[allow(dead_code)]
        struct Test {
            #[serde(with = "super")]
            duration: Duration,
        }

        let result: Result<Test, _> = serde_json::from_str(r#"{"duration":"invalid"}"#);
        assert!(result.is_err());
    }
}
