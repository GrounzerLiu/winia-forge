//! 'winia sign' — sign Android APKs with `apksigner`.
//!
//! Credentials come from the environment so passwords never appear in the
//! process list: `WINIA_ANDROID_KEYSTORE`, `WINIA_ANDROID_KEY_ALIAS`,
//! `WINIA_ANDROID_KEYSTORE_PASSWORD`, and optionally
//! `WINIA_ANDROID_KEY_PASSWORD`. Relative `--apk` and `--keystore` paths are
//! resolved against the current directory. By default every APK in `dist/` is
//! signed into a `*-signed.apk` copy; `--in-place` overwrites instead.

use crate::cli::SignArgs;
use crate::config::WiniaConfig;
use crate::platform::tools::{apksigner_command, build_tools_dir, executable};
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
    let windows = targets.iter().any(|target| target == "windows");
    let android = targets.iter().any(|target| target == "android");
    if let Some(unsupported) = targets
        .iter()
        .find(|target| target.as_str() != "android" && target.as_str() != "windows")
    {
        bail!(
            "signing for {unsupported} is not implemented yet; only android and windows are supported"
        );
    }
    if windows && !android {
        return super::sign_windows::run(&root, &cwd, &args);
    }
    if !root.join("android").is_dir() {
        bail!("no Android project found; create it with --platforms android");
    }

    let build_tools = build_tools_dir();
    let inputs = resolve_inputs(&cwd, &root, &args, build_tools.as_deref())?;
    let plans: Vec<(PathBuf, PathBuf)> = inputs
        .into_iter()
        .map(|input| {
            let output = if args.in_place {
                input.clone()
            } else {
                signed_path(&input)
            };
            (input, output)
        })
        .collect();

    if args.dry_run {
        return report_plan(&root, &plans, args.json, started.elapsed());
    }

    let build_tools = build_tools
        .ok_or_else(|| anyhow::anyhow!("Android build-tools not found; set ANDROID_HOME"))?;
    let keystore = resolve_keystore(&cwd, &args)?;
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

    let mut results = Vec::new();
    for (input, output) in plans {
        sign(
            &build_tools,
            &keystore,
            &alias,
            &input,
            &output,
            key_password_set,
        )?;
        let certificate = verify(&build_tools, &output)?;
        results.push((input, output, certificate));
    }

    report(&root, &results, args.json, started.elapsed())
}

fn resolve_keystore(cwd: &Path, args: &SignArgs) -> Result<PathBuf> {
    let path = args
        .keystore
        .clone()
        .or_else(|| env_value("WINIA_ANDROID_KEYSTORE").map(PathBuf::from))
        .ok_or_else(|| {
            anyhow::anyhow!("keystore required; pass --keystore or set WINIA_ANDROID_KEYSTORE")
        })?;
    let path = absolute(cwd, &path);
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

fn resolve_inputs(
    cwd: &Path,
    root: &Path,
    args: &SignArgs,
    build_tools: Option<&Path>,
) -> Result<Vec<PathBuf>> {
    if let Some(apk) = &args.apk {
        let path = absolute(cwd, apk);
        if !path.is_file() {
            bail!("APK not found: {}", path.display());
        }
        return Ok(vec![path]);
    }

    let dist = root.join("dist");
    if !dist.is_dir() {
        bail!(
            "{} does not exist; run 'winia package --target android' first or pass --apk",
            dist.display()
        );
    }
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

    if args.release {
        let mut filtered = Vec::new();
        for apk in inputs {
            match build_tools.and_then(|tools| apk_is_debuggable(tools, &apk)) {
                Some(true) => {
                    if !args.json {
                        eprintln!("note: skipping debuggable APK {}", apk.display());
                    }
                }
                None => {
                    if !args.json {
                        eprintln!(
                            "warning: could not determine whether {} is debuggable; keeping it",
                            apk.display()
                        );
                    }
                    filtered.push(apk);
                }
                Some(false) => filtered.push(apk),
            }
        }
        inputs = filtered;
    }

    if inputs.is_empty() {
        bail!(
            "no APKs found in {}; run 'winia package --target android' or pass --apk",
            dist.display()
        );
    }
    Ok(inputs)
}

fn apk_is_debuggable(build_tools: &Path, apk: &Path) -> Option<bool> {
    let aapt2 = build_tools.join(executable("aapt2"));
    if !aapt2.is_file() {
        return None;
    }
    let output = Command::new(&aapt2)
        .args(["dump", "badging"])
        .arg(apk)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    Some(
        String::from_utf8_lossy(&output.stdout)
            .lines()
            .any(|line| line.starts_with("application-debuggable")),
    )
}

fn absolute(base: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_owned()
    } else {
        std::path::absolute(base.join(path)).unwrap_or_else(|_| base.join(path))
    }
}

fn signed_path(input: &Path) -> PathBuf {
    let stem = input
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_else(|| "app".to_owned());
    input.with_file_name(format!("{stem}-signed.apk"))
}

fn sign(
    build_tools: &Path,
    keystore: &Path,
    alias: &str,
    input: &Path,
    output: &Path,
    key_password_set: bool,
) -> Result<()> {
    let mut command = apksigner_command(build_tools)?;
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

fn verify(build_tools: &Path, apk: &Path) -> Result<String> {
    let mut command = apksigner_command(build_tools)?;
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
        .map(certificate_dn)
        .unwrap_or_else(|| "certificate verified".to_owned()))
}

fn certificate_dn(line: &str) -> String {
    line.split("certificate DN:")
        .nth(1)
        .map(str::trim)
        .unwrap_or_else(|| line.trim())
        .to_owned()
}

fn report_plan(
    root: &Path,
    plans: &[(PathBuf, PathBuf)],
    json: bool,
    elapsed: std::time::Duration,
) -> Result<()> {
    if json {
        let artifacts: Vec<_> = plans
            .iter()
            .map(|(input, output)| {
                serde_json::json!({
                    "input": display_relative(root, input),
                    "output": display_relative(root, output),
                    "certificate": "",
                })
            })
            .collect();
        let value = serde_json::json!({
            "target": "android",
            "dry_run": true,
            "artifacts": artifacts,
            "duration_ms": elapsed.as_millis(),
        });
        println!("{}", serde_json::to_string_pretty(&value)?);
        return Ok(());
    }
    println!("Would sign:");
    for (input, output) in plans {
        println!(
            "  {} -> {}",
            display_relative(root, input),
            display_relative(root, output)
        );
    }
    Ok(())
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
            "dry_run": false,
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

    #[test]
    fn certificate_dn_is_extracted_from_the_signer_line() {
        assert_eq!(
            certificate_dn("V3.0 Signer: certificate DN: CN=Winia Test, O=Winia"),
            "CN=Winia Test, O=Winia"
        );
        assert_eq!(
            certificate_dn("certificate verified"),
            "certificate verified"
        );
    }
}
