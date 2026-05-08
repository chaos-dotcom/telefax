use crate::config::{Config, PRINT_HISTORY_FILE};
use anyhow::{Context, Result};
use chrono::{DateTime, Duration, Utc};
use log::{info, warn};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tokio::fs;
use tokio::sync::Mutex;

pub const UNAUTHORIZED_USER_PRINT_INTERVAL: Duration = Duration::days(7);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserPrintRecord {
    pub last_print: DateTime<Utc>,
    pub username: String,
}

pub type PrintHistory = HashMap<i64, UserPrintRecord>;

pub async fn load_print_history() -> PrintHistory {
    match fs::read_to_string(PRINT_HISTORY_FILE).await {
        Ok(contents) => {
            match serde_json::from_str::<HashMap<String, serde_json::Value>>(&contents) {
                Ok(raw) => {
                    let mut loaded = HashMap::with_capacity(raw.len());
                    for (user_id_str, data) in raw {
                        match parse_history_entry(&user_id_str, &data) {
                            Ok((user_id, record)) => {
                                loaded.insert(user_id, record);
                            }
                            Err(e) => {
                                warn!(
                                    "Skipping entry for user {} due to parsing error: {}",
                                    user_id_str, e
                                );
                            }
                        }
                    }
                    info!(
                        "Loaded print history for {} users from {}",
                        loaded.len(),
                        PRINT_HISTORY_FILE
                    );
                    loaded
                }
                Err(e) => {
                    warn!(
                        "Error parsing {}: {}. Starting with empty history.",
                        PRINT_HISTORY_FILE, e
                    );
                    HashMap::new()
                }
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            info!(
                "{} not found. Starting with empty history.",
                PRINT_HISTORY_FILE
            );
            HashMap::new()
        }
        Err(e) => {
            warn!(
                "Error reading {}: {}. Starting with empty history.",
                PRINT_HISTORY_FILE, e
            );
            HashMap::new()
        }
    }
}

fn parse_history_entry(
    user_id_str: &str,
    data: &serde_json::Value,
) -> Result<(i64, UserPrintRecord)> {
    let user_id: i64 = user_id_str
        .parse()
        .with_context(|| format!("Invalid user ID: {}", user_id_str))?;

    let record = match data {
        serde_json::Value::Object(map) => {
            let last_print = map
                .get("last_print")
                .and_then(|v| v.as_str())
                .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
                .map(|dt| dt.with_timezone(&Utc))
                .with_context(|| format!("Missing or invalid last_print for user {}", user_id))?;
            let username = map
                .get("username")
                .and_then(|v| v.as_str())
                .unwrap_or("Unknown")
                .to_string();
            UserPrintRecord {
                last_print,
                username,
            }
        }
        serde_json::Value::String(s) => {
            let last_print = DateTime::parse_from_rfc3339(s)
                .map(|dt| dt.with_timezone(&Utc))
                .with_context(|| format!("Invalid timestamp for user {}", user_id))?;
            UserPrintRecord {
                last_print,
                username: "Unknown".to_string(),
            }
        }
        _ => {
            anyhow::bail!("Invalid data type for user {} in history file", user_id);
        }
    };

    Ok((user_id, record))
}

pub async fn save_print_history(history: &PrintHistory) {
    let to_save: HashMap<String, UserPrintRecord> = history
        .iter()
        .map(|(k, v)| (k.to_string(), v.clone()))
        .collect();

    match serde_json::to_string_pretty(&to_save) {
        Ok(json) => {
            if let Err(e) = fs::write(PRINT_HISTORY_FILE, json).await {
                warn!("Error saving print history to {}: {}", PRINT_HISTORY_FILE, e);
            }
        }
        Err(e) => {
            warn!("Error serializing print history: {}", e);
        }
    }
}

/// Checks if a user is allowed to print.
/// Returns (true, None) if allowed.
/// Returns (false, reason_message) if not allowed.
pub fn can_print(user_id: i64, _username: Option<&str>, config: &Config, history: &PrintHistory) -> (bool, Option<String>) {
    let is_authorized = config.is_authorized(user_id);
    info!("[can_print] user_id={} authorized={} guest={:?}", user_id, is_authorized, config.allow_guest_printing);

    if is_authorized {
        info!("[can_print] ALLOWED — user is authorized");
        return (true, None);
    }

    // Rate-limit override (e.g. EMFcamp weekend) — skip cooldown entirely
    if config.is_rate_limit_override_active() {
        info!("[can_print] OVERRIDE ACTIVE — skipping rate-limit check for user {}", user_id);
        if config.allow_guest_printing {
            return (true, None);
        }
        warn!("[can_print] BLOCKED — guest printing disabled for user {}", user_id);
        return (false, Some("Printing is restricted to authorized users only.".to_string()));
    }

    // Rate limit check for non-authorized users
    if let Some(record) = history.get(&user_id) {
        let time_since_last_print = Utc::now() - record.last_print;
        info!("[can_print] Found history | last_print={} ago", format_wait_time(time_since_last_print));
        if time_since_last_print < UNAUTHORIZED_USER_PRINT_INTERVAL {
            let wait = UNAUTHORIZED_USER_PRINT_INTERVAL - time_since_last_print;
            let wait_str = format_wait_time(wait);
            let reason = format!(
                "You have already printed recently. Please wait {} before printing again.",
                wait_str
            );
            warn!(
                "[can_print] BLOCKED — user {} still in cooldown | wait={}",
                user_id, wait_str
            );
            return (false, Some(reason));
        }
        info!("[can_print] Cooldown expired");
    } else {
        info!("[can_print] No history for user {}", user_id);
    }

    if config.allow_guest_printing {
        info!("[can_print] ALLOWED — guest printing enabled");
        (true, None)
    } else {
        warn!(
            "[can_print] BLOCKED — guest printing disabled for user {}",
            user_id
        );
        (
            false,
            Some("Printing is restricted to authorized users only.".to_string()),
        )
    }
}

fn format_wait_time(wait: Duration) -> String {
    let days = wait.num_days();
    let hours = wait.num_hours() % 24;
    let minutes = wait.num_minutes() % 60;

    let mut parts = Vec::new();
    if days > 0 {
        parts.push(format!("{} day{}", days, if days == 1 { "" } else { "s" }));
    }
    if hours > 0 {
        parts.push(format!("{} hour{}", hours, if hours == 1 { "" } else { "s" }));
    }
    if days == 0 && hours == 0 && minutes > 0 {
        parts.push(format!(
            "{} minute{}",
            minutes,
            if minutes == 1 { "" } else { "s" }
        ));
    }
    if parts.is_empty() {
        parts.push("less than a minute".to_string());
    }
    parts.join(", ")
}

pub async fn record_print(
    user_id: i64,
    username: Option<&str>,
    history: &Mutex<PrintHistory>,
) {
    let now = Utc::now();
    let display_name = username.unwrap_or("Unknown").to_string();
    let snapshot = {
        let mut guard = history.lock().await;
        guard.insert(
            user_id,
            UserPrintRecord {
                last_print: now,
                username: display_name.clone(),
            },
        );
        guard.clone()
    };
    info!(
        "Recorded print for user {} ({}) at {}",
        user_id, display_name, now
    );
    save_print_history(&snapshot).await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    fn test_config_with_users(allowed: Vec<i64>, guest: bool) -> Config {
        Config {
            telegram_bot_token: "t".to_string(),
            cups_printer_name: "p".to_string(),
            cups_server_host: None,
            allowed_user_ids: allowed,
            allow_guest_printing: guest,
            max_copies: 100,
            label_width_inches: 4.0,
            label_height_inches: 6.0,
            label_width_px: 1200,
            label_height_px: 1800,
            rate_limit_override_start: None,
            rate_limit_override_end: None,
        }
    }

    #[test]
    fn authorized_user_always_allowed() {
        let config = test_config_with_users(vec![100], true);
        let history = PrintHistory::new();
        let (allowed, reason) = can_print(100, None, &config, &history);
        assert!(allowed);
        assert!(reason.is_none());
    }

    #[test]
    fn unauthorized_no_history_guest_enabled() {
        let config = test_config_with_users(vec![100], true);
        let history = PrintHistory::new();
        let (allowed, reason) = can_print(200, None, &config, &history);
        assert!(allowed);
        assert!(reason.is_none());
    }

    #[test]
    fn unauthorized_no_history_guest_disabled() {
        let config = test_config_with_users(vec![100], false);
        let history = PrintHistory::new();
        let (allowed, reason) = can_print(200, None, &config, &history);
        assert!(!allowed);
        assert_eq!(reason, Some("Printing is restricted to authorized users only.".to_string()));
    }

    #[test]
    fn unauthorized_recent_print_rate_limited() {
        let config = test_config_with_users(vec![100], true);
        let mut history = PrintHistory::new();
        history.insert(
            200,
            UserPrintRecord {
                last_print: Utc::now() - Duration::hours(1),
                username: "test".to_string(),
            },
        );
        let (allowed, reason) = can_print(200, None, &config, &history);
        assert!(!allowed);
        assert!(reason.as_ref().unwrap().contains("Please wait"));
    }

    #[test]
    fn unauthorized_old_print_allowed() {
        let config = test_config_with_users(vec![100], true);
        let mut history = PrintHistory::new();
        history.insert(
            200,
            UserPrintRecord {
                last_print: Utc::now() - Duration::days(8),
                username: "test".to_string(),
            },
        );
        let (allowed, reason) = can_print(200, None, &config, &history);
        assert!(allowed);
        assert!(reason.is_none());
    }

    #[test]
    fn rate_limit_override_skips_cooldown() {
        let mut config = test_config_with_users(vec![100], true);
        // Set override to today
        let today = chrono::Utc::now().date_naive();
        config.rate_limit_override_start = Some(today);
        config.rate_limit_override_end = Some(today);

        let mut history = PrintHistory::new();
        history.insert(
            200,
            UserPrintRecord {
                last_print: Utc::now() - Duration::hours(1),
                username: "test".to_string(),
            },
        );
        let (allowed, reason) = can_print(200, None, &config, &history);
        assert!(allowed);
        assert!(reason.is_none());
    }

    #[test]
    fn rate_limit_override_respects_guest_disabled() {
        let mut config = test_config_with_users(vec![100], false);
        let today = chrono::Utc::now().date_naive();
        config.rate_limit_override_start = Some(today);
        config.rate_limit_override_end = Some(today);

        let mut history = PrintHistory::new();
        history.insert(
            200,
            UserPrintRecord {
                last_print: Utc::now() - Duration::hours(1),
                username: "test".to_string(),
            },
        );
        let (allowed, reason) = can_print(200, None, &config, &history);
        assert!(!allowed);
        assert_eq!(reason, Some("Printing is restricted to authorized users only.".to_string()));
    }

    #[test]
    fn format_wait_time_days_only() {
        let wait = Duration::days(3);
        assert_eq!(format_wait_time(wait), "3 days");
    }

    #[test]
    fn format_wait_time_hours_only() {
        let wait = Duration::hours(5);
        assert_eq!(format_wait_time(wait), "5 hours");
    }

    #[test]
    fn format_wait_time_minutes_only() {
        let wait = Duration::minutes(45);
        assert_eq!(format_wait_time(wait), "45 minutes");
    }

    #[test]
    fn format_wait_time_days_and_hours() {
        let wait = Duration::days(2) + Duration::hours(5);
        assert_eq!(format_wait_time(wait), "2 days, 5 hours");
    }

    #[test]
    fn format_wait_time_less_than_a_minute() {
        let wait = Duration::seconds(30);
        assert_eq!(format_wait_time(wait), "less than a minute");
    }

    #[test]
    fn parse_history_entry_new_format() {
        let json = serde_json::json!({
            "last_print": "2025-01-15T10:00:00Z",
            "username": "Alice"
        });
        let (id, record) = parse_history_entry("42", &json).unwrap();
        assert_eq!(id, 42);
        assert_eq!(record.username, "Alice");
        assert_eq!(record.last_print, DateTime::parse_from_rfc3339("2025-01-15T10:00:00Z").unwrap().with_timezone(&Utc));
    }

    #[test]
    fn parse_history_entry_old_format() {
        let json = serde_json::json!("2025-01-15T10:00:00Z");
        let (id, record) = parse_history_entry("42", &json).unwrap();
        assert_eq!(id, 42);
        assert_eq!(record.username, "Unknown");
    }

    #[test]
    fn parse_history_entry_invalid_user_id() {
        let json = serde_json::json!({"last_print": "2025-01-15T10:00:00Z"});
        assert!(parse_history_entry("not_a_number", &json).is_err());
    }

    #[test]
    fn parse_history_entry_invalid_data_type() {
        let json = serde_json::json!(42);
        assert!(parse_history_entry("1", &json).is_err());
    }
}
