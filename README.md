# Telefax 📠

A Telegram bot that receives images and prints them to a CUPS-managed printer. Think of it like faxing yourself through Telegram!

While designed to work with any CUPS-compatible printer, it was developed with the Dymo LabelWriter 4XL (4x6 inch label printer) particularly in mind. 🏷️

Written in **Rust** 🦀 for performance, safety, and a small memory footprint.

## ✨ Features

- 📥 Receives images sent via Telegram.
- 📐 Resizes images to fit configurable label dimensions (defaults to 4x6 inches).
- 🖨️ Prints images to a specified CUPS printer.
- 🔢 Supports printing multiple copies via image caption (e.g., `x3` or `copies=5`).
- 🔒 Restricts usage to allowed Telegram user IDs.
- 👤 Optional guest printing with rate limiting (1 print per 7 days).
- ⚙️ Runtime command to set the maximum number of copies per print job.

## 🛠️ Setup

1.  **Clone the repository:** 📂
    ```bash
    git clone https://github.com/Johnr24/telefax.git
    cd telefax
    ```
2.  **Configure Environment Variables:** 📝
    Copy the `.env.template` file to `.env` and fill in the required values:

    ```bash
    cp .env.template .env
    ```

    Edit `.env` with your details:
    - `TELEGRAM_BOT_TOKEN`: Your Telegram Bot Token obtained from BotFather.
    - `CUPS_PRINTER_NAME`: The name of your printer as configured in CUPS.
    - `ALLOWED_USER_IDS`: A comma-separated list of Telegram user IDs allowed to use the bot. Leave empty to allow all users (subject to guest printing limits).
    - `CUPS_SERVER_HOST` (Optional): The hostname or IP address if your CUPS server is running on a different machine than the bot.
    - `MAX_COPIES` (Optional): Set a default maximum number of copies allowed per print job. Defaults to 100 if not set.
    - `ALLOW_GUEST_PRINTING` (Optional): Set to `False`, `0`, or `No` to disable guest printing. Defaults to `True`.
    - `LABEL_WIDTH_INCHES` (Optional): The width of the label in inches. Defaults to 4 if not set.
    - `LABEL_HEIGHT_INCHES` (Optional): The height of the label in inches. Defaults to 6 if not set.

3.  **Build and Run with Docker Compose:** 🐳

    ```bash
    docker-compose up --build -d
    ```

    Or build and run locally with Cargo (requires Rust ≥ 1.85):

    ```bash
    cargo build --release
    ./target/release/telefax
    ```

## 🚀 Usage

1.  💬 **Start a chat** with your bot on Telegram.
2.  🖼️ **Send an image** to the bot.
3.  **(Optional)** Add a caption to the image specifying the number of copies. The caption must contain **only** the copy specifier:
    - `x3` — prints 3 copies
    - `copies=5` — prints 5 copies
      If no caption is provided, or the caption does not match the format, it defaults to 1 copy.
4.  🤖 The bot will resize the image to fit the configured label dimensions (default 4x6 inches) and send it to the configured CUPS printer.

### 🤖 Commands

- `/start`: Displays a welcome message 👋
- `/help`: Shows help information ℹ️
- `/setmaxcopies <number>`: (Authorized users only) Sets the maximum number of copies allowed per print job for the current session 👮

## 🙌 Contributing

Contributions are welcome! Please feel free to submit a pull request or open an issue.

## 📜 License

This project is licensed under the GNU General Public License v3.0 - see the [LICENSE](LICENSE) file for details.

Additionally, this project is a signatory of the [Pride Flag Covenant](https://github.com/Pride-Flag-Covenant/The-Pride-Flag-Convenant). We stand in solidarity with the LGBTQ+ community. 🏳️‍🌈 Trans Rights are Human Rights 🏳️‍⚧️

This software was written by a LGBTQ+ Person Using Aider.
