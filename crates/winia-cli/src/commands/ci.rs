//! 'winia ci' — write a CI workflow for the current project.
//!
//! Only the GitHub and GitLab shapes are generated. The workflow installs Winia Forge from the
//! configured source (a git URL by default) because the CLI is not published on crates.io yet.

use crate::cli::{CiArgs, CiCommand, CiInitArgs};
use anyhow::{Context, Result, bail};
use std::path::Path;

const DEFAULT_SOURCE: &str = "https://github.com/GrounzerLiu/winia-forge";

pub fn run(args: CiArgs) -> Result<()> {
    let init = match args.command {
        Some(CiCommand::Init(init)) => init,
        None => CiInitArgs {
            provider: "github".to_owned(),
            platforms: Vec::new(),
            winia_source: DEFAULT_SOURCE.to_owned(),
            force: false,
        },
    };
    let cwd = std::env::current_dir().context("reading the current directory")?;
    let config_path = crate::config::find_config(&cwd)
        .ok_or_else(|| anyhow::anyhow!("winia.toml not found; run this inside a Winia project"))?;
    let root = config_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .to_path_buf();

    let platforms = platforms(&root, &init.platforms)?;
    let provider = init.provider.to_lowercase();
    let source = if init.winia_source.trim().is_empty() {
        DEFAULT_SOURCE
    } else {
        init.winia_source.trim()
    };
    if source.contains('"') || source.contains('\n') || source.contains('\r') {
        bail!("--winia-source must not contain quotes or newlines");
    }
    let (relative, contents) = match provider.as_str() {
        "github" => (
            ".github/workflows/ci.yml",
            render_github(&platforms, source),
        ),
        "gitlab" => (".gitlab-ci.yml", render_gitlab(&platforms, source)),
        other => bail!("unknown CI provider '{other}'; expected github or gitlab"),
    };

    let destination = root.join(relative);
    if destination.exists() && !init.force {
        bail!("{} already exists; pass --force to overwrite it", relative);
    }
    if let Some(parent) = destination.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    std::fs::write(&destination, contents)
        .with_context(|| format!("writing {}", destination.display()))?;

    println!("Wrote {relative}");
    println!("Platforms: {}", platforms.join(", "));
    println!("Winia source: {source}");
    Ok(())
}

fn platforms(root: &Path, requested: &[String]) -> Result<Vec<String>> {
    let android = root.join("android").is_dir();
    let requested: Vec<String> = requested
        .iter()
        .map(|value| value.trim().to_lowercase())
        .filter(|value| !value.is_empty())
        .collect();
    let list = if requested.is_empty() {
        let mut defaults = vec!["windows".to_owned(), "macos".to_owned(), "linux".to_owned()];
        if android {
            defaults.push("android".to_owned());
        }
        defaults
    } else {
        requested
    };
    for platform in &list {
        match platform.as_str() {
            "windows" | "macos" | "linux" | "android" => {}
            other => {
                bail!("unknown platform '{other}'; expected windows, macos, linux, or android")
            }
        }
    }
    // Duplicate names would emit duplicate YAML keys.
    let mut unique: Vec<String> = Vec::new();
    for platform in list {
        if !unique.contains(&platform) {
            unique.push(platform);
        }
    }
    Ok(unique)
}

fn runner(platform: &str) -> &'static str {
    match platform {
        "windows" => "windows-latest",
        "macos" => "macos-latest",
        _ => "ubuntu-latest",
    }
}

/// GitHub workflow: one job per desktop platform plus an optional Android job.
fn render_github(platforms: &[String], source: &str) -> String {
    let mut out = String::new();
    out.push_str("name: CI\n\n");
    out.push_str("on:\n  push:\n  pull_request:\n  workflow_dispatch:\n\n");
    out.push_str("permissions:\n  contents: read\n\n");
    out.push_str("jobs:\n");

    for platform in platforms {
        if platform == "android" {
            continue;
        }
        let runner = runner(platform);
        out.push_str(&format!("  desktop-{platform}:\n"));
        out.push_str(&format!("    name: Desktop ({platform})\n"));
        out.push_str(&format!("    runs-on: {runner}\n"));
        out.push_str("    steps:\n");
        out.push_str("      - uses: actions/checkout@v4\n");
        out.push_str("      - uses: dtolnay/rust-toolchain@stable\n");
        out.push_str("        with:\n          components: rustfmt, clippy\n");
        out.push_str("      - uses: Swatinem/rust-cache@v2\n");
        out.push_str("      - name: Install Winia Forge\n");
        out.push_str(&format!(
            "        run: cargo install --git \"{source}\" winia-cli\n"
        ));
        out.push_str("      - run: cargo fmt --all -- --check\n");
        out.push_str("      - run: cargo clippy --all-targets -- -D warnings\n");
        out.push_str("      - run: cargo test\n");
        out.push_str("      - name: Package\n        run: winia package --release\n");
        out.push_str("      - uses: actions/upload-artifact@v4\n");
        out.push_str("        with:\n");
        out.push_str(&format!("          name: dist-{platform}\n"));
        out.push_str("          path: dist/\n\n");
    }

    if platforms.iter().any(|platform| platform == "android") {
        out.push_str("  android:\n");
        out.push_str("    name: Android\n");
        out.push_str("    runs-on: ubuntu-latest\n");
        out.push_str("    steps:\n");
        out.push_str("      - uses: actions/checkout@v4\n");
        out.push_str("      - uses: dtolnay/rust-toolchain@stable\n");
        out.push_str(
            "        with:\n          targets: aarch64-linux-android,x86_64-linux-android\n",
        );
        out.push_str("      - uses: actions/setup-java@v4\n");
        out.push_str(
            "        with:\n          distribution: temurin\n          java-version: \"21\"\n",
        );
        out.push_str("      - uses: android-actions/setup-android@v3\n");
        out.push_str("      - name: Install SDK packages\n");
        out.push_str("        run: sdkmanager \"platforms;android-36\" \"build-tools;36.0.0\" \"ndk;27.0.12077973\"\n");
        out.push_str("      - uses: Swatinem/rust-cache@v2\n");
        out.push_str("      - name: Install Winia Forge\n");
        out.push_str(&format!(
            "        run: cargo install --git \"{source}\" winia-cli\n"
        ));
        out.push_str(
            "      - name: Package\n        run: winia package --target android --release\n",
        );
        out.push_str("      - uses: actions/upload-artifact@v4\n");
        out.push_str("        with:\n          name: dist-android\n          path: dist/\n");
    }

    out
}

/// GitLab CI: shell-runner jobs, one per platform.
fn render_gitlab(platforms: &[String], source: &str) -> String {
    let mut out = String::new();
    out.push_str("stages:\n  - desktop\n  - android\n\n");

    out.push_str(".desktop:\n");
    out.push_str("  stage: desktop\n");
    out.push_str("  image: rust:1.85-bookworm\n");
    out.push_str("  before_script:\n");
    out.push_str("    - rustup component add clippy rustfmt\n");
    out.push_str("  script:\n");
    out.push_str(&format!(
        "    - cargo install --git \"{source}\" winia-cli\n"
    ));
    out.push_str("    - cargo fmt --all -- --check\n");
    out.push_str("    - cargo clippy --all-targets -- -D warnings\n");
    out.push_str("    - cargo test\n");
    out.push_str("    - winia package --release\n");
    out.push_str("  artifacts:\n    paths:\n      - dist/\n\n");

    for platform in platforms {
        if platform == "android" {
            continue;
        }
        out.push_str(&format!("desktop-{platform}:\n  extends: .desktop\n"));
        out.push_str(&format!("  tags: [\"{platform}\"]\n\n"));
    }

    if platforms.iter().any(|platform| platform == "android") {
        out.push_str("android:\n");
        out.push_str("  stage: android\n");
        out.push_str("  image: ubuntu:24.04\n");
        out.push_str("  variables:\n");
        out.push_str("    ANDROID_HOME: \"$CI_PROJECT_DIR/.android-sdk\"\n");
        out.push_str("    ANDROID_SDK_ROOT: \"$CI_PROJECT_DIR/.android-sdk\"\n");
        out.push_str("  before_script:\n");
        out.push_str("    - apt-get update && apt-get install -y --no-install-recommends openjdk-21-jdk unzip curl ca-certificates git build-essential\n");
        out.push_str("    - curl -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain 1.85.0 --profile minimal\n");
        out.push_str("    - . \"$HOME/.cargo/env\"\n");
        out.push_str("    - rustup target add aarch64-linux-android x86_64-linux-android\n");
        out.push_str("    - curl -sSLo /tmp/cmdline-tools.zip https://dl.google.com/android/repository/commandlinetools-linux-11076708_latest.zip\n");
        out.push_str("    - mkdir -p \"$ANDROID_HOME/cmdline-tools\" && unzip -q /tmp/cmdline-tools.zip -d \"$ANDROID_HOME/cmdline-tools\"\n");
        out.push_str("    - mv \"$ANDROID_HOME/cmdline-tools/cmdline-tools\" \"$ANDROID_HOME/cmdline-tools/latest\"\n");
        out.push_str("    - yes | \"$ANDROID_HOME/cmdline-tools/latest/bin/sdkmanager\" --licenses > /dev/null || true\n");
        out.push_str("    - \"$ANDROID_HOME/cmdline-tools/latest/bin/sdkmanager\" \"platforms;android-36\" \"build-tools;36.0.0\" \"ndk;27.0.12077973\"\n");
        out.push_str("  script:\n");
        out.push_str(&format!(
            "    - cargo install --git \"{source}\" winia-cli\n"
        ));
        out.push_str("    - winia package --target android --release\n");
        out.push_str("  artifacts:\n    paths:\n      - dist/\n");
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn github_workflow_has_one_job_per_platform() {
        let workflow = render_github(
            &["windows".to_owned(), "macos".to_owned(), "linux".to_owned()],
            DEFAULT_SOURCE,
        );
        assert!(workflow.contains("desktop-windows:"));
        assert!(workflow.contains("runs-on: macos-latest"));
        assert!(workflow.contains("runs-on: ubuntu-latest"));
        assert!(!workflow.contains("  android:"));
        assert!(workflow.contains("winia package --release"));
    }

    #[test]
    fn github_workflow_adds_the_android_job() {
        let workflow = render_github(&["android".to_owned()], DEFAULT_SOURCE);
        assert!(workflow.contains("  android:"));
        assert!(workflow.contains("sdkmanager"));
        assert!(!workflow.contains("desktop-windows:"));
    }

    #[test]
    fn workflow_inlines_the_source_and_installs_android_targets() {
        let workflow = render_github(
            &["windows".to_owned(), "android".to_owned()],
            DEFAULT_SOURCE,
        );
        assert!(
            workflow.contains(&format!(
                "cargo install --git \"{DEFAULT_SOURCE}\" winia-cli"
            )),
            "the install step must not depend on a shell variable"
        );
        assert!(!workflow.contains("$WINIA_SOURCE"));
        assert!(workflow.contains("targets: aarch64-linux-android,x86_64-linux-android"));
    }

    #[test]
    fn gitlab_android_job_installs_rust_and_the_sdk() {
        let workflow = render_gitlab(&["android".to_owned()], DEFAULT_SOURCE);
        assert!(workflow.contains("image: rust:1.85-bookworm") || workflow.contains("rustup"));
        assert!(workflow.contains("rustup target add aarch64-linux-android"));
        assert!(workflow.contains("sdkmanager"));
        assert!(!workflow.contains("$WINIA_SOURCE"));
    }

    #[test]
    fn duplicate_platforms_are_collapsed() {
        let root = std::env::temp_dir().join(format!("winia-ci-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&root);
        let list = platforms(&root, &["windows".to_owned(), "windows".to_owned()]).unwrap();
        assert_eq!(list, vec!["windows"]);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn rejects_unknown_platforms() {
        let root = std::env::current_dir().unwrap();
        assert!(platforms(&root, &["beos".to_owned()]).is_err());
    }
}
