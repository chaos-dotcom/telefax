use anyhow::{Context, Result};
use chrono::{NaiveDate, Utc};
use log::{info, warn};
use std::env;

pub const IMAGE_DPI: u32 = 300;
pub const PRINT_HISTORY_FILE: &str = "print_history.json";

#[derive(Debug, Clone)]
pub struct Config {
    pub telegram_bot_token: String,
    pub cups_printer_name: String,
    pub cups_server_host: Option<String>,
    pub allowed_user_ids: Vec<i64>,
    pub allow_guest_printing: bool,
    pub max_copies: usize,
    pub label_width_inches: f64,
    pub label_height_inches: f64,
    pub label_width_px: u32,
    pub label_height_px: u32,
    /// Inclusive start date (YYYY-MM-DD) when rate limits are bypassed for guests.
    pub rate_limit_override_start: Option<NaiveDate>,
    /// Inclusive end date (YYYY-MM-DD) when rate limits are bypassed for guests.
    pub rate_limit_override_end: Option<NaiveDate>,
}

impl Config {
    pub fn from_env() -> Result<Self> {
        let telegram_bot_token = env::var("TELEGRAM_BOT_TOKEN")
            .context("TELEGRAM_BOT_TOKEN environment variable is not set")?;

        let cups_printer_name = env::var("CUPS_PRINTER_NAME").unwrap_or_default();
        if cups_printer_name.is_empty() {
            warn!("CUPS_PRINTER_NAME environment variable is not set. Printing will fail.");
        }

        let cups_server_host = env::var("CUPS_SERVER_HOST").ok().filter(|s| !s.is_empty());

        let allowed_user_ids: Vec<i64> = env::var("ALLOWED_USER_IDS")
            .unwrap_or_default()
            .split(',')
            .filter_map(|s| {
                let trimmed = s.trim();
                if trimmed.is_empty() {
                    None
                } else {
                    trimmed.parse().ok()
                }
            })
            .collect();

        let allow_guest_printing = {
            let v = env::var("ALLOW_GUEST_PRINTING").unwrap_or_else(|_| "true".to_string());
            matches!(v.to_lowercase().trim(), "true" | "1" | "yes")
        };

        let max_copies = parse_positive_usize("MAX_COPIES", 100);

        let rate_limit_override_start = parse_date("RATE_LIMIT_OVERRIDE_START");
        let rate_limit_override_end = parse_date("RATE_LIMIT_OVERRIDE_END");
        if let (Some(start), Some(end)) = (rate_limit_override_start, rate_limit_override_end) {
            info!(
                "Rate-limit override configured: {} to {} (inclusive)",
                start, end
            );
            let today = Utc::now().date_naive();
            if today >= start && today <= end {
                info!(
                    "Rate-limit override is ACTIVE today ({}) — guest cooldown is suspended",
                    today
                );
            }
        }

        let label_width_inches = parse_positive_f64("LABEL_WIDTH_INCHES", 4.0)?;
        let label_height_inches = parse_positive_f64("LABEL_HEIGHT_INCHES", 6.0)?;

        let label_width_px = (label_width_inches * f64::from(IMAGE_DPI)) as u32;
        let label_height_px = (label_height_inches * f64::from(IMAGE_DPI)) as u32;

        if !allowed_user_ids.is_empty() {
            info!("Bot access restricted to user IDs: {:?}", allowed_user_ids);
            if allow_guest_printing {
                info!("Guest printing ENABLED (1 print per week limit applies to non-authorized users).");
            } else {
                info!("Guest printing DISABLED. Only authorized users can print.");
            }
        } else if allow_guest_printing {
            warn!("ALLOWED_USER_IDS is not set. Bot is open to everyone (1 print per week limit applies).");
        } else {
            warn!("ALLOWED_USER_IDS is not set AND Guest printing is DISABLED. No one can print!");
        }

        Ok(Config {
            telegram_bot_token,
            cups_printer_name,
            cups_server_host,
            allowed_user_ids,
            allow_guest_printing,
            max_copies,
            label_width_inches,
            label_height_inches,
            label_width_px,
            label_height_px,
            rate_limit_override_start,
            rate_limit_override_end,
        })
    }

    pub fn is_authorized(&self, user_id: i64) -> bool {
        !self.allowed_user_ids.is_empty() && self.allowed_user_ids.contains(&user_id)
    }

    /// Returns true if today falls within the configured rate-limit override window.
    pub fn is_rate_limit_override_active(&self) -> bool {
        let today = chrono::Utc::now().date_naive();
        match (self.rate_limit_override_start, self.rate_limit_override_end) {
            (Some(start), Some(end)) => today >= start && today <= end,
            _ => false,
        }
    }
}

fn parse_date(var: &str) -> Option<NaiveDate> {
    let raw = env::var(var).ok()?;
    match NaiveDate::parse_from_str(&raw, "%Y-%m-%d") {
        Ok(d) => Some(d),
        Err(_) => {
            warn!(
                "Invalid date format for {}: '{}'. Expected YYYY-MM-DD.",
                var, raw
            );
            None
        }
    }
}

fn parse_positive_f64(var: &str, default: f64) -> Result<f64> {
    let raw = env::var(var).unwrap_or_else(|_| default.to_string());
    match raw.parse::<f64>() {
        Ok(v) if v > 0.0 => Ok(v),
        Ok(v) => {
            warn!(
                "{} must be positive (got {}). Defaulting to {}.",
                var, v, default
            );
            Ok(default)
        }
        Err(_) => {
            warn!(
                "Invalid {} value in environment. Defaulting to {}.",
                var, default
            );
            Ok(default)
        }
    }
}

fn parse_positive_usize(var: &str, default: usize) -> usize {
    let raw = env::var(var).unwrap_or_else(|_| default.to_string());
    match raw.parse::<usize>() {
        Ok(v) if v > 0 => v,
        Ok(v) => {
            warn!(
                "{} must be positive (got {}). Defaulting to {}.",
                var, v, default
            );
            default
        }
        Err(_) => {
            warn!(
                "Invalid {} value in environment. Defaulting to {}.",
                var, default
            );
            default
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;

    fn set_env(key: &str, val: &str) {
        unsafe { env::set_var(key, val) };
    }

    fn remove_env(key: &str) {
        unsafe { env::remove_var(key) };
    }

    #[test]
    #[serial]
    fn parse_positive_f64_valid() {
        set_env("TEST_F64", "4.5");
        assert_eq!(parse_positive_f64("TEST_F64", 4.0).unwrap(), 4.5);
        remove_env("TEST_F64");
    }

    #[test]
    #[serial]
    fn parse_positive_f64_invalid_uses_default() {
        set_env("TEST_F64", "not_a_number");
        assert_eq!(parse_positive_f64("TEST_F64", 6.0).unwrap(), 6.0);
        remove_env("TEST_F64");
    }

    #[test]
    #[serial]
    fn parse_positive_f64_non_positive_uses_default() {
        set_env("TEST_F64", "-2.0");
        assert_eq!(parse_positive_f64("TEST_F64", 6.0).unwrap(), 6.0);
        remove_env("TEST_F64");
    }

    #[test]
    #[serial]
    fn parse_positive_f64_missing_uses_default() {
        remove_env("TEST_F64");
        assert_eq!(parse_positive_f64("TEST_F64", 4.0).unwrap(), 4.0);
    }

    #[test]
    #[serial]
    fn parse_positive_usize_valid() {
        set_env("TEST_USIZE", "50");
        assert_eq!(parse_positive_usize("TEST_USIZE", 100), 50);
        remove_env("TEST_USIZE");
    }

    #[test]
    #[serial]
    fn parse_positive_usize_invalid_uses_default() {
        set_env("TEST_USIZE", "abc");
        assert_eq!(parse_positive_usize("TEST_USIZE", 100), 100);
        remove_env("TEST_USIZE");
    }

    #[test]
    #[serial]
    fn parse_positive_usize_zero_uses_default() {
        set_env("TEST_USIZE", "0");
        assert_eq!(parse_positive_usize("TEST_USIZE", 100), 100);
        remove_env("TEST_USIZE");
    }

    #[test]
    #[serial]
    fn parse_positive_usize_missing_uses_default() {
        remove_env("TEST_USIZE");
        assert_eq!(parse_positive_usize("TEST_USIZE", 100), 100);
    }

    #[test]
    #[serial]
    fn config_from_env_happy_path() {
        set_env("TELEGRAM_BOT_TOKEN", "test_token");
        set_env("CUPS_PRINTER_NAME", "TestPrinter");
        set_env("ALLOWED_USER_IDS", "123,456");
        set_env("ALLOW_GUEST_PRINTING", "false");
        set_env("MAX_COPIES", "50");
        set_env("LABEL_WIDTH_INCHES", "4.5");
        set_env("LABEL_HEIGHT_INCHES", "6.5");

        let config = Config::from_env().unwrap();
        assert_eq!(config.telegram_bot_token, "test_token");
        assert_eq!(config.cups_printer_name, "TestPrinter");
        assert_eq!(config.allowed_user_ids, vec![123, 456]);
        assert!(!config.allow_guest_printing);
        assert_eq!(config.max_copies, 50);
        assert_eq!(config.label_width_inches, 4.5);
        assert_eq!(config.label_height_inches, 6.5);
        assert_eq!(config.label_width_px, 1350); // 4.5 * 300
        assert_eq!(config.label_height_px, 1950); // 6.5 * 300

        remove_env("TELEGRAM_BOT_TOKEN");
        remove_env("CUPS_PRINTER_NAME");
        remove_env("ALLOWED_USER_IDS");
        remove_env("ALLOW_GUEST_PRINTING");
        remove_env("MAX_COPIES");
        remove_env("LABEL_WIDTH_INCHES");
        remove_env("LABEL_HEIGHT_INCHES");
    }

    #[test]
    #[serial]
    fn config_from_env_defaults() {
        set_env("TELEGRAM_BOT_TOKEN", "tok");
        remove_env("CUPS_PRINTER_NAME");
        remove_env("ALLOWED_USER_IDS");
        remove_env("ALLOW_GUEST_PRINTING");
        remove_env("MAX_COPIES");
        remove_env("LABEL_WIDTH_INCHES");
        remove_env("LABEL_HEIGHT_INCHES");

        let config = Config::from_env().unwrap();
        assert!(config.cups_printer_name.is_empty());
        assert!(config.allowed_user_ids.is_empty());
        assert!(config.allow_guest_printing);
        assert_eq!(config.max_copies, 100);
        assert_eq!(config.label_width_inches, 4.0);
        assert_eq!(config.label_height_inches, 6.0);

        remove_env("TELEGRAM_BOT_TOKEN");
    }

    #[test]
    #[serial]
    fn config_is_authorized() {
        let config = Config {
            telegram_bot_token: "t".to_string(),
            cups_printer_name: "p".to_string(),
            cups_server_host: None,
            allowed_user_ids: vec![100, 200],
            allow_guest_printing: true,
            max_copies: 10,
            label_width_inches: 4.0,
            label_height_inches: 6.0,
            label_width_px: 1200,
            label_height_px: 1800,
            rate_limit_override_start: None,
            rate_limit_override_end: None,
        };

        assert!(config.is_authorized(100));
        assert!(config.is_authorized(200));
        assert!(!config.is_authorized(300));
    }

    #[test]
    fn config_is_authorized_empty_list() {
        let config = Config {
            telegram_bot_token: "t".to_string(),
            cups_printer_name: "p".to_string(),
            cups_server_host: None,
            allowed_user_ids: vec![],
            allow_guest_printing: true,
            max_copies: 10,
            label_width_inches: 4.0,
            label_height_inches: 6.0,
            label_width_px: 1200,
            label_height_px: 1800,
            rate_limit_override_start: None,
            rate_limit_override_end: None,
        };

        assert!(!config.is_authorized(100));
    }

    #[test]
    fn config_rate_limit_override_active() {
        let today = Utc::now().date_naive();
        let config = Config {
            telegram_bot_token: "t".to_string(),
            cups_printer_name: "p".to_string(),
            cups_server_host: None,
            allowed_user_ids: vec![100],
            allow_guest_printing: true,
            max_copies: 10,
            label_width_inches: 4.0,
            label_height_inches: 6.0,
            label_width_px: 1200,
            label_height_px: 1800,
            rate_limit_override_start: Some(today),
            rate_limit_override_end: Some(today),
        };
        assert!(config.is_rate_limit_override_active());
    }

    #[test]
    fn config_rate_limit_override_inactive() {
        let today = Utc::now().date_naive();
        let config = Config {
            telegram_bot_token: "t".to_string(),
            cups_printer_name: "p".to_string(),
            cups_server_host: None,
            allowed_user_ids: vec![100],
            allow_guest_printing: true,
            max_copies: 10,
            label_width_inches: 4.0,
            label_height_inches: 6.0,
            label_width_px: 1200,
            label_height_px: 1800,
            rate_limit_override_start: Some(today - chrono::Duration::days(10)),
            rate_limit_override_end: Some(today - chrono::Duration::days(5)),
        };
        assert!(!config.is_rate_limit_override_active());
    }
}
