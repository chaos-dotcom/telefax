use crate::history::{can_print, record_print};
use crate::print::{print_image_cups, resize_image};
use crate::AppState;
use log::{error, info, warn};
use regex::Regex;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::sync::LazyLock;
use teloxide::prelude::*;
use teloxide::utils::command::BotCommands;

#[derive(BotCommands, Clone)]
#[command(rename_rule = "lowercase", description = "Supported commands:")]
pub enum Command {
    #[command(description = "Display the welcome message.")]
    Start,
    #[command(description = "Show this help message.")]
    Help,
    #[command(description = "Set the max copies allowed per print.")]
    SetMaxCopies,
}

pub async fn command_handler(
    bot: Bot,
    msg: Message,
    cmd: Command,
    state: Arc<AppState>,
) -> ResponseResult<()> {
    match cmd {
        Command::Start => start(bot, msg, state).await,
        Command::Help => help(bot, msg, state).await,
        Command::SetMaxCopies => set_max_copies(bot, msg, state).await,
    }
}

pub async fn start(bot: Bot, msg: Message, state: Arc<AppState>) -> ResponseResult<()> {
    let user = msg.from.as_ref().expect("Message has no sender");
    info!("[cmd:start] user_id={} username={}", user.id, user.username.as_deref().unwrap_or("None"));

    let mention = format!("<a href=\"tg://user?id={}\">{}</a>", user.id.0, user.full_name());
    let mut welcome = format!("Hi {}! Send me an image to print on the label printer.", mention);

    if state.config.cups_printer_name.is_empty() {
        welcome.push_str("\n\n<b>⚠️ Warning:</b> The printer is not configured. Printing is currently disabled. Please contact the administrator.");
        warn!("[cmd:start] Printer not configured — informing user {}", user.id);
    }

    if !state.config.is_authorized(user.id.0 as i64) {
        let history = state.history.lock().await;
        let (can, reason) = can_print(user.id.0 as i64, user.username.as_deref(), &state.config, &history);
        drop(history);
        if !can
            && let Some(ref r) = reason
                && r.contains("Please wait") {
                    welcome.push_str(&format!("\n\n<b>⏳ Status:</b> {}", r));
                }
    }

    bot.send_message(msg.chat.id, welcome).parse_mode(teloxide::types::ParseMode::Html).await?;
    Ok(())
}

pub async fn help(bot: Bot, msg: Message, state: Arc<AppState>) -> ResponseResult<()> {
    let user = msg.from.as_ref().expect("Message has no sender");
    info!("[cmd:help] user_id={} username={}", user.id, user.username.as_deref().unwrap_or("None"));

    let is_authorized = state.config.is_authorized(user.id.0 as i64);
    let label_width_str = format_label_dim(state.config.label_width_inches);
    let label_height_str = format_label_dim(state.config.label_height_inches);
    let max_copies = state.max_copies.load(Ordering::SeqCst);

    let base_help_text = if is_authorized {
        format!(
            "<b>🤖 Bot Commands & Usage:</b>\n\n\
            👋 /start - Display the welcome message.\n\
            ❓ /help - Show this help message.\n\
            ⚙️ /setmaxcopies &lt;number&gt; - Set the max copies allowed per print (e.g., <code>/setmaxcopies 50</code>). (Authorized users only)\n\n\
            <b>🖨️ Printing:</b>\n\
            Simply send an image 🖼️ to the chat. The bot will automatically resize it and print it on a {}x{} inch label.\n\n\
            <b>#️⃣ Multiple Copies:</b>\n\
            To print multiple copies, the image caption must contain <b>only</b> the copy specifier (case-insensitive, ignoring surrounding whitespace):\n\
            • <code>x3</code> (prints 3 copies)\n\
            • <code>copies=5</code> (prints 5 copies)\n\
            Any other text in the caption, or no caption, will result in 1 copy being printed.\n\n\
            <b>⚠️ Max Copies Limit:</b>\n\
            The maximum number of copies per request is currently <b>{}</b>.",
            label_width_str, label_height_str, max_copies
        )
    } else {
        format!(
            "<b>🤖 Bot Commands & Usage:</b>\n\n\
            👋 /start - Display the welcome message.\n\
            ❓ /help - Show this help message.\n\n\
            <b>🖨️ Printing:</b>\n\
            Simply send an image 🖼️ to the chat. The bot will automatically print <b>one copy</b> on a {}x{} inch label.",
            label_width_str, label_height_str
        )
    };

    let guest_status = if state.config.allow_guest_printing {
        "\n\n<b>👤 Guest Printing:</b>\nGuest printing is currently <b>enabled</b>. Users not on the authorized list can print one image every 7 days.".to_string()
    } else {
        "\n\n<b>👤 Guest Printing:</b>\nGuest printing is currently <b>disabled</b>. Only authorized users can print.".to_string()
    };

    let mut help_text = base_help_text + &guest_status;

    if !is_authorized {
        let history = state.history.lock().await;
        let (can, reason) = can_print(user.id.0 as i64, user.username.as_deref(), &state.config, &history);
        drop(history);
        if !can
            && let Some(ref r) = reason
                && r.contains("Please wait") {
                    help_text.push_str(&format!("\n\n<b>⏳ Status:</b> {}", r));
                }
    }

    bot.send_message(msg.chat.id, help_text).parse_mode(teloxide::types::ParseMode::Html).await?;
    Ok(())
}

pub async fn set_max_copies(bot: Bot, msg: Message, state: Arc<AppState>) -> ResponseResult<()> {
    let user = msg.from.as_ref().expect("Message has no sender");
    if !state.config.is_authorized(user.id.0 as i64) {
        warn!("[cmd:setmaxcopies] UNAUTHORIZED user_id={} username={}", user.id, user.username.as_deref().unwrap_or("None"));
        bot.send_message(msg.chat.id, "Sorry, you are not authorized to use this command.").await?;
        return Ok(());
    }

    let text = msg.text().unwrap_or_default();
    let parts: Vec<&str> = text.split_whitespace().collect();
    if parts.len() != 2 {
        bot.send_message(msg.chat.id, "Usage: /setmaxcopies <number>\nExample: /setmaxcopies 50").await?;
        return Ok(());
    }

    match parts[1].parse::<usize>() {
        Ok(new_max) if new_max > 0 => {
            state.max_copies.store(new_max, Ordering::SeqCst);
            info!("[cmd:setmaxcopies] user_id={} set MAX_COPIES to {}", user.id, new_max);
            bot.send_message(
                msg.chat.id,
                format!("Maximum copies per request set to <b>{}</b> for this session.", new_max),
            )
            .parse_mode(teloxide::types::ParseMode::Html)
            .await?;
        }
        Ok(_) => {
            bot.send_message(msg.chat.id, "Maximum copies must be a positive number.").await?;
        }
        Err(_) => {
            bot.send_message(msg.chat.id, "Invalid number provided. Please enter a whole number.").await?;
        }
    }

    Ok(())
}

pub async fn handle_image(bot: Bot, msg: Message, state: Arc<AppState>) -> ResponseResult<()> {
    let user = msg.from.as_ref().expect("Message has no sender");
    let user_id = user.id.0 as i64;
    let username = user.username.as_deref().unwrap_or("None");

    info!("[handle_image] === START === user_id={} username={} chat_id={} message_id={}", user_id, username, msg.chat.id, msg.id);

    {
        let history = state.history.lock().await;
        info!("[handle_image] History loaded: {} user(s)", history.len());
        let (is_allowed, reason) = can_print(user_id, user.username.as_deref(), &state.config, &history);
        drop(history);
        if !is_allowed {
            warn!("[handle_image] BLOCKED user_id={} reason={:?}", user_id, reason);
            let reply = format!("Sorry, you cannot print right now. {}", reason.unwrap_or_default());
            bot.send_message(msg.chat.id, reply).await?;
            return Ok(());
        }
        info!("[handle_image] Auth/rate-limit check PASSED");
    }

    let photos = match msg.photo() {
        Some(p) if !p.is_empty() => p,
        _ => {
            warn!("[handle_image] No photo in message from user {}", user_id);
            bot.send_message(msg.chat.id, "Please send an image file.").await?;
            return Ok(());
        }
    };

    info!("[handle_image] Photo count={}", photos.len());
    let largest = photos.last().unwrap();
    info!(
        "[handle_image] Largest photo | file_id={} width={} height={} file_size={:?}",
        largest.file.id, largest.width, largest.height, largest.file.size
    );

    if state.config.cups_printer_name.is_empty() {
        error!("[handle_image] CUPS_PRINTER_NAME is EMPTY — printing disabled");
        bot.send_message(msg.chat.id, "Printer is not configured. Please contact the administrator.").await?;
        return Ok(());
    }
    info!(
        "[handle_image] Config | printer='{}' server={:?} label={}x{}in ({}x{}px)",
        state.config.cups_printer_name,
        state.config.cups_server_host,
        state.config.label_width_inches,
        state.config.label_height_inches,
        state.config.label_width_px,
        state.config.label_height_px,
    );

    let is_authorized = state.config.is_authorized(user_id);
    let caption = msg.caption();
    let max_copies = state.max_copies.load(Ordering::SeqCst);
    info!("[handle_image] Parse | is_authorized={} caption={:?} max_copies={}", is_authorized, caption, max_copies);
    let requested_copies = parse_copies(caption, max_copies);
    info!("[handle_image] Parse | requested_copies={}", requested_copies);

    let (copies_to_print, copies_message) = if is_authorized {
        let msg = if requested_copies == 1 {
            "1 copy".to_string()
        } else {
            format!("{} copies", requested_copies)
        };
        (requested_copies, msg)
    } else {
        if requested_copies > 1 {
            info!("[handle_image] Guest user requested {} copies — forcing 1", requested_copies);
            (1, "1 copy (multiple copies ignored for guest users)".to_string())
        } else {
            (1, "1 copy".to_string())
        }
    };
    info!("[handle_image] Will print | copies_to_print={} message='{}'", copies_to_print, copies_message);

    bot.send_message(
        msg.chat.id,
        format!(
            "Received image. Resizing for {}x{}in label and preparing to print {}...",
            format_label_dim(state.config.label_width_inches),
            format_label_dim(state.config.label_height_inches),
            copies_message
        ),
    )
    .await?;

    let file = bot.get_file(largest.file.id.clone()).await?;
    let url = format!("https://api.telegram.org/file/bot{}/{}", bot.token(), file.path);
    info!("[handle_image] Downloading image from Telegram | url_len={}", url.len());

    let image_bytes = reqwest::get(&url)
        .await
        .map_err(|e| {
            error!("[handle_image] Download GET failed: {}", e);
            teloxide::errors::RequestError::Io(std::io::Error::other(e).into())
        })?
        .bytes()
        .await
        .map_err(|e| {
            error!("[handle_image] Download body failed: {}", e);
            teloxide::errors::RequestError::Io(std::io::Error::other(e).into())
        })?
        .to_vec();

    info!("[handle_image] Downloaded {} bytes from Telegram", image_bytes.len());

    let (resized, image_format) = match resize_image(&image_bytes, &state.config) {
        Ok((buf, fmt)) => {
            info!("[handle_image] Resize OK | format={} output_bytes={}", fmt, buf.len());
            (buf, fmt)
        }
        Err(e) => {
            error!("[handle_image] Resize FAILED: {}", e);
            bot.send_message(msg.chat.id, "Failed to process the image.").await?;
            return Ok(());
        }
    };

    info!(
        "[handle_image] Spawning CUPS | printer='{}' copies={} format={}",
        state.config.cups_printer_name, copies_to_print, image_format
    );

    match print_image_cups(&resized, &state.config.cups_printer_name, copies_to_print, &image_format, &state.config).await {
        Ok(cups_msg) => {
            info!("[handle_image] CUPS SUCCESS | stdout='{}'", cups_msg.trim());
            let reply = format!(
                "Sent {} cop{} to printer! CUPS message: {}",
                copies_to_print,
                if copies_to_print == 1 { "y" } else { "ies" },
                cups_msg
            );
            bot.send_message(msg.chat.id, reply).await?;

            if state.config.allow_guest_printing && !is_authorized {
                info!("[handle_image] Recording print for guest user {}", user_id);
                record_print(user_id, user.username.as_deref(), &state.history).await;
            }
        }
        Err(e) => {
            error!("[handle_image] CUPS FAILURE | error='{}'", e);
            bot.send_message(msg.chat.id, format!("Failed to send to printer. Error: {}", e)).await?;
        }
    }

    info!("[handle_image] === END === user_id={}", user_id);
    Ok(())
}

fn format_label_dim(dim: f64) -> String {
    dim.to_string()
}

static RE_X: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)^x(\d+)$").unwrap());
static RE_COPIES: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)^copies\s*=\s*(\d+)$").unwrap());

fn parse_copies(caption: Option<&str>, max_copies: usize) -> usize {
    let caption = caption.map(|s| s.trim().to_lowercase()).unwrap_or_default();
    if caption.is_empty() {
        return 1;
    }

    if let Some(caps) = RE_X.captures(&caption)
        && let Ok(n) = caps[1].parse::<usize>() {
            if (1..=max_copies).contains(&n) {
                return n;
            } else {
                warn!("[parse_copies] Out of range: requested={} max={}", n, max_copies);
                return 1;
            }
        }

    if let Some(caps) = RE_COPIES.captures(&caption)
        && let Ok(n) = caps[1].parse::<usize>() {
            if (1..=max_copies).contains(&n) {
                return n;
            } else {
                warn!("[parse_copies] Out of range: requested={} max={}", n, max_copies);
                return 1;
            }
        }

    info!("[parse_copies] No match for caption='{}', defaulting to 1", caption);
    1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_copies_empty() {
        assert_eq!(parse_copies(None, 100), 1);
        assert_eq!(parse_copies(Some("  "), 100), 1);
    }

    #[test]
    fn parse_copies_x_format() {
        assert_eq!(parse_copies(Some("x3"), 100), 3);
        assert_eq!(parse_copies(Some("X3"), 100), 3);
        assert_eq!(parse_copies(Some("  x5  "), 100), 5);
    }

    #[test]
    fn parse_copies_equals_format() {
        assert_eq!(parse_copies(Some("copies=5"), 100), 5);
        assert_eq!(parse_copies(Some("COPIES=5"), 100), 5);
        assert_eq!(parse_copies(Some("copies = 7"), 100), 7);
    }

    #[test]
    fn parse_copies_out_of_range() {
        assert_eq!(parse_copies(Some("x0"), 100), 1);
        assert_eq!(parse_copies(Some("x101"), 100), 1);
        assert_eq!(parse_copies(Some("copies=999"), 100), 1);
    }

    #[test]
    fn parse_copies_random_text() {
        assert_eq!(parse_copies(Some("hello world"), 100), 1);
        assert_eq!(parse_copies(Some("x3 extra"), 100), 1);
        assert_eq!(parse_copies(Some("print 3 copies"), 100), 1);
    }

    #[test]
    fn format_label_dim_whole_number() {
        assert_eq!(format_label_dim(4.0), "4");
        assert_eq!(format_label_dim(6.0), "6");
    }

    #[test]
    fn format_label_dim_decimal() {
        assert_eq!(format_label_dim(4.5), "4.5");
        assert_eq!(format_label_dim(3.25), "3.25");
    }
}
