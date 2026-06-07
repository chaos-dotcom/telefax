use crate::config::Config;
use anyhow::{Context, Result};
use image::ImageFormat;
use log::{error, info};
use std::io::Cursor;
use tokio::process::Command;

pub fn resize_image(image_bytes: &[u8], config: &Config) -> Result<(Vec<u8>, String)> {
    info!("[resize_image] Loading image from memory | input_bytes={}", image_bytes.len());
    let img = image::load_from_memory(image_bytes).context("Failed to load image from memory")?;
    info!("[resize_image] Loaded | width={} height={} color={:?}", img.width(), img.height(), img.color());

    let target_w = config.label_width_px;
    let target_h = config.label_height_px;

    // Crop-to-fill: scale the image so it completely covers the label dimensions,
    // then crop from the centre. This guarantees zero white margins regardless of
    // the source image's aspect ratio.
    let scale_x = target_w as f64 / img.width() as f64;
    let scale_y = target_h as f64 / img.height() as f64;
    let scale = scale_x.max(scale_y);

    let cover_w = (img.width() as f64 * scale).ceil() as u32;
    let cover_h = (img.height() as f64 * scale).ceil() as u32;
    info!("[resize_image] Cover resize | scale={:.3} dims={}x{}", scale, cover_w, cover_h);

    let resized = img.resize(cover_w, cover_h, image::imageops::FilterType::Lanczos3);

    let crop_x = (resized.width().saturating_sub(target_w)) / 2;
    let crop_y = (resized.height().saturating_sub(target_h)) / 2;
    info!("[resize_image] Center crop | x={} y={} output={}x{}", crop_x, crop_y, target_w, target_h);

    let use_png = matches!(
        resized.color(),
        image::ColorType::Rgba8
            | image::ColorType::Rgba16
            | image::ColorType::Rgba32F
            | image::ColorType::La8
            | image::ColorType::La16
    );
    info!("[resize_image] Output format | use_png={} color={:?}", use_png, resized.color());

    let mut output_buffer = Vec::new();
    let format = if use_png {
        let mut rgba = resized.to_rgba8();
        let cropped = image::imageops::crop(&mut rgba, crop_x, crop_y, target_w, target_h);
        let cropped_img = cropped.to_image();
        cropped_img.write_to(&mut Cursor::new(&mut output_buffer), ImageFormat::Png)?;
        "png".to_string()
    } else {
        let mut rgb = resized.to_rgb8();
        let cropped = image::imageops::crop(&mut rgb, crop_x, crop_y, target_w, target_h);
        let cropped_img = cropped.to_image();
        cropped_img.write_to(&mut Cursor::new(&mut output_buffer), ImageFormat::Jpeg)?;
        "jpeg".to_string()
    };

    info!("[resize_image] Done | format={} output_bytes={} exact_dims={}x{}",
        format, output_buffer.len(), target_w, target_h);
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
    // scaling=100 + ppi=300 tells CUPS to print the image at its natural size
    // assuming 300 DPI. Since our processed image is always exactly
    // label_width_px × label_height_px (e.g. 1200×1800), this comes out to
    // exactly 4×6 inches — one label, no scaling, no fitting to a smaller
    // "printable area" (which is what fit-to-page does and why it left margins).
    args.push("-o".to_string());
    args.push("scaling=100".to_string());
    args.push("-o".to_string());
    args.push("ppi=300".to_string());
    // Force zero margins so the image starts at the physical edge.
    args.push("-o".to_string());
    args.push("page-left=0".to_string());
    args.push("-o".to_string());
    args.push("page-right=0".to_string());
    args.push("-o".to_string());
    args.push("page-top=0".to_string());
    args.push("-o".to_string());
    args.push("page-bottom=0".to_string());
    // Hard safety: never allow more than 1 page even if the driver miscalculates.
    args.push("-o".to_string());
    args.push("page-ranges=1".to_string());
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
    info!("[print_image_cups] Creating temp file | suffix={}", suffix);
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
    info!("[print_image_cups] Temp file | path={} size={}", path, image_buffer.len());

    let args = build_lp_args(printer_name, copies, image_format, config, path);
    info!("[print_image_cups] Spawning | {}", args.join(" "));

    let output = Command::new(&args[0])
        .args(&args[1..])
        .output()
        .await
        .context("Failed to execute lp command")?;

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    let code = output.status.code().unwrap_or(-1);

    if output.status.success() {
        info!("[print_image_cups] SUCCESS | exit={} stdout='{}' stderr='{}'", code, stdout.trim(), stderr.trim());
        Ok(stdout)
    } else {
        error!(
            "[print_image_cups] FAILURE | exit={} stdout='{}' stderr='{}'",
            code, stdout.trim(), stderr.trim()
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
            rate_limit_override_start: None,
            rate_limit_override_end: None,
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

        // Image is padded to exact label dimensions
        let result_img = image::load_from_memory(&out).unwrap();
        assert_eq!(result_img.width(), config.label_width_px);
        assert_eq!(result_img.height(), config.label_height_px);
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
        assert_eq!(result_img.width(), config.label_width_px);
        assert_eq!(result_img.height(), config.label_height_px);
    }

    #[test]
    fn resize_small_image_scaled_up_to_label_size() {
        let img = image::RgbImage::new(100, 100);
        let mut buf = Vec::new();
        img.write_to(&mut Cursor::new(&mut buf), ImageFormat::Png).unwrap();

        let config = test_config();
        let (out, _) = resize_image(&buf, &config).unwrap();
        let result_img = image::load_from_memory(&out).unwrap();
        // Small image is scaled up and cropped to exact label dimensions
        assert_eq!(result_img.width(), config.label_width_px);
        assert_eq!(result_img.height(), config.label_height_px);
    }

    #[test]
    fn resize_large_image_cover_cropped_to_label_size() {
        let mut img = image::RgbImage::new(3000, 4000);
        for pixel in img.pixels_mut() {
            *pixel = image::Rgb([50, 100, 150]);
        }
        let mut buf = Vec::new();
        img.write_to(&mut Cursor::new(&mut buf), ImageFormat::Jpeg).unwrap();

        let config = test_config();
        let (out, fmt) = resize_image(&buf, &config).unwrap();
        assert_eq!(fmt, "jpeg");
        let result_img = image::load_from_memory(&out).unwrap();
        assert_eq!(result_img.width(), config.label_width_px);
        assert_eq!(result_img.height(), config.label_height_px);
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
        assert!(args.contains(&"scaling=100".to_string()));
        assert!(args.contains(&"ppi=300".to_string()));
        assert!(args.contains(&"page-left=0".to_string()));
        assert!(args.contains(&"page-right=0".to_string()));
        assert!(args.contains(&"page-top=0".to_string()));
        assert!(args.contains(&"page-bottom=0".to_string()));
        assert!(args.contains(&"page-ranges=1".to_string()));
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
