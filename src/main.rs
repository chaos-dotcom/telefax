use log::{error, info};
use std::sync::atomic::AtomicUsize;
use std::sync::Arc;
use teloxide::prelude::*;
use tokio::sync::Mutex;

mod config;
mod handlers;
mod history;
mod print;

use config::Config;
use history::PrintHistory;

#[derive(Clone)]
pub struct AppState {
    pub config: Config,
    pub history: Arc<Mutex<PrintHistory>>,
    pub max_copies: Arc<AtomicUsize>,
}

#[tokio::main]
async fn main() {
    dotenvy::dotenv().ok();

    // Default to INFO in release builds so containers produce useful logs
    // without requiring RUST_LOG to be set.  RUST_LOG still overrides.
    let mut builder = env_logger::Builder::from_default_env();
    if std::env::var("RUST_LOG").is_err() {
        builder.filter_level(log::LevelFilter::Info);
    }
    builder.init();

    let config = match Config::from_env() {
        Ok(c) => c,
        Err(e) => {
            error!("Failed to load configuration: {}", e);
            std::process::exit(1);
        }
    };

    let history = if config.allow_guest_printing {
        history::load_print_history().await
    } else {
        PrintHistory::new()
    };

    let state = AppState {
        config: config.clone(),
        history: Arc::new(Mutex::new(history)),
        max_copies: Arc::new(AtomicUsize::new(config.max_copies)),
    };

    let bot = Bot::new(&config.telegram_bot_token);

    let handler = dptree::entry()
        .branch(
            Update::filter_message()
                .filter_command::<handlers::Command>()
                .endpoint(handlers::command_handler),
        )
        .branch(
            Update::filter_message()
                .filter(|msg: Message| msg.photo().is_some())
                .endpoint(handlers::handle_image),
        );

    info!("Starting bot polling...");

    Dispatcher::builder(bot, handler)
        .dependencies(dptree::deps![Arc::new(state)])
        .default_handler(|upd| async move {
            log::debug!("Unhandled update: {:?}", upd);
        })
        .error_handler(LoggingErrorHandler::with_custom_text(
            "An error has occurred in the dispatcher",
        ))
        .enable_ctrlc_handler()
        .build()
        .dispatch()
        .await;
}
