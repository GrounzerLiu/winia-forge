//! 'winia package' — assemble distributable Android artifacts.
//!
//! `apk` builds through the same pipeline as `winia build` and is copied to
//! `dist/` with a versioned name. `--split per-abi` builds one APK per ABI;
//! the default is a universal APK. `aab` runs the Gradle bundle task.
//! Every APK is checked for 16 KB ZIP alignment, the configured native ABIs,
//! and (for debug builds) a valid signature.

use crate::cli::{BuildArgs, PackageArgs, SplitMode};
use crate::commands::build;
use crate::config::WiniaConfig;
use crate::platform::android::Abi;
use anyhow::{Context, Result, bail};
use std::path::{Path, PathBuf};
use std::process::Command;

pub fn run(args: PackageArgs) -> Result<()> {
    let cwd = std::env::current_dir().context("reading the current directory")?;
    let config_path = crate::config::find_config(&cwd)
        .ok_or_else(|| anyhow::anyhow!("winia.toml not found; run this inside a Winia project"))?;
    let root = config_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .to_path_buf();
    let config = WiniaConfig::load(&config_path)?;
    config.validate()?;

    let targets = if args.target.is_empty() {
        vec![build::host_target().to_owned()]
    } else {
        args.target.clone()
    };
    if let Some(unsupported) = targets.iter().find(|target| target.as_str() != "android") {
        bail!(
            "packaging for {unsupported} is not implemented yet; only --target android is supported"
        );
    }
    package_android(&root, &config, &args)
}

fn package_android(root: &Path, config: &WiniaConfig, args: &PackageArgs) -> Result<()> {
    let android_dir = root.join("android");
    let gradlew = android_dir.join(if cfg!(windows) {
        "gradlew.bat"
    } else {
        "gradlew"
    });
    if !gradlew.is_file() {
        bail!(
            "{} does not look like a complete Android project; run 'winia android sync' or recreate it with --platforms android",
            android_dir.display()
        );
    }

    let formats: Vec<String> = if args.format.is_empty() {
        config.package.android.formats.clone()
    } else {
        args.format.clone()
    }
    .iter()
    .map(|format| format.to_lowercase())
    .collect();
    if formats.is_empty() {
        bail!(
            "no Android formats selected; pass --format apk,aab or set [package.android].formats"
        );
    }
    if let Some(unknown) = formats
        .iter()
        .find(|format| format.as_str() != "apk" && format.as_str() != "aab")
    {
        bail!("unsupported Android format '{unknown}'; expected apk or aab");
    }

    let abis: Vec<Abi> = config
        .android
        .abis
        .iter()
        .map(|abi| Abi::parse(abi))
        .collect::<Result<Vec<_>>>()?;
    if abis.is_empty() {
        bail!("no Android ABIs configured; set [android].abis");
    }

    let out_dir = args.out_dir.clone().unwrap_or_else(|| root.join("dist"));
    std::fs::create_dir_all(&out_dir).with_context(|| format!("creating {}", out_dir.display()))?;
    let stem = artifact_stem(config);
    let version = config.app.version.clone();
    let profile = if args.release { "release" } else { "debug" };
    let mut produced = Vec::new();

    if formats.iter().any(|format| format == "apk") {
        let split = args.split.unwrap_or(SplitMode::Universal);
        match split {
            SplitMode::Universal => {
                let built = build::build_all(root, config, &build_args(args, Vec::new()))?;
                let apk = find_apk(&built)?;
                let destination = out_dir.join(format!("{stem}-{version}-android-universal.apk"));
                copy_artifact(&apk, &destination)?;
                verify_apk(&destination, &abis, args.release)?;
                produced.push(destination);
            }
            SplitMode::PerAbi => {
                for abi in &abis {
                    let built = build::build_all(
                        root,
                        config,
                        &build_args(args, vec![abi.as_str().to_owned()]),
                    )?;
                    let apk = find_apk(&built)?;
                    let destination =
                        out_dir.join(format!("{stem}-{version}-android-{}.apk", abi.as_str()));
                    copy_artifact(&apk, &destination)?;
                    verify_apk(&destination, &[*abi], args.release)?;
                    produced.push(destination);
                }
            }
        }
    }

    if formats.iter().any(|format| format == "aab") {
        build::build_all(root, config, &build_args(args, Vec::new()))?;
        let task = if args.release {
            "bundleRelease"
        } else {
            "bundleDebug"
        };
        build::run_gradle_task(&android_dir, task, false)?;
        let bundle = android_dir
            .join("app/build/outputs/bundle")
            .join(profile)
            .join(format!("app-{profile}.aab"));
        if !bundle.is_file() {
            bail!(
                "Gradle finished but no AAB was found at {}",
                bundle.display()
            );
        }
        let destination = out_dir.join(format!("{stem}-{version}-android.aab"));
        copy_artifact(&bundle, &destination)?;
        produced.push(destination);
    }

    for artifact in &produced {
        let bytes = std::fs::metadata(artifact)
            .map(|metadata| metadata.len())
            .unwrap_or(0);
        println!(
            "{} ({:.1} MB)",
            display_relative(root, artifact),
            bytes as f64 / (1024.0 * 1024.0)
        );
    }
    Ok(())
}

fn build_args(args: &PackageArgs, abis: Vec<String>) -> BuildArgs {
    BuildArgs {
        target: vec!["android".to_owned()],
        release: args.release,
        abi: abis,
        features: Vec::new(),
        json: false,
        out_dir: None,
    }
}

fn find_apk(built: &[(String, Vec<build::Artifact>)]) -> Result<PathBuf> {
    built
        .iter()
        .flat_map(|(_, artifacts)| artifacts.iter())
        .find(|artifact| extension(&artifact.path) == Some("apk"))
        .map(|artifact| artifact.path.clone())
        .ok_or_else(|| anyhow::anyhow!("the build produced no APK"))
}

fn copy_artifact(source: &Path, destination: &Path) -> Result<()> {
    if source == destination {
        return Ok(());
    }
    std::fs::copy(source, destination)
        .with_context(|| format!("copying {} to {}", source.display(), destination.display()))?;
    Ok(())
}

fn verify_apk(apk: &Path, abis: &[Abi], release: bool) -> Result<()> {
    let Some(build_tools) = build_tools_dir() else {
        println!("warning: Android build-tools not found; skipped APK verification");
        return Ok(());
    };

    let zipalign = build_tools.join(executable("zipalign"));
    if zipalign.is_file() {
        let output = Command::new(&zipalign)
            .args(["-c", "-P", "16", "4"])
            .arg(apk)
            .output()
            .with_context(|| format!("running {}", zipalign.display()))?;
        if !output.status.success() {
            bail!(
                "{} is not 16 KB aligned: {}",
                apk.display(),
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }
    }

    let aapt2 = build_tools.join(executable("aapt2"));
    if aapt2.is_file() {
        let output = Command::new(&aapt2)
            .args(["dump", "badging"])
            .arg(apk)
            .output()
            .with_context(|| format!("running {}", aapt2.display()))?;
        let text = String::from_utf8_lossy(&output.stdout);
        let native = text
            .lines()
            .find(|line| line.starts_with("native-code:"))
            .unwrap_or("native-code: (none)");
        for abi in abis {
            let needle = format!("'{}'", abi.as_str());
            if !native.contains(&needle) {
                bail!(
                    "{} does not contain native code for {} ({native})",
                    apk.display(),
                    abi.as_str()
                );
            }
        }
    }

    let apksigner = build_tools.join(if cfg!(windows) {
        "apksigner.bat"
    } else {
        "apksigner"
    });
    if apksigner.is_file() {
        let output = if cfg!(windows) {
            Command::new("cmd")
                .arg("/c")
                .arg(&apksigner)
                .args(["verify", "--print-certs"])
                .arg(apk)
                .output()
        } else {
            Command::new(&apksigner)
                .args(["verify", "--print-certs"])
                .arg(apk)
                .output()
        }
        .with_context(|| format!("running {}", apksigner.display()))?;
        if !output.status.success() {
            let detail = String::from_utf8_lossy(&output.stdout);
            if release {
                println!(
                    "warning: {} is unsigned or its signature could not be verified",
                    apk.display()
                );
            } else {
                bail!(
                    "signature verification failed for {}: {}",
                    apk.display(),
                    detail.trim()
                );
            }
        }
    }
    Ok(())
}

fn build_tools_dir() -> Option<PathBuf> {
    for variable in ["ANDROID_HOME", "ANDROID_SDK_ROOT"] {
        if let Some(value) = std::env::var_os(variable).filter(|value| !value.is_empty()) {
            let root = PathBuf::from(value).join("build-tools");
            let mut versions: Vec<PathBuf> = std::fs::read_dir(&root)
                .map(|entries| {
                    entries
                        .flatten()
                        .map(|entry| entry.path())
                        .filter(|path| path.is_dir())
                        .collect()
                })
                .unwrap_or_default();
            versions.sort();
            if let Some(latest) = versions.pop() {
                return Some(latest);
            }
        }
    }
    None
}

fn executable(name: &str) -> String {
    if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_owned()
    }
}

fn artifact_stem(config: &WiniaConfig) -> String {
    let stem: String = config
        .app
        .name
        .to_lowercase()
        .replace(' ', "-")
        .chars()
        .filter(|character| {
            character.is_ascii_alphanumeric() || *character == '-' || *character == '_'
        })
        .collect();
    if stem.is_empty() {
        "app".to_owned()
    } else {
        stem
    }
}

fn extension(path: &Path) -> Option<&str> {
    path.extension().and_then(|extension| extension.to_str())
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
    fn artifact_names_are_lowercase_and_safe() {
        let mut config = WiniaConfig::default();
        config.app.name = "My App!".to_owned();
        assert_eq!(artifact_stem(&config), "my-app");
    }
}
