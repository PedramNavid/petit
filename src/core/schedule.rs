//! Schedule parsing and next occurrence calculation.
//!
//! Supports standard cron expressions, extended 6-field cron (with seconds),
//! shortcuts (@daily, @hourly, etc.), and interval expressions (@every).

use chrono::{DateTime, Utc};
use chrono_tz::Tz;
use cron::Schedule as CronSchedule;
use serde::{Deserialize, Serialize};
use std::str::FromStr;
use thiserror::Error;

/// Errors that can occur when parsing or using schedules.
#[derive(Debug, Error)]
pub enum ScheduleError {
    /// Invalid cron expression.
    #[error("invalid cron expression: {0}")]
    InvalidCron(String),

    /// Invalid interval expression.
    #[error("invalid interval expression: {0}")]
    InvalidInterval(String),

    /// Invalid timezone.
    #[error("invalid timezone: {0}")]
    InvalidTimezone(String),

    /// No more occurrences.
    #[error("no more occurrences")]
    NoMoreOccurrences,
}

/// A schedule for job execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Schedule {
    /// The original expression string.
    expression: String,
    /// The timezone for this schedule.
    timezone: String,
    /// Parsed schedule type.
    #[serde(skip)]
    schedule_type: ScheduleType,
}

#[derive(Debug, Clone, Default)]
enum ScheduleType {
    /// Standard cron schedule.
    Cron(Box<CronSchedule>),
    /// Interval-based schedule (e.g., @every 5m).
    Interval(std::time::Duration),
    /// Not yet parsed.
    #[default]
    Unparsed,
}

impl Schedule {
    /// Create a new schedule from a cron expression.
    ///
    /// Supports:
    /// - Standard 5-field cron: `minute hour day month weekday`
    /// - Extended 6-field cron: `second minute hour day month weekday`
    /// - Shortcuts: `@yearly`, `@monthly`, `@weekly`, `@daily`, `@hourly`
    /// - Intervals: `@every 5m`, `@every 1h30m`
    pub fn new(expression: impl Into<String>) -> Result<Self, ScheduleError> {
        Self::with_timezone(expression, "UTC")
    }

    /// Create a new schedule with a specific timezone.
    pub fn with_timezone(
        expression: impl Into<String>,
        timezone: impl Into<String>,
    ) -> Result<Self, ScheduleError> {
        let expression = expression.into();
        let timezone = timezone.into();

        // Validate timezone
        timezone
            .parse::<Tz>()
            .map_err(|_| ScheduleError::InvalidTimezone(timezone.clone()))?;

        let schedule_type = Self::parse_expression(&expression)?;

        Ok(Self {
            expression,
            timezone,
            schedule_type,
        })
    }

    /// Parse the expression into a schedule type.
    fn parse_expression(expression: &str) -> Result<ScheduleType, ScheduleError> {
        let trimmed = expression.trim();

        // Handle shortcuts
        if trimmed.starts_with('@') {
            return Self::parse_shortcut(trimmed);
        }

        // Try to parse as cron expression
        Self::parse_cron(trimmed)
    }

    /// Parse a shortcut expression (@daily, @every, etc.).
    fn parse_shortcut(expression: &str) -> Result<ScheduleType, ScheduleError> {
        match expression.to_lowercase().as_str() {
            "@yearly" | "@annually" => Self::parse_cron("0 0 1 1 *"),
            "@monthly" => Self::parse_cron("0 0 1 * *"),
            "@weekly" => Self::parse_cron("0 0 * * SUN"),
            "@daily" | "@midnight" => Self::parse_cron("0 0 * * *"),
            "@hourly" => Self::parse_cron("0 * * * *"),
            s if s.starts_with("@every ") => Self::parse_interval(&s[7..]),
            _ => Err(ScheduleError::InvalidCron(format!(
                "unknown shortcut: {}",
                expression
            ))),
        }
    }

    /// Parse an interval expression (e.g., "5m", "1h30m").
    fn parse_interval(interval: &str) -> Result<ScheduleType, ScheduleError> {
        let trimmed = interval.trim();
        let duration = Self::parse_duration(trimmed)?;
        Ok(ScheduleType::Interval(duration))
    }

    /// Parse a duration string like "5m", "1h", "1h30m", "30s".
    fn parse_duration(s: &str) -> Result<std::time::Duration, ScheduleError> {
        let mut total_secs: u64 = 0;
        let mut current_num = String::new();

        for c in s.chars() {
            if c.is_ascii_digit() {
                current_num.push(c);
            } else {
                let num: u64 = current_num
                    .parse()
                    .map_err(|_| ScheduleError::InvalidInterval(s.to_string()))?;
                current_num.clear();

                match c {
                    's' => total_secs += num,
                    'm' => total_secs += num * 60,
                    'h' => total_secs += num * 3600,
                    'd' => total_secs += num * 86400,
                    _ => return Err(ScheduleError::InvalidInterval(s.to_string())),
                }
            }
        }

        if total_secs == 0 {
            return Err(ScheduleError::InvalidInterval(s.to_string()));
        }

        Ok(std::time::Duration::from_secs(total_secs))
    }

    /// Parse a cron expression.
    fn parse_cron(expression: &str) -> Result<ScheduleType, ScheduleError> {
        // Count fields to determine if it's 5-field or 6-field cron
        let fields: Vec<&str> = expression.split_whitespace().collect();

        let cron_expr = match fields.len() {
            5 => {
                // Standard 5-field cron, add seconds field
                format!("0 {}", expression)
            }
            6 => {
                // Extended 6-field cron with seconds
                expression.to_string()
            }
            _ => {
                return Err(ScheduleError::InvalidCron(format!(
                    "expected 5 or 6 fields, got {}",
                    fields.len()
                )));
            }
        };

        let schedule = CronSchedule::from_str(&cron_expr)
            .map_err(|e| ScheduleError::InvalidCron(e.to_string()))?;

        Ok(ScheduleType::Cron(Box::new(schedule)))
    }

    /// Get the next occurrence after the given time.
    pub fn next_after(&self, after: DateTime<Utc>) -> Result<DateTime<Utc>, ScheduleError> {
        let tz: Tz = self
            .timezone
            .parse()
            .map_err(|_| ScheduleError::InvalidTimezone(self.timezone.clone()))?;

        match &self.schedule_type {
            ScheduleType::Cron(schedule) => {
                // Convert to timezone, find next, convert back to UTC
                let local_time = after.with_timezone(&tz);
                schedule
                    .after(&local_time)
                    .next()
                    .map(|dt| dt.with_timezone(&Utc))
                    .ok_or(ScheduleError::NoMoreOccurrences)
            }
            ScheduleType::Interval(duration) => {
                // For intervals, just add the duration
                let next = after + chrono::Duration::from_std(*duration).unwrap();
                Ok(next)
            }
            ScheduleType::Unparsed => Err(ScheduleError::InvalidCron("schedule not parsed".into())),
        }
    }

    /// Get the next occurrence from now.
    pub fn next(&self) -> Result<DateTime<Utc>, ScheduleError> {
        self.next_after(Utc::now())
    }

    /// Get the next N occurrences after the given time.
    pub fn next_n_after(
        &self,
        after: DateTime<Utc>,
        n: usize,
    ) -> Result<Vec<DateTime<Utc>>, ScheduleError> {
        let tz: Tz = self
            .timezone
            .parse()
            .map_err(|_| ScheduleError::InvalidTimezone(self.timezone.clone()))?;

        match &self.schedule_type {
            ScheduleType::Cron(schedule) => {
                let local_time = after.with_timezone(&tz);
                Ok(schedule
                    .after(&local_time)
                    .take(n)
                    .map(|dt| dt.with_timezone(&Utc))
                    .collect())
            }
            ScheduleType::Interval(duration) => {
                let chrono_duration = chrono::Duration::from_std(*duration).unwrap();
                let mut results = Vec::with_capacity(n);
                let mut current = after;
                for _ in 0..n {
                    current += chrono_duration;
                    results.push(current);
                }
                Ok(results)
            }
            ScheduleType::Unparsed => Err(ScheduleError::InvalidCron("schedule not parsed".into())),
        }
    }

    /// Get the next N occurrences from now.
    pub fn next_n(&self, n: usize) -> Result<Vec<DateTime<Utc>>, ScheduleError> {
        self.next_n_after(Utc::now(), n)
    }

    /// Get the original expression string.
    pub fn expression(&self) -> &str {
        &self.expression
    }

    /// Get the timezone.
    pub fn timezone(&self) -> &str {
        &self.timezone
    }
}

// Custom deserialization to re-parse the schedule after loading
impl<'de> Deserialize<'de> for ScheduleType {
    fn deserialize<D>(_deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        // Always deserialize as Unparsed; the Schedule will re-parse
        Ok(ScheduleType::Unparsed)
    }
}

impl Serialize for ScheduleType {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        // Serialize as unit (nothing)
        serializer.serialize_unit()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Timelike};

    #[test]
    fn test_parse_standard_5_field_cron() {
        let schedule = Schedule::new("0 * * * *").unwrap();
        assert_eq!(schedule.expression(), "0 * * * *");

        // Should get next occurrence
        let next = schedule.next();
        assert!(next.is_ok());
    }

    #[test]
    fn test_parse_extended_6_field_cron() {
        // Every 30 seconds
        let schedule = Schedule::new("30 * * * * *").unwrap();
        assert_eq!(schedule.expression(), "30 * * * * *");

        let next = schedule.next();
        assert!(next.is_ok());
    }

    #[test]
    fn test_parse_daily_shortcut() {
        let schedule = Schedule::new("@daily").unwrap();
        assert_eq!(schedule.expression(), "@daily");

        // Next occurrence should be at midnight
        let base = Utc.with_ymd_and_hms(2024, 1, 15, 12, 0, 0).unwrap();
        let next = schedule.next_after(base).unwrap();

        // Should be next day at midnight
        assert_eq!(next.hour(), 0);
        assert_eq!(next.minute(), 0);
    }

    #[test]
    fn test_parse_hourly_shortcut() {
        let schedule = Schedule::new("@hourly").unwrap();
        assert_eq!(schedule.expression(), "@hourly");

        let base = Utc.with_ymd_and_hms(2024, 1, 15, 12, 30, 0).unwrap();
        let next = schedule.next_after(base).unwrap();

        // Should be next hour at :00
        assert_eq!(next.minute(), 0);
        assert!(next > base);
    }

    #[test]
    fn test_parse_weekly_shortcut() {
        let schedule = Schedule::new("@weekly").unwrap();
        assert_eq!(schedule.expression(), "@weekly");

        let next = schedule.next();
        assert!(next.is_ok());
    }

    #[test]
    fn test_parse_every_5m_interval() {
        let schedule = Schedule::new("@every 5m").unwrap();
        assert_eq!(schedule.expression(), "@every 5m");

        let base = Utc.with_ymd_and_hms(2024, 1, 15, 12, 0, 0).unwrap();
        let next = schedule.next_after(base).unwrap();

        // Should be 5 minutes later
        assert_eq!((next - base).num_minutes(), 5);
    }

    #[test]
    fn test_parse_every_1h30m_interval() {
        let schedule = Schedule::new("@every 1h30m").unwrap();

        let base = Utc.with_ymd_and_hms(2024, 1, 15, 12, 0, 0).unwrap();
        let next = schedule.next_after(base).unwrap();

        // Should be 1 hour 30 minutes later
        assert_eq!((next - base).num_minutes(), 90);
    }

    #[test]
    fn test_get_next_occurrence_from_now() {
        let schedule = Schedule::new("* * * * *").unwrap(); // Every minute
        let now = Utc::now();
        let next = schedule.next().unwrap();

        assert!(next > now);
    }

    #[test]
    fn test_get_next_n_occurrences() {
        let schedule = Schedule::new("@every 1h").unwrap();

        let base = Utc.with_ymd_and_hms(2024, 1, 15, 12, 0, 0).unwrap();
        let occurrences = schedule.next_n_after(base, 5).unwrap();

        assert_eq!(occurrences.len(), 5);

        // Each should be 1 hour apart
        for (i, occurrence) in occurrences.iter().enumerate() {
            let expected = base + chrono::Duration::hours((i + 1) as i64);
            assert_eq!(*occurrence, expected);
        }
    }

    #[test]
    fn test_timezone_aware_scheduling() {
        // Schedule at 9 AM in New York
        let schedule = Schedule::with_timezone("0 9 * * *", "America/New_York").unwrap();
        assert_eq!(schedule.timezone(), "America/New_York");

        let base = Utc.with_ymd_and_hms(2024, 1, 15, 12, 0, 0).unwrap();
        let next = schedule.next_after(base).unwrap();

        // The result should be in UTC but represent 9 AM New York time
        assert!(next > base);
    }

    #[test]
    fn test_invalid_cron_expression_returns_error() {
        let result = Schedule::new("invalid cron");
        assert!(result.is_err());

        match result {
            Err(ScheduleError::InvalidCron(_)) => {}
            _ => panic!("Expected InvalidCron error"),
        }
    }

    #[test]
    fn test_invalid_timezone_returns_error() {
        let result = Schedule::with_timezone("0 * * * *", "Invalid/Timezone");
        assert!(result.is_err());

        match result {
            Err(ScheduleError::InvalidTimezone(_)) => {}
            _ => panic!("Expected InvalidTimezone error"),
        }
    }

    #[test]
    fn test_invalid_interval_returns_error() {
        let result = Schedule::new("@every invalid");
        assert!(result.is_err());
    }

    #[test]
    fn test_monthly_shortcut() {
        let schedule = Schedule::new("@monthly").unwrap();
        let next = schedule.next();
        assert!(next.is_ok());
    }

    #[test]
    fn test_yearly_shortcut() {
        let schedule = Schedule::new("@yearly").unwrap();
        let next = schedule.next();
        assert!(next.is_ok());
    }

    #[test]
    fn test_cron_with_specific_values() {
        // Every day at 2:30 AM
        let schedule = Schedule::new("30 2 * * *").unwrap();

        let base = Utc.with_ymd_and_hms(2024, 1, 15, 0, 0, 0).unwrap();
        let next = schedule.next_after(base).unwrap();

        assert_eq!(next.hour(), 2);
        assert_eq!(next.minute(), 30);
    }

    #[test]
    fn test_seconds_precision() {
        // Every 15 seconds
        let schedule = Schedule::new("15 * * * * *").unwrap();

        let base = Utc.with_ymd_and_hms(2024, 1, 15, 12, 0, 0).unwrap();
        let next = schedule.next_after(base).unwrap();

        assert_eq!(next.second(), 15);
    }

    #[test]
    fn test_interval_with_seconds() {
        let schedule = Schedule::new("@every 30s").unwrap();

        let base = Utc.with_ymd_and_hms(2024, 1, 15, 12, 0, 0).unwrap();
        let next = schedule.next_after(base).unwrap();

        assert_eq!((next - base).num_seconds(), 30);
    }

    #[test]
    fn test_interval_with_days() {
        let schedule = Schedule::new("@every 1d").unwrap();

        let base = Utc.with_ymd_and_hms(2024, 1, 15, 12, 0, 0).unwrap();
        let next = schedule.next_after(base).unwrap();

        assert_eq!((next - base).num_days(), 1);
    }
}
