//! Platform icon generation from one source PNG or SVG.
//!
//! The source is `app.icon`; generated files live under
//! `target/winia/icons` and are reused until the source hash changes.

use crate::config::WiniaConfig;
use anyhow::{Context, Result, bail};
use image::{ExtendedColorType, ImageEncoder, Rgba, RgbaImage, imageops::FilterType};
use resvg::{tiny_skia, usvg};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::io::Cursor;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct IconSet {
    pub windows_ico: PathBuf,
    pub macos_icns: PathBuf,
    pub linux_icons: PathBuf,
    pub android_res: PathBuf,
    pub store_png: PathBuf,
}

#[derive(Serialize, Deserialize)]
struct Marker {
    source_sha256: String,
}

pub fn ensure(root: &Path, config: &WiniaConfig) -> Result<Option<IconSet>> {
    let Some(source) = source_path(root, config)? else {
        return Ok(None);
    };
    let bytes = std::fs::read(&source)
        .with_context(|| format!("reading icon source {}", source.display()))?;
    let digest = format!("{:x}", Sha256::digest(&bytes));
    let icons = icon_root(root);
    if marker_matches(&icons, &digest) {
        return Ok(Some(paths(&icons)));
    }
    generate_from_bytes(root, config, &bytes, &source, &digest).map(Some)
}

pub fn generate(root: &Path, config: &WiniaConfig, source: Option<&Path>) -> Result<IconSet> {
    let source = match source {
        Some(path) => path.to_path_buf(),
        None => source_path(root, config)?
            .ok_or_else(|| anyhow::anyhow!("app.icon is not configured"))?,
    };
    let bytes = std::fs::read(&source)
        .with_context(|| format!("reading icon source {}", source.display()))?;
    let digest = format!("{:x}", Sha256::digest(&bytes));
    generate_from_bytes(root, config, &bytes, &source, &digest)
}

fn source_path(root: &Path, config: &WiniaConfig) -> Result<Option<PathBuf>> {
    let Some(value) = config.app.icon.as_deref() else {
        return Ok(None);
    };
    let path = PathBuf::from(value);
    let path = if path.is_absolute() {
        path
    } else {
        root.join(path)
    };
    if !path.is_file() {
        bail!("app.icon not found: {}", path.display());
    }
    Ok(Some(path))
}

fn generate_from_bytes(
    root: &Path,
    config: &WiniaConfig,
    bytes: &[u8],
    source: &Path,
    digest: &str,
) -> Result<IconSet> {
    let icons = icon_root(root);
    std::fs::create_dir_all(&icons).with_context(|| format!("creating {}", icons.display()))?;
    let source = Source::load(bytes, source)?;

    let windows_ico = icons.join("windows/app.ico");
    let macos_icns = icons.join("macos/app.icns");
    let linux_icons = icons.join("linux/share/icons");
    let android_res = icons.join("android/res");
    let store_png = icons.join("store/icon-512.png");

    if let Some(parent) = windows_ico.parent() {
        std::fs::create_dir_all(parent)?;
    }
    if let Some(parent) = macos_icns.parent() {
        std::fs::create_dir_all(parent)?;
    }
    write_ico(&windows_ico, &source)?;
    write_icns(&macos_icns, &source)?;
    write_linux(&linux_icons, &source, &config.app.id)?;
    write_android(&android_res, &source)?;
    write_png(&store_png, &render(&source, 512)?)?;

    std::fs::write(
        icons.join("manifest.json"),
        serde_json::to_string_pretty(&Marker {
            source_sha256: digest.to_owned(),
        })?,
    )?;
    Ok(paths(&icons))
}

fn paths(icons: &Path) -> IconSet {
    IconSet {
        windows_ico: icons.join("windows/app.ico"),
        macos_icns: icons.join("macos/app.icns"),
        linux_icons: icons.join("linux/share/icons"),
        android_res: icons.join("android/res"),
        store_png: icons.join("store/icon-512.png"),
    }
}

fn icon_root(root: &Path) -> PathBuf {
    root.join("target").join("winia").join("icons")
}

fn marker_matches(icons: &Path, digest: &str) -> bool {
    let marker = icons.join("manifest.json");
    let Ok(text) = std::fs::read_to_string(marker) else {
        return false;
    };
    let Ok(marker) = serde_json::from_str::<Marker>(&text) else {
        return false;
    };
    marker.source_sha256 == digest
        && icons.join("windows/app.ico").is_file()
        && icons.join("macos/app.icns").is_file()
        && icons.join("store/icon-512.png").is_file()
}

enum Source {
    Bitmap(RgbaImage),
    Svg(Box<usvg::Tree>),
}

impl Source {
    fn load(bytes: &[u8], path: &Path) -> Result<Self> {
        let extension = path
            .extension()
            .and_then(|extension| extension.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        if extension == "svg" {
            let options = usvg::Options::default();
            let tree = usvg::Tree::from_data(bytes, &options)
                .with_context(|| format!("parsing SVG icon {}", path.display()))?;
            Ok(Source::Svg(Box::new(tree)))
        } else {
            let image = image::load_from_memory(bytes)
                .with_context(|| format!("decoding icon {}", path.display()))?
                .to_rgba8();
            Ok(Source::Bitmap(image))
        }
    }
}

fn render(source: &Source, size: u32) -> Result<RgbaImage> {
    match source {
        Source::Bitmap(image) => Ok(resize_square(image, size)),
        Source::Svg(tree) => {
            let svg = tree.size();
            let scale = size as f32 / svg.width().max(svg.height());
            let x = (size as f32 - svg.width() * scale) / 2.0;
            let y = (size as f32 - svg.height() * scale) / 2.0;
            let mut pixmap = tiny_skia::Pixmap::new(size, size)
                .ok_or_else(|| anyhow::anyhow!("cannot allocate {size}x{size} icon canvas"))?;
            resvg::render(
                tree,
                tiny_skia::Transform::from_scale(scale, scale).post_translate(x, y),
                &mut pixmap.as_mut(),
            );
            let png = pixmap.encode_png().context("encoding rendered SVG")?;
            Ok(image::load_from_memory(&png)?.to_rgba8())
        }
    }
}

fn resize_square(image: &RgbaImage, size: u32) -> RgbaImage {
    let (width, height) = image.dimensions();
    let scale = size as f32 / width.max(height) as f32;
    let width = (width as f32 * scale).round().max(1.0) as u32;
    let height = (height as f32 * scale).round().max(1.0) as u32;
    let resized = image::imageops::resize(image, width, height, FilterType::Lanczos3);
    let mut canvas = RgbaImage::from_pixel(size, size, Rgba([0, 0, 0, 0]));
    image::imageops::overlay(
        &mut canvas,
        &resized,
        ((size - width) / 2) as i64,
        ((size - height) / 2) as i64,
    );
    canvas
}

fn write_png(path: &Path, image: &RgbaImage) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let file =
        std::fs::File::create(path).with_context(|| format!("creating {}", path.display()))?;
    image::codecs::png::PngEncoder::new(file)
        .write_image(
            image.as_raw(),
            image.width(),
            image.height(),
            ExtendedColorType::Rgba8,
        )
        .with_context(|| format!("writing {}", path.display()))?;
    Ok(())
}

fn png_bytes(image: &RgbaImage) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    image::codecs::png::PngEncoder::new(&mut bytes)
        .write_image(
            image.as_raw(),
            image.width(),
            image.height(),
            ExtendedColorType::Rgba8,
        )
        .context("encoding icon PNG")?;
    Ok(bytes)
}

fn write_ico(path: &Path, source: &Source) -> Result<()> {
    let mut directory = ico::IconDir::new(ico::ResourceType::Icon);
    for size in [16u32, 24, 32, 48, 64, 128, 256] {
        let image = render(source, size)?;
        let ico_image = ico::IconImage::from_rgba_data(size, size, image.into_raw());
        directory.add_entry(ico::IconDirEntry::encode_as_png(&ico_image)?);
    }
    directory.write(std::fs::File::create(path)?)?;
    Ok(())
}

fn write_icns(path: &Path, source: &Source) -> Result<()> {
    let mut family = icns::IconFamily::new();
    for size in [16u32, 32, 64, 128, 256, 512, 1024] {
        let image = render(source, size)?;
        let encoded = icns::Image::read_png(Cursor::new(png_bytes(&image)?))?;
        family.add_icon(&encoded)?;
    }
    family.write(std::fs::File::create(path)?)?;
    Ok(())
}

fn write_linux(root: &Path, source: &Source, name: &str) -> Result<()> {
    if root.exists() {
        std::fs::remove_dir_all(root)?;
    }
    let file_name = format!("{name}.png");
    for size in [16u32, 24, 32, 48, 64, 128, 256, 512] {
        let path = root
            .join(format!("hicolor/{size}x{size}/apps"))
            .join(&file_name);
        write_png(&path, &render(source, size)?)?;
    }
    Ok(())
}

fn write_android(root: &Path, source: &Source) -> Result<()> {
    if root.exists() {
        std::fs::remove_dir_all(root)?;
    }
    for (density, size) in [
        ("mdpi", 48u32),
        ("hdpi", 72),
        ("xhdpi", 96),
        ("xxhdpi", 144),
        ("xxxhdpi", 192),
    ] {
        let directory = root.join(format!("mipmap-{density}"));
        write_png(&directory.join("ic_launcher.png"), &render(source, size)?)?;
        write_png(
            &directory.join("ic_launcher_round.png"),
            &render(source, size)?,
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generates_platform_icons_from_png() {
        let root = std::env::temp_dir().join(format!("winia-icons-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("assets")).unwrap();
        let image = RgbaImage::from_pixel(64, 64, Rgba([10, 20, 30, 255]));
        write_png(&root.join("assets/icon.png"), &image).unwrap();
        let mut config = WiniaConfig::default();
        config.app.icon = Some("assets/icon.png".to_owned());
        let icons = generate(&root, &config, None).unwrap();
        assert!(icons.windows_ico.is_file());
        assert!(icons.macos_icns.is_file());
        assert!(
            icons
                .android_res
                .join("mipmap-mdpi/ic_launcher.png")
                .is_file()
        );
        assert!(icons.store_png.is_file());
        std::fs::remove_dir_all(&root).unwrap();
    }
}
