use crate::config::Config;
use anyhow::{Context, Result};
use image::ImageFormat;
use log::{error, info};
use std::io::Cursor;
use tokio::process::Command;

pub fn resize_image(image_bytes: &[u8], config: &Config) -> Result<(Vec<u8>, String)> {
    let img = image::load_from_memory(image_bytes).context("Failed to load image from memory")?;

    let resized = if img.width() > config.label_width_px || img.height() > config.label_height_px {
        img.thumbnail(config.label_width_px, config.label_height_px)
    } else {
        img
    };

    let use_png = matches!(
        resized.color(),
        image::ColorType::Rgba8
            | image::ColorType::Rgba16
            | image::ColorType::Rgba32F
            | image::ColorType::La8
            | image::ColorType::La16
    );

    let mut output_buffer = Vec::new();
    let format = if use_png {
        resized.write_to(
            &mut Cursor::new(&mut output_buffer),
            ImageFormat::Png,
        )?;
        "png".to_string()
    } else {
        // Convert to RGB8 for JPEG to avoid issues with grayscale or other modes
        let rgb_img = resized.to_rgb8();
        rgb_img.write_to(
            &mut Cursor::new(&mut output_buffer),
            ImageFormat::Jpeg,
        )?;
        "jpeg".to_string()
    };

    Ok((output_buffer, format))
}

/// Builds the `lp` command arguments for CUPS printing.
/// Exposed for unit testing.
pub fn build_lp_args(
    printer_name: &str,
    copies: usize,
    image_format: &str,
    config: &Config,
    temp_path: &str,
) -> Vec<String> {
    let mut args = vec!["lp".to_string()];

    if let Some(host) = &config.cups_server_host {
        args.push("-h".to_string());
        args.push(host.clone());
    }

    args.push("-d".to_string());
    args.push(printer_name.to_string());
    args.push("-n".to_string());
    args.push(copies.to_string());

    let width_str = format!("{:.2}", config.label_width_inches).replace(".00", "");
    let height_str = format!("{:.2}", config.label_height_inches).replace(".00", "");
    let media_option = format!("media=Custom.{}x{}in", width_str, height_str);
    args.push("-o".to_string());
    args.push(media_option);
    args.push("-o".to_string());
    args.push("fit-to-page".to_string());
    args.push(temp_path.to_string());

    // image_format is currently only used for the temp file suffix in the caller.
    // We include it in the returned args as a comment-like marker at the end
    // so tests can assert the format was considered if we ever need it.
    // For now we just ignore it in the lp command.
    let _ = image_format;

    args
}

pub async fn print_image_cups(
    image_buffer: &[u8],
    printer_name: &str,
    copies: usize,
    image_format: &str,
    config: &Config,
) -> Result<String> {
    let suffix = format!(".{}", image_format);
    let mut temp_file = tempfile::Builder::new()
        .suffix(&suffix)
        .tempfile()
        .context("Failed to create temporary file for printing")?;

    std::io::Write::write_all(&mut temp_file, image_buffer)
        .context("Failed to write image to temporary file")?;
    std::io::Write::flush(&mut temp_file).context("Failed to flush temporary file")?;

    let path = temp_file
        .path()
        .to_str()
        .context("Temporary file path is not valid UTF-8")?;

    let args = build_lp_args(printer_name, copies, image_format, config, path);
    info!("Executing CUPS command: {}", args.join(" "));

    let output = Command::new(&args[0])
        .args(&args[1..])
        .output()
        .await
        .context("Failed to execute lp command")?;

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();

    if !stdout.is_empty() {
        info!("CUPS Output: {}", stdout);
    }
    if !stderr.is_empty() {
        info!("CUPS Error Output: {}", stderr);
    }

    if output.status.success() {
        Ok(stdout)
    } else {
        let code = output.status.code().unwrap_or(-1);
        error!(
            "CUPS printing failed. Return code: {}. Stderr: {}",
            code, stderr
        );
        anyhow::bail!("CUPS printing failed (code {}): {}", code, stderr)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config() -> Config {
        Config {
            telegram_bot_token: "test".to_string(),
            cups_printer_name: "TestPrinter".to_string(),
            cups_server_host: None,
            allowed_user_ids: vec![],
            allow_guest_printing: true,
            max_copies: 100,
            label_width_inches: 4.0,
            label_height_inches: 6.0,
            label_width_px: 1200,
            label_height_px: 1800,
        }
    }

    #[test]
    fn resize_rgb_produces_jpeg() {
        let mut img = image::RgbImage::new(2000, 3000);
        // Fill with a color so it's not all zeros
        for pixel in img.pixels_mut() {
            *pixel = image::Rgb([100, 150, 200]);
        }
        let mut buf = Vec::new();
        img.write_to(&mut Cursor::new(&mut buf), ImageFormat::Jpeg).unwrap();

        let config = test_config();
        let (out, fmt) = resize_image(&buf, &config).unwrap();
        assert_eq!(fmt, "jpeg");
        assert!(!out.is_empty());

        // Verify dimensions are within bounds
        let result_img = image::load_from_memory(&out).unwrap();
        assert!(result_img.width() <= config.label_width_px);
        assert!(result_img.height() <= config.label_height_px);
    }

    #[test]
    fn resize_rgba_produces_png() {
        let mut img = image::RgbaImage::new(2000, 3000);
        for pixel in img.pixels_mut() {
            *pixel = image::Rgba([100, 150, 200, 128]);
        }
        let mut buf = Vec::new();
        img.write_to(&mut Cursor::new(&mut buf), ImageFormat::Png).unwrap();

        let config = test_config();
        let (out, fmt) = resize_image(&buf, &config).unwrap();
        assert_eq!(fmt, "png");
        assert!(!out.is_empty());

        let result_img = image::load_from_memory(&out).unwrap();
        assert!(result_img.width() <= config.label_width_px);
        assert!(result_img.height() <= config.label_height_px);
    }

    #[test]
    fn resize_small_image_not_upscaled() {
        let img = image::RgbImage::new(100, 100);
        let mut buf = Vec::new();
        img.write_to(&mut Cursor::new(&mut buf), ImageFormat::Png).unwrap();

        let config = test_config();
        let (out, _) = resize_image(&buf, &config).unwrap();
        let result_img = image::load_from_memory(&out).unwrap();
        assert_eq!(result_img.width(), 100);
        assert_eq!(result_img.height(), 100);
    }

    #[test]
    fn build_lp_args_basic() {
        let config = test_config();
        let args = build_lp_args("MyPrinter", 3, "png", &config, "/tmp/test.png");
        assert_eq!(args[0], "lp");
        assert!(args.contains(&"-d".to_string()));
        assert!(args.contains(&"MyPrinter".to_string()));
        assert!(args.contains(&"-n".to_string()));
        assert!(args.contains(&"3".to_string()));
        assert!(args.contains(&"-o".to_string()));
        assert!(args.contains(&"media=Custom.4x6in".to_string()));
        assert!(args.contains(&"fit-to-page".to_string()));
        assert!(args.contains(&"/tmp/test.png".to_string()));
    }

    #[test]
    fn build_lp_args_with_remote_host() {
        let mut config = test_config();
        config.cups_server_host = Some("cups.remote".to_string());
        let args = build_lp_args("MyPrinter", 1, "jpeg", &config, "/tmp/test.jpg");
        assert!(args.contains(&"-h".to_string()));
        assert!(args.contains(&"cups.remote".to_string()));
    }

    #[test]
    fn build_lp_args_strips_trailing_zeros() {
        let mut config = test_config();
        config.label_width_inches = 4.50;
        config.label_height_inches = 6.00;
        let args = build_lp_args("P", 1, "png", &config, "/tmp/t.png");
        assert!(args.contains(&"media=Custom.4.50x6in".to_string()));
    }
}
