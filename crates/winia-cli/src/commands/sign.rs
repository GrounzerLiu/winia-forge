//! 'winia sign' — sign Android APKs with `apksigner`.
//!
//! Credentials come from the environment so passwords never appear in the
//! process list: `WINIA_ANDROID_KEYSTORE`, `WINIA_ANDROID_KEY_ALIAS`,
//! `WINIA_ANDROID_KEYSTORE_PASSWORD`, and optionally
//! `WINIA_ANDROID_KEY_PASSWORD`. By default every APK in `dist/` is signed
//! into a `*-signed.apk` copy; `--in-place` overwrites instead.

use crate::cli::SignArgs;
use crate::config::WiniaConfig;
use crate::platform::tools::build_tools_dir;
use anyhow::{Context, Result, bail};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

pub fn run(args: SignArgs) -> Result<()> {
    let started = Instant::now();
    let cwd = std::env::current_dir().context("reading the current directory")?;
    let config_path = crate::config::find_config(&cwd)
        .ok_or_else(|| anyhow::anyhow!("winia.toml not found; run this inside a Winia project"))?;
    let root = config_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .to_path_buf();
    let _config = WiniaConfig::load(&config_path)?;

    let targets = if args.target.is_empty() {
        vec!["android".to_owned()]
    } else {
        args.target.clone()
    };
    if let Some(unsupported) = targets.iter().find(|target| target.as_str() != "android") {
        bail!(
            "signing for {unsupported} is not implemented yet; only --target android is supported"
        );
    }

    let build_tools = build_tools_dir()
        .ok_or_else(|| anyhow::anyhow!("Android build-tools not found; set ANDROID_HOME"))?;
    let apksigner = build_tools.join(if cfg!(windows) {
        "apksigner.bat"
    } else {
        "apksigner"
    });
    if !apksigner.is_file() {
        bail!("apksigner not found at {}", apksigner.display());
    }

    let keystore = resolve_keystore(&args)?;
    let alias = args
        .alias
        .clone()
        .or_else(|| env_value("WINIA_ANDROID_KEY_ALIAS"))
        .ok_or_else(|| {
            anyhow::anyhow!("key alias required; pass --alias or set WINIA_ANDROID_KEY_ALIAS")
        })?;
    if env_value("WINIA_ANDROID_KEYSTORE_PASSWORD").is_none() {
        bail!("WINIA_ANDROID_KEYSTORE_PASSWORD is not set");
    }
    let key_password_set = env_value("WINIA_ANDROID_KEY_PASSWORD").is_some();

    let inputs = resolve_inputs(&root, &args)?;
    let mut results = Vec::new();
    for input in inputs {
        let output = if args.in_place {
            input.clone()
        } else {
            signed_path(&input)
        };
        sign(
            &apksigner,
            &keystore,
            &alias,
            &input,
            &output,
            key_password_set,
        )?;
        let certificate = verify(&apksigner, &output)?;
        results.push((input, output, certificate));
    }

    report(&root, &results, args.json, started.elapsed())
}

fn resolve_keystore(args: &SignArgs) -> Result<PathBuf> {
    let path = args
        .keystore
        .clone()
        .or_else(|| env_value("WINIA_ANDROID_KEYSTORE").map(PathBuf::from))
        .ok_or_else(|| {
            anyhow::anyhow!("keystore required; pass --keystore or set WINIA_ANDROID_KEYSTORE")
        })?;
    if !path.is_file() {
        bail!("keystore not found: {}", path.display());
    }
    Ok(path)
}

fn env_value(name: &str) -> Option<String> {
    std::env::var_os(name)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_string_lossy().into_owned())
}

fn resolve_inputs(root: &Path, args: &SignArgs) -> Result<Vec<PathBuf>> {
    if let Some(apk) = &args.apk {
        let path = if apk.is_absolute() {
            apk.clone()
        } else {
            root.join(apk)
        };
        if !path.is_file() {
            bail!("APK not found: {}", path.display());
        }
        return Ok(vec![path]);
    }
    let dist = root.join("dist");
    let mut inputs: Vec<PathBuf> = std::fs::read_dir(&dist)
        .with_context(|| format!("reading {}", dist.display()))?
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension().and_then(|extension| extension.to_str()) == Some("apk")
                && !path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .map(|name| name.ends_with("-signed.apk"))
                    .unwrap_or(false)
        })
        .collect();
    inputs.sort();
    if inputs.is_empty() {
        bail!(
            "no APKs found in {}; run 'winia package --target android' or pass --apk",
            dist.display()
        );
    }
    Ok(inputs)
}

fn signed_path(input: &Path) -> PathBuf {
    let stem = input
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_else(|| "app".to_owned());
    input.with_file_name(format!("{stem}-signed.apk"))
}

fn apksigner_command(apksigner: &Path) -> Command {
    if cfg!(windows) {
        let mut command = Command::new("cmd");
        command.arg("/c").arg(apksigner);
        command
    } else {
        Command::new(apksigner)
    }
}

fn sign(
    apksigner: &Path,
    keystore: &Path,
    alias: &str,
    input: &Path,
    output: &Path,
    key_password_set: bool,
) -> Result<()> {
    let mut command = apksigner_command(apksigner);
    command
        .arg("sign")
        .arg("--ks")
        .arg(keystore)
        .arg("--ks-key-alias")
        .arg(alias)
        .arg("--ks-pass")
        .arg("env:WINIA_ANDROID_KEYSTORE_PASSWORD");
    if key_password_set {
        command
            .arg("--key-pass")
            .arg("env:WINIA_ANDROID_KEY_PASSWORD");
    }
    if input != output {
        command.arg("--out").arg(output);
    }
    command.arg(input);
    let result = command
        .output()
        .context("running apksigner sign (is the JDK on PATH?)")?;
    if !result.status.success() {
        bail!(
            "apksigner sign failed for {}: {}",
            input.display(),
            String::from_utf8_lossy(&result.stderr).trim()
        );
    }
    if !output.is_file() {
        bail!(
            "apksigner finished but {} was not created",
            output.display()
        );
    }
    Ok(())
}

fn verify(apksigner: &Path, apk: &Path) -> Result<String> {
    let mut command = apksigner_command(apksigner);
    command.args(["verify", "--print-certs"]).arg(apk);
    let result = command.output().context("running apksigner verify")?;
    if !result.status.success() {
        bail!(
            "apksigner verify failed for {}: {}",
            apk.display(),
            String::from_utf8_lossy(&result.stdout).trim()
        );
    }
    let text = String::from_utf8_lossy(&result.stdout);
    Ok(text
        .lines()
        .find(|line| line.contains("certificate DN"))
        .map(str::trim)
        .unwrap_or("certificate verified")
        .to_owned())
}

fn report(
    root: &Path,
    results: &[(PathBuf, PathBuf, String)],
    json: bool,
    elapsed: std::time::Duration,
) -> Result<()> {
    if json {
        let artifacts: Vec<_> = results
            .iter()
            .map(|(input, output, certificate)| {
                serde_json::json!({
                    "input": display_relative(root, input),
                    "output": display_relative(root, output),
                    "certificate": certificate,
                })
            })
            .collect();
        let value = serde_json::json!({
            "target": "android",
            "artifacts": artifacts,
            "duration_ms": elapsed.as_millis(),
        });
        println!("{}", serde_json::to_string_pretty(&value)?);
        return Ok(());
    }
    for (_, output, certificate) in results {
        println!("Signed {} ({certificate})", display_relative(root, output));
    }
    Ok(())
}

fn display_relative(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .map(|relative| relative.display().to_string())
        .unwrap_or_else(|_| path.display().to_string())
        .replace('\\', "/")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signed_copies_get_a_suffix() {
        assert_eq!(
            signed_path(Path::new("dist/app-0.1.0-android-universal.apk")),
            PathBuf::from("dist/app-0.1.0-android-universal-signed.apk")
        );
    }
}
