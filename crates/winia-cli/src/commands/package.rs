//! 'winia package' — assemble distributable Android artifacts.
//!
//! `apk` builds through the same pipeline as `winia build` and is copied to
//! `dist/` with a versioned name. `--split per-abi` builds one APK per ABI;
//! the default is a universal APK. `aab` runs the Gradle bundle task.
//! Every APK is checked for 16 KB ZIP alignment, the configured native ABIs,
//! version metadata, and (for debug builds) a valid signature.

use crate::cli::{BuildArgs, PackageArgs, SplitMode};
use crate::commands::build;
use crate::config::WiniaConfig;
use crate::platform::android::Abi;
use crate::platform::tools::{apksigner_command, build_tools_dir, executable};
use anyhow::{Context, Result, bail};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

pub fn run(args: PackageArgs) -> Result<()> {
    let started = Instant::now();
    let cwd = std::env::current_dir().context("reading the current directory")?;
    let config_path = crate::config::find_config(&cwd)
        .ok_or_else(|| anyhow::anyhow!("winia.toml not found; run this inside a Winia project"))?;
    let root = config_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .to_path_buf();
    let config = WiniaConfig::load(&config_path)?;
    config.validate()?;

    let targets = resolve_targets(&root, &args)?;
    if targets.iter().any(|target| target != "android") {
        bail!("only Android packaging is implemented");
    }
    package_android(&root, &config, &args, started)
}

fn resolve_targets(root: &Path, args: &PackageArgs) -> Result<Vec<String>> {
    if args.target.is_empty() {
        if root.join("android").is_dir() {
            return Ok(vec!["android".to_owned()]);
        }
        bail!("no packageable platform found; pass --target android");
    }
    let mut targets = Vec::new();
    for target in &args.target {
        match target.as_str() {
            "android" => targets.push("android".to_owned()),
            "windows" | "macos" | "linux" => bail!(
                "packaging for {target} is not implemented yet; only --target android is supported"
            ),
            other => bail!("unknown target '{other}'; expected android"),
        }
    }
    Ok(targets)
}

fn package_android(
    root: &Path,
    config: &WiniaConfig,
    args: &PackageArgs,
    started: Instant,
) -> Result<()> {
    let android_dir = root.join("android");

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

    let out_dir = match &args.out_dir {
        Some(path) if path.is_absolute() => path.clone(),
        Some(path) => root.join(path),
        None => root.join("dist"),
    };
    let stem = artifact_stem(config);
    let version = config.app.version.clone();
    let profile = if args.release { "release" } else { "debug" };
    let split = args.split.unwrap_or(SplitMode::Universal);
    if args.split.is_some() && !formats.iter().any(|format| format == "apk") && !args.json {
        eprintln!(
            "warning: --split only affects the apk format; ignoring it for the requested formats"
        );
    }
    let planned = planned_artifacts(&stem, &version, &formats, split, &abis, &out_dir);

    if args.dry_run {
        return report(
            root,
            &planned,
            true,
            args.json,
            started.elapsed(),
            ReportMeta {
                formats: &formats,
                split,
                version: &version,
            },
        );
    }

    std::fs::create_dir_all(&out_dir).with_context(|| format!("creating {}", out_dir.display()))?;
    if args.clean {
        clean_output_dir(&out_dir, &stem, &version)?;
    } else if has_package_artifacts(&out_dir, &stem, &version) && !args.json {
        println!(
            "note: {} already contains files; pass --clean to remove stale artifacts",
            out_dir.display()
        );
    }

    let mut produced = Vec::new();

    if formats.iter().any(|format| format == "apk") {
        match split {
            SplitMode::Universal => {
                let built = build::build_all(root, config, &build_args(args, Vec::new()))?;
                let apk = find_apk(&built)?;
                let destination = out_dir.join(format!("{stem}-{version}-android-universal.apk"));
                copy_artifact(&apk, &destination)?;
                verify_apk(
                    &destination,
                    &abis,
                    args.release,
                    config.android.version_code,
                    &version,
                    args.json,
                )?;
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
                    verify_apk(
                        &destination,
                        &[*abi],
                        args.release,
                        config.android.version_code,
                        &version,
                        args.json,
                    )?;
                    produced.push(destination);
                }
            }
        }
    }

    if formats.iter().any(|format| format == "aab") {
        build::build_android_libraries(root, config, &build_args(args, Vec::new()))?;
        let task = if args.release {
            "bundleRelease"
        } else {
            "bundleDebug"
        };
        build::run_gradle_task(
            &android_dir,
            task,
            &build::version_properties(config),
            args.json,
        )?;
        let bundle_dir = android_dir.join("app/build/outputs/bundle").join(profile);
        let mut bundle = bundle_dir.join(format!("app-{profile}.aab"));
        if !bundle.is_file() {
            let mut candidates: Vec<PathBuf> = std::fs::read_dir(&bundle_dir)
                .map(|entries| {
                    entries
                        .flatten()
                        .map(|entry| entry.path())
                        .filter(|path| extension(path) == Some("aab"))
                        .collect()
                })
                .unwrap_or_default();
            candidates.sort_by_key(|path| {
                std::fs::metadata(path)
                    .and_then(|metadata| metadata.modified())
                    .ok()
            });
            bundle = candidates.pop().ok_or_else(|| {
                anyhow::anyhow!(
                    "Gradle finished but no AAB was found in {}",
                    bundle_dir.display()
                )
            })?;
        }
        let destination = out_dir.join(format!("{stem}-{version}-android.aab"));
        copy_artifact(&bundle, &destination)?;
        validate_aab(&destination, args.json)?;
        produced.push(destination);
    }

    report(
        root,
        &produced,
        false,
        args.json,
        started.elapsed(),
        ReportMeta {
            formats: &formats,
            split,
            version: &version,
        },
    )
}

fn planned_artifacts(
    stem: &str,
    version: &str,
    formats: &[String],
    split: SplitMode,
    abis: &[Abi],
    out_dir: &Path,
) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if formats.iter().any(|format| format == "apk") {
        match split {
            SplitMode::Universal => {
                paths.push(out_dir.join(format!("{stem}-{version}-android-universal.apk")))
            }
            SplitMode::PerAbi => {
                for abi in abis {
                    paths.push(
                        out_dir.join(format!("{stem}-{version}-android-{}.apk", abi.as_str())),
                    );
                }
            }
        }
    }
    if formats.iter().any(|format| format == "aab") {
        paths.push(out_dir.join(format!("{stem}-{version}-android.aab")));
    }
    paths
}

fn build_args(args: &PackageArgs, abis: Vec<String>) -> BuildArgs {
    BuildArgs {
        target: vec!["android".to_owned()],
        release: args.release,
        abi: abis,
        features: Vec::new(),
        json: args.json,
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

fn clean_output_dir(out_dir: &Path, stem: &str, version: &str) -> Result<()> {
    if !out_dir.is_dir() {
        return Ok(());
    }
    for entry in
        std::fs::read_dir(out_dir).with_context(|| format!("reading {}", out_dir.display()))?
    {
        let path = entry?.path();
        if path.is_file() && is_package_artifact(&path, stem, version) {
            std::fs::remove_file(&path).with_context(|| format!("removing {}", path.display()))?;
        }
    }
    Ok(())
}

fn is_package_artifact(path: &Path, stem: &str, version: &str) -> bool {
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    let base = format!("{stem}-{version}-android");
    (name.starts_with(&format!("{base}-")) || name.starts_with(&format!("{base}.")))
        && (name.ends_with(".apk") || name.ends_with(".aab"))
}

fn has_package_artifacts(directory: &Path, stem: &str, version: &str) -> bool {
    std::fs::read_dir(directory)
        .map(|entries| {
            entries
                .flatten()
                .any(|entry| is_package_artifact(&entry.path(), stem, version))
        })
        .unwrap_or(false)
}

fn verify_apk(
    apk: &Path,
    abis: &[Abi],
    release: bool,
    version_code: u32,
    version_name: &str,
    json: bool,
) -> Result<()> {
    let Some(build_tools) = build_tools_dir() else {
        if !json {
            eprintln!("warning: Android build-tools not found; skipped APK verification");
        }
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
    } else if !json {
        eprintln!("warning: zipalign not found; skipped alignment check");
    }

    let aapt2 = build_tools.join(executable("aapt2"));
    if aapt2.is_file() {
        let output = Command::new(&aapt2)
            .args(["dump", "badging"])
            .arg(apk)
            .output()
            .with_context(|| format!("running {}", aapt2.display()))?;
        if !output.status.success() {
            bail!(
                "aapt2 failed on {}: {}",
                apk.display(),
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }
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
        match badging_version(&text) {
            Some((actual_code, actual_name)) => {
                if actual_code != Some(version_code) {
                    bail!(
                        "{} reports versionCode {:?}, expected {}",
                        apk.display(),
                        actual_code,
                        version_code
                    );
                }
                if actual_name.as_deref() != Some(version_name) {
                    bail!(
                        "{} reports versionName {:?}, expected {}",
                        apk.display(),
                        actual_name,
                        version_name
                    );
                }
            }
            None => {
                if !json {
                    eprintln!(
                        "warning: could not read version metadata from {}",
                        apk.display()
                    );
                }
            }
        }
    } else if !json {
        eprintln!("warning: aapt2 not found; skipped ABI and version checks");
    }

    let launcher = build_tools.join(if cfg!(windows) {
        "apksigner.bat"
    } else {
        "apksigner"
    });
    if launcher.is_file() || build_tools.join("lib").join("apksigner.jar").is_file() {
        let mut command = apksigner_command(&build_tools)?;
        command.args(["verify", "--print-certs"]).arg(apk);
        let output = command.output().context("running apksigner")?;
        if !output.status.success() {
            if release {
                if !json {
                    eprintln!(
                        "warning: {} is unsigned; run 'winia sign --target android' before distributing",
                        apk.display()
                    );
                }
            } else {
                bail!(
                    "signature verification failed for {}: {}",
                    apk.display(),
                    String::from_utf8_lossy(&output.stdout).trim()
                );
            }
        }
    } else if !json {
        eprintln!("warning: apksigner not found; skipped signature check");
    }
    Ok(())
}

fn validate_aab(aab: &Path, json: bool) -> Result<()> {
    let Some(mut command) = bundletool_command() else {
        if !json {
            eprintln!("warning: bundletool not found; skipped AAB validation");
        }
        return Ok(());
    };
    let output = command
        .args(["validate", "--bundle"])
        .arg(aab)
        .output()
        .context("running bundletool")?;
    if !output.status.success() {
        bail!(
            "bundletool rejected {}: {}",
            aab.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(())
}

fn bundletool_command() -> Option<Command> {
    if let Some(value) = std::env::var_os("BUNDLETOOL_JAR").filter(|value| !value.is_empty()) {
        let jar = PathBuf::from(value);
        if jar.is_file() {
            let mut command = Command::new("java");
            command.arg("-jar").arg(jar);
            return Some(command);
        }
    }
    if Command::new("bundletool")
        .arg("version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
    {
        return Some(Command::new("bundletool"));
    }
    None
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

fn attribute(line: &str, key: &str) -> Option<String> {
    let marker = format!("{key}='");
    let start = line.find(&marker)? + marker.len();
    let rest = &line[start..];
    let end = rest.find('\'')?;
    Some(rest[..end].to_owned())
}

fn badging_version(text: &str) -> Option<(Option<u32>, Option<String>)> {
    let line = text.lines().find(|line| line.starts_with("package: "))?;
    let code = attribute(line, "versionCode").and_then(|value| value.parse().ok());
    let name = attribute(line, "versionName");
    Some((code, name))
}

struct ReportMeta<'a> {
    formats: &'a [String],
    split: SplitMode,
    version: &'a str,
}

fn report(
    root: &Path,
    artifacts: &[PathBuf],
    dry_run: bool,
    json: bool,
    elapsed: Duration,
    meta: ReportMeta<'_>,
) -> Result<()> {
    if json {
        let values: Vec<_> = artifacts
            .iter()
            .map(|path| {
                let bytes = if dry_run {
                    0
                } else {
                    std::fs::metadata(path)
                        .map(|metadata| metadata.len())
                        .unwrap_or(0)
                };
                serde_json::json!({
                    "path": display_relative(root, path),
                    "bytes": bytes,
                })
            })
            .collect();
        let split = match meta.split {
            SplitMode::Universal => "universal",
            SplitMode::PerAbi => "per-abi",
        };
        let value = serde_json::json!({
            "target": "android",
            "version": meta.version,
            "formats": meta.formats,
            "split": split,
            "dry_run": dry_run,
            "artifacts": values,
            "duration_ms": elapsed.as_millis(),
        });
        println!("{}", serde_json::to_string_pretty(&value)?);
        return Ok(());
    }
    println!(
        "{}",
        if dry_run {
            "Would create:"
        } else {
            "Packaged:"
        }
    );
    for artifact in artifacts {
        if dry_run {
            println!("  {}", display_relative(root, artifact));
        } else {
            let bytes = std::fs::metadata(artifact)
                .map(|metadata| metadata.len())
                .unwrap_or(0);
            println!(
                "  {} ({:.1} MB)",
                display_relative(root, artifact),
                bytes as f64 / (1024.0 * 1024.0)
            );
        }
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
    fn artifact_names_are_lowercase_and_safe() {
        let mut config = WiniaConfig::default();
        config.app.name = "My App!".to_owned();
        assert_eq!(artifact_stem(&config), "my-app");
    }

    #[test]
    fn planned_names_follow_the_format_and_split() {
        let abis = [Abi::Arm64V8a, Abi::X86_64];
        let out = Path::new("dist");
        let universal = planned_artifacts(
            "demo",
            "0.1.0",
            &["apk".to_owned()],
            SplitMode::Universal,
            &abis,
            out,
        );
        assert_eq!(
            universal,
            vec![PathBuf::from("dist/demo-0.1.0-android-universal.apk")]
        );

        let split = planned_artifacts(
            "demo",
            "0.1.0",
            &["apk".to_owned(), "aab".to_owned()],
            SplitMode::PerAbi,
            &abis,
            out,
        );
        assert_eq!(
            split,
            vec![
                PathBuf::from("dist/demo-0.1.0-android-arm64-v8a.apk"),
                PathBuf::from("dist/demo-0.1.0-android-x86_64.apk"),
                PathBuf::from("dist/demo-0.1.0-android.aab"),
            ]
        );
    }

    #[test]
    fn reads_version_metadata_from_aapt_badging() {
        let line = "package: name='com.example.demo' versionCode='7' versionName='0.2.0' platformBuildVersionName='16'";
        assert_eq!(
            badging_version(line),
            Some((Some(7), Some("0.2.0".to_owned())))
        );
    }

    #[test]
    fn only_matching_artifacts_are_cleaned() {
        let stem = "demo";
        let version = "0.1.0";
        assert!(is_package_artifact(
            Path::new("demo-0.1.0-android-universal.apk"),
            stem,
            version
        ));
        assert!(is_package_artifact(
            Path::new("demo-0.1.0-android-arm64-v8a.apk"),
            stem,
            version
        ));
        assert!(is_package_artifact(
            Path::new("demo-0.1.0-android.aab"),
            stem,
            version
        ));
        assert!(!is_package_artifact(Path::new("notes.txt"), stem, version));
        assert!(!is_package_artifact(
            Path::new("demo-0.2.0-android-universal.apk"),
            stem,
            version
        ));
    }
}
