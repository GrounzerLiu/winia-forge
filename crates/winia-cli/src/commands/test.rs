//! 'winia test' — run host tests and Android smoke checks.
//!
//! Host tests delegate to Cargo. Android smoke tests build the debug APK, install
//! it, launch it, check logcat for panics, and capture one device screenshot.

use crate::cli::{BuildArgs, TestArgs};
use crate::commands::{build, run};
use crate::config::WiniaConfig;
use anyhow::{Context, Result, bail};
use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

pub fn run(args: TestArgs) -> Result<()> {
    let cwd = std::env::current_dir().context("reading the current directory")?;
    let config_path = crate::config::find_config(&cwd)
        .ok_or_else(|| anyhow::anyhow!("winia.toml not found; run this inside a Winia project"))?;
    let root = config_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .to_path_buf();
    let config = WiniaConfig::load(&config_path)?;
    config.validate()?;

    let target = args
        .target
        .clone()
        .unwrap_or_else(|| build::host_target().to_owned());
    if target == "android" {
        return android_smoke(&root, &config, &args);
    }
    if target != build::host_target() {
        bail!(
            "testing the {target} target on {} is not supported; run it on that platform",
            build::host_target()
        );
    }
    host_tests(&root, &config, &args)
}

fn host_tests(root: &Path, config: &WiniaConfig, args: &TestArgs) -> Result<()> {
    let started = Instant::now();
    let mut features = config.build.features.clone();
    for feature in &args.features {
        if !features.iter().any(|existing| existing == feature) {
            features.push(feature.clone());
        }
    }

    let mut command = Command::new("cargo");
    command.current_dir(root).arg("test");
    if args.release {
        command.arg("--release");
    }
    if !features.is_empty() {
        command.arg("--features").arg(features.join(","));
    }
    if let Some(filter) = &args.filter {
        command.arg(filter);
    }
    if args.update_snapshots {
        command.env("WINIA_UPDATE_SNAPSHOTS", "1");
        eprintln!("note: WINIA_UPDATE_SNAPSHOTS is set; the current test suite must consume it");
    }

    if args.json {
        let output = command.output().context("running cargo test")?;
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        let (passed, failed, ignored) = test_counts(&text);
        let success = output.status.success();
        let value = serde_json::json!({
            "target": "host",
            "passed": passed,
            "failed": failed,
            "ignored": ignored,
            "success": success,
            "exit_code": output.status.code(),
            "duration_ms": started.elapsed().as_millis(),
        });
        println!("{}", serde_json::to_string_pretty(&value)?);
        if !success {
            eprintln!("{text}");
            bail!("tests failed");
        }
        return Ok(());
    }

    let status = command.status().context("running cargo test")?;
    if !status.success() {
        bail!("tests failed with {status}");
    }
    println!("Tests passed in {:.1}s", started.elapsed().as_secs_f64());
    Ok(())
}

fn test_counts(text: &str) -> (u64, u64, u64) {
    let mut passed = 0;
    let mut failed = 0;
    let mut ignored = 0;
    for line in text.lines() {
        if !line.contains("test result:") {
            continue;
        }
        for part in line.split(';') {
            let words: Vec<&str> = part.split_whitespace().collect();
            for pair in words.windows(2) {
                let value = pair[0].parse::<u64>().unwrap_or(0);
                match pair[1] {
                    "passed" => passed += value,
                    "failed" => failed += value,
                    "ignored" => ignored += value,
                    _ => {}
                }
            }
        }
    }
    (passed, failed, ignored)
}

fn android_smoke(root: &Path, config: &WiniaConfig, args: &TestArgs) -> Result<()> {
    let started = Instant::now();
    if args.filter.is_some() {
        eprintln!("note: --filter only applies to host tests; ignoring it for Android smoke");
    }
    if args.update_snapshots {
        eprintln!("note: --update-snapshots has no effect on Android smoke");
    }

    let device = run::select_device(args.device.as_deref())?;
    let abi = run::device_abi(&device)?;
    if !args.json {
        println!("Device {device} uses {}", abi.as_str());
    }

    let mut features = config.build.features.clone();
    for feature in &args.features {
        if !features.iter().any(|existing| existing == feature) {
            features.push(feature.clone());
        }
    }
    let built = build::build_all(
        root,
        config,
        &BuildArgs {
            target: vec!["android".to_owned()],
            release: args.release,
            abi: vec![abi.as_str().to_owned()],
            features,
            json: args.json,
            out_dir: None,
        },
    )?;
    let apk = built
        .iter()
        .flat_map(|(_, artifacts)| artifacts)
        .find(|artifact| run::extension(&artifact.path) == Some("apk"))
        .ok_or_else(|| anyhow::anyhow!("the build produced no APK"))?;

    if args.json {
        run::install_quiet(&device, &apk.path)?;
    } else {
        run::install(&device, &apk.path)?;
    }
    run::clear_logcat(&device);
    let package = if config.android.package.is_empty() {
        config.app.id.clone()
    } else {
        config.android.package.clone()
    };
    let component = run::activity_component(root, &package);
    if args.json {
        run::launch_quiet(&device, &package, &component, &[])?;
    } else {
        run::launch(&device, &package, &component, &[])?;
    }
    std::thread::sleep(Duration::from_secs(2));

    let logcat = match run::app_pid(&device, &package) {
        Some(pid) => run::logcat_pid(&device, pid)
            .unwrap_or_else(|_| run::logcat_dump(&device).unwrap_or_default()),
        None => run::logcat_dump(&device).unwrap_or_default(),
    };
    let panic = logcat
        .lines()
        .find(|line| line.contains("panicked at") || line.contains("FATAL EXCEPTION"));

    let png = run::screencap(&device)?;
    let directory = root.join("target").join("winia-test");
    std::fs::create_dir_all(&directory)
        .with_context(|| format!("creating {}", directory.display()))?;
    let screenshot = directory.join("android-smoke.png");
    std::fs::write(&screenshot, png)
        .with_context(|| format!("writing {}", screenshot.display()))?;

    if args.json {
        let value = serde_json::json!({
            "target": "android",
            "device": device,
            "passed": if panic.is_some() { 0 } else { 1 },
            "failed": if panic.is_some() { 1 } else { 0 },
            "ignored": 0,
            "success": panic.is_none(),
            "screenshot": screenshot,
            "duration_ms": started.elapsed().as_millis(),
        });
        println!("{}", serde_json::to_string_pretty(&value)?);
    } else if let Some(line) = panic {
        eprintln!("logcat: {line}");
        eprintln!("Screenshot: {}", screenshot.display());
        bail!("Android smoke detected a panic");
    } else {
        println!("Android smoke passed; screenshot: {}", screenshot.display());
    }

    if let Some(line) = panic {
        bail!("Android smoke detected a panic: {line}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_test_result_counts() {
        let text = "test result: ok. 5 passed; 1 failed; 2 ignored; 0 measured; 0 filtered out
                    test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
";
        assert_eq!(test_counts(text), (8, 1, 2));
    }
}
