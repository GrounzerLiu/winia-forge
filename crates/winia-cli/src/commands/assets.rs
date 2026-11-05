//! 'winia assets icon' — generate platform icon sets from one source.
//!
//! The same generator is used automatically by build/package from `app.icon`.

use crate::cli::{AssetsArgs, AssetsCommand};
use crate::config::WiniaConfig;
use anyhow::{Context, Result, bail};

pub fn run(args: AssetsArgs) -> Result<()> {
    let cwd = std::env::current_dir().context("reading the current directory")?;
    let config_path = crate::config::find_config(&cwd)
        .ok_or_else(|| anyhow::anyhow!("winia.toml not found; run this inside a Winia project"))?;
    let root = config_path
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."))
        .to_path_buf();
    let config = WiniaConfig::load(&config_path)?;
    config.validate()?;

    match args.command {
        Some(AssetsCommand::Icon(icon)) => {
            let source = if icon.source.is_absolute() {
                icon.source
            } else {
                cwd.join(icon.source)
            };
            let icons = crate::icons::generate(&root, &config, Some(&source))?;
            // Persist the choice: otherwise the next build falls back to the built-in default and
            // silently replaces these icons.
            let relative = source
                .strip_prefix(&root)
                .map(|path| path.to_string_lossy().replace('\\', "/"))
                .unwrap_or_else(|_| source.to_string_lossy().into_owned());
            crate::config::set_app_icon(&config_path, &relative)?;
            println!("Updated app.icon = \"{relative}\" in winia.toml");
            print_icons(&icons);
        }
        None => {
            let Some(icons) = crate::icons::ensure(&root, &config)? else {
                bail!("app.icon is not configured in winia.toml");
            };
            print_icons(&icons);
        }
    }
    Ok(())
}

fn print_icons(icons: &crate::icons::IconSet) {
    println!("Generated icons:");
    println!("  windows: {}", icons.windows_ico.display());
    println!("  macOS:   {}", icons.macos_icns.display());
    println!("  Linux:   {}", icons.linux_icons.display());
    println!("  Android: {}", icons.android_res.display());
    println!("  store:   {}", icons.store_png.display());
}
