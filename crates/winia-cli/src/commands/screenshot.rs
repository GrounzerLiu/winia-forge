//! 'winia screenshot' captures the device screen through adb.
//!
//! This is the standalone capture path: it needs no debug server, so it works for any installed
//! build. 'winia inspect screenshot' remains the option when the app exposes a debug server and
//! the frame should come from the renderer.

use crate::cli::ScreenshotArgs;
use crate::commands::run;
use anyhow::{Context, Result, bail};
use std::path::{Path, PathBuf};
use std::time::Instant;

pub fn run(args: ScreenshotArgs) -> Result<()> {
    let started = Instant::now();
    if let Some(target) = args.target.as_deref() {
        if target != "android" {
            bail!("screenshots are only implemented for android, not '{target}'");
        }
    }
    let device = run::select_device(args.device.as_deref())?;
    let output = args
        .output
        .clone()
        .unwrap_or_else(|| PathBuf::from("screenshot.png"));
    let bytes = run::adb_binary(&["-s", &device, "exec-out", "screencap", "-p"])?;
    if !is_png(&bytes) {
        bail!(
            "'adb exec-out screencap -p' returned {} bytes that are not a PNG",
            bytes.len()
        );
    }
    if let Some(parent) = output
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    std::fs::write(&output, &bytes).with_context(|| format!("writing {}", output.display()))?;

    let (width, height) = png_size(&bytes).unwrap_or((0, 0));
    if args.json {
        let value = serde_json::json!({
            "device": device,
            "path": output.display().to_string(),
            "bytes": bytes.len(),
            "width": width,
            "height": height,
            "duration_ms": started.elapsed().as_millis(),
        });
        println!("{}", serde_json::to_string_pretty(&value)?);
        return Ok(());
    }
    println!(
        "Saved {} ({}x{}, {:.1} KB) from {}",
        output.display(),
        width,
        height,
        bytes.len() as f64 / 1024.0,
        device
    );
    Ok(())
}

fn is_png(bytes: &[u8]) -> bool {
    bytes.starts_with(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A])
}

/// PNG stores width and height as big-endian u32 right after the IHDR chunk header.
fn png_size(bytes: &[u8]) -> Option<(u32, u32)> {
    if !is_png(bytes) || bytes.len() < 24 {
        return None;
    }
    let width = u32::from_be_bytes(bytes[16..20].try_into().ok()?);
    let height = u32::from_be_bytes(bytes[20..24].try_into().ok()?);
    Some((width, height))
}

/// Kept for tests and callers that only need the file name.
pub fn default_output() -> PathBuf {
    Path::new("screenshot.png").to_path_buf()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognises_a_png_header() {
        let mut bytes = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
        bytes.extend_from_slice(&[0, 0, 0, 13]);
        bytes.extend_from_slice(b"IHDR");
        bytes.extend_from_slice(&1080u32.to_be_bytes());
        bytes.extend_from_slice(&1920u32.to_be_bytes());
        assert!(is_png(&bytes));
        assert_eq!(png_size(&bytes), Some((1080, 1920)));
        assert!(!is_png(b"not a png"));
        assert_eq!(png_size(b"short"), None);
    }

    #[test]
    fn default_output_is_a_png() {
        assert_eq!(default_output(), PathBuf::from("screenshot.png"));
    }
}
