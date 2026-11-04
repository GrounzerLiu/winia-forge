//! 'winia build' — build the application for one or more targets.
//!
//! Desktop targets delegate to `cargo build`. Android uses a built-in NDK driver:
//! it derives the linker / `CC` / `AR` environment for each ABI, runs
//! `cargo build --target <triple> --lib`, copies the library into
//! `android/app/src/main/jniLibs/<abi>/`, checks that every LOAD segment is
//! aligned to 16 KB, and assembles the APK with the project's Gradle wrapper.
//! Neither cargo-ndk nor cargo-apk2 is required.

use crate::cli::BuildArgs;
use crate::config::WiniaConfig;
use crate::platform::android::Abi;
use anyhow::{Context, Result, bail};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

#[derive(Debug)]
pub(crate) struct Artifact {
    pub(crate) path: PathBuf,
    pub(crate) bytes: u64,
}

#[derive(Debug)]
struct ProjectMetadata {
    target_directory: PathBuf,
    binary_name: String,
    library_name: String,
}

pub fn run(args: BuildArgs) -> Result<()> {
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
    let built = build_all(&root, &config, &args)?;

    let mut all = Vec::new();
    for (target, artifacts) in &built {
        for artifact in artifacts {
            all.push(artifact);
        }
        if target != "android" {
            if let Some(executable) = artifacts.iter().find(|artifact| {
                artifact.path.extension().and_then(|ext| ext.to_str()) != Some("apk")
            }) {
                if let Some(parent) = executable.path.parent() {
                    crate::assets::stage_desktop(&root, &config, parent)?;
                }
            }
        }
    }
    let profile = if args.release { "release" } else { "debug" };
    if let Some(out_dir) = &args.out_dir {
        copy_artifacts(&all, out_dir)?;
        crate::assets::stage_desktop(&root, &config, out_dir)?;
    }
    report(&root, &built, profile, started.elapsed(), args.json)
}

/// Build the requested targets without printing a report. Shared by
/// `winia build` and `winia run`.
pub(crate) fn build_all(
    root: &Path,
    config: &WiniaConfig,
    args: &BuildArgs,
) -> Result<Vec<(String, Vec<Artifact>)>> {
    crate::icons::ensure(root, config)?;
    let metadata = cargo_metadata(root)?;
    let features = effective_features(config, args);
    let targets = resolve_targets(&args.target)?;
    let mut built = Vec::new();
    for target in &targets {
        let artifacts = match target.as_str() {
            "android" => build_android(root, config, args, &metadata, &features)?,
            host => build_desktop(root, args, &metadata, host, &features)?,
        };
        built.push((target.clone(), artifacts));
    }
    Ok(built)
}

fn resolve_targets(requested: &[String]) -> Result<Vec<String>> {
    if requested.is_empty() {
        return Ok(vec![host_target().to_owned()]);
    }
    let mut targets = Vec::new();
    for raw in requested {
        let name = raw.trim();
        if name.is_empty() {
            continue;
        }
        match name {
            "all" => {
                let host = host_target().to_owned();
                if !targets.contains(&host) {
                    targets.push(host);
                }
                if !targets.iter().any(|target| target == "android") {
                    targets.push("android".to_owned());
                }
            }
            "windows" | "macos" | "linux" | "android" => {
                if !targets.iter().any(|target| target == name) {
                    targets.push(name.to_owned());
                }
            }
            other => {
                bail!("unknown target '{other}'; expected windows, macos, linux, android, or all")
            }
        }
    }
    Ok(targets)
}

pub(crate) fn host_target() -> &'static str {
    if cfg!(windows) {
        "windows"
    } else if cfg!(target_os = "macos") {
        "macos"
    } else {
        "linux"
    }
}

fn build_desktop(
    root: &Path,
    args: &BuildArgs,
    metadata: &ProjectMetadata,
    target: &str,
    features: &[String],
) -> Result<Vec<Artifact>> {
    if target != host_target() {
        bail!(
            "building the {target} target on {} is not supported yet",
            host_target()
        );
    }

    let mut command = Command::new("cargo");
    command.arg("build").current_dir(root);
    if args.release {
        command.arg("--release");
    }
    if !features.is_empty() {
        command.arg("--features").arg(features.join(","));
    }
    run_command(&mut command, "cargo build", args.json)?;

    let profile = if args.release { "release" } else { "debug" };
    let executable = metadata.target_directory.join(profile).join(format!(
        "{}{}",
        metadata.binary_name,
        std::env::consts::EXE_SUFFIX
    ));
    Ok(vec![artifact(executable)?])
}

fn build_android(
    root: &Path,
    config: &WiniaConfig,
    args: &BuildArgs,
    metadata: &ProjectMetadata,
    features: &[String],
) -> Result<Vec<Artifact>> {
    build_android_libraries_with(root, config, args, metadata, features)?;
    let android_dir = root.join("android");
    let apk = run_gradle(&android_dir, config, args.release, args.json)?;
    Ok(vec![artifact(apk)?])
}

/// Build and copy the native libraries for every selected ABI without invoking
/// Gradle. Packaging uses this for AABs, which do not need an intermediate APK.
pub(crate) fn build_android_libraries(
    root: &Path,
    config: &WiniaConfig,
    args: &BuildArgs,
) -> Result<()> {
    let metadata = cargo_metadata(root)?;
    let features = effective_features(config, args);
    build_android_libraries_with(root, config, args, &metadata, &features)
}

fn build_android_libraries_with(
    root: &Path,
    config: &WiniaConfig,
    args: &BuildArgs,
    metadata: &ProjectMetadata,
    features: &[String],
) -> Result<()> {
    let android_dir = root.join("android");
    if !android_dir.is_dir() {
        bail!(
            "android/ not found; create the project with --platforms android or run 'winia platform add android'"
        );
    }

    ensure_android_project(&android_dir)?;
    crate::icons::ensure(root, config)?;
    let toolchain = AndroidToolchain::discover(config)?;
    let abis = selected_abis(config, args)?;
    ensure_rust_targets(&abis)?;
    toolchain.validate(&abis)?;
    clean_jni_libs(root, &abis)?;

    for abi in &abis {
        build_android_library(root, args, metadata, &toolchain, *abi, features)?;
    }
    crate::assets::stage_android(root, config)?;
    Ok(())
}

fn selected_abis(config: &WiniaConfig, args: &BuildArgs) -> Result<Vec<Abi>> {
    let raw: Vec<String> = if args.abi.is_empty() {
        config.android.abis.clone()
    } else {
        args.abi.clone()
    };
    if raw.is_empty() {
        bail!("no Android ABIs selected; set [android].abis or pass --abi");
    }
    let mut abis = Vec::new();
    for name in raw {
        let abi = Abi::parse(&name)?;
        if !abis.contains(&abi) {
            abis.push(abi);
        }
    }
    Ok(abis)
}

fn build_android_library(
    root: &Path,
    args: &BuildArgs,
    metadata: &ProjectMetadata,
    toolchain: &AndroidToolchain,
    abi: Abi,
    features: &[String],
) -> Result<()> {
    let mut command = Command::new("cargo");
    command
        .args(["build", "--target", abi.rust_triple(), "--lib"])
        .current_dir(root);
    if args.release {
        command.arg("--release");
    }
    if !features.is_empty() {
        command.arg("--features").arg(features.join(","));
    }
    command.envs(toolchain.environment(abi));
    run_command(
        &mut command,
        &format!("cargo build --target {}", abi.rust_triple()),
        args.json,
    )?;

    let profile = if args.release { "release" } else { "debug" };
    let library = metadata
        .target_directory
        .join(abi.rust_triple())
        .join(profile)
        .join(format!("lib{}.so", metadata.library_name));
    if !library.is_file() {
        bail!(
            "expected native library not found: {} (check [lib] crate-type = [\"cdylib\"])",
            library.display()
        );
    }

    let destination_dir = root
        .join("android/app/src/main/jniLibs")
        .join(abi.jni_libs_dir());
    std::fs::create_dir_all(&destination_dir)
        .with_context(|| format!("creating {}", destination_dir.display()))?;
    let destination = destination_dir.join(format!("lib{}.so", metadata.library_name));
    copy_if_changed(&library, &destination)?;
    verify_alignment(toolchain, &destination)?;
    Ok(())
}

fn effective_features(config: &WiniaConfig, args: &BuildArgs) -> Vec<String> {
    let mut features = config.build.features.clone();
    for feature in &args.features {
        if !features.iter().any(|existing| existing == feature) {
            features.push(feature.clone());
        }
    }
    features
}

fn ensure_android_project(android_dir: &Path) -> Result<()> {
    let gradlew = android_dir.join(if cfg!(windows) {
        "gradlew.bat"
    } else {
        "gradlew"
    });
    if !gradlew.is_file() {
        bail!(
            "{} does not look like a complete Android project (missing {}); run 'winia android sync' or recreate it with --platforms android",
            android_dir.display(),
            gradlew
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("gradlew")
        );
    }
    let manifest = android_dir.join("app/src/main/AndroidManifest.xml");
    if !manifest.is_file() {
        bail!(
            "{} is missing app/src/main/AndroidManifest.xml; run 'winia android sync'",
            android_dir.display()
        );
    }
    Ok(())
}

/// Remove native libraries for ABIs that are no longer selected, so a narrowed
/// build cannot package a stale library left by an earlier build.
fn clean_jni_libs(root: &Path, selected: &[Abi]) -> Result<()> {
    let jni = root.join("android/app/src/main/jniLibs");
    if !jni.is_dir() {
        return Ok(());
    }
    for abi in Abi::ALL {
        if selected.contains(&abi) {
            continue;
        }
        let path = jni.join(abi.jni_libs_dir());
        if path.is_dir() {
            std::fs::remove_dir_all(&path)
                .with_context(|| format!("removing stale {}", path.display()))?;
        }
    }
    Ok(())
}

/// Copy only when the contents differ, so an unchanged library does not make
/// Gradle repackage the APK on every build.
fn copy_if_changed(source: &Path, destination: &Path) -> Result<()> {
    if destination.is_file() && same_contents(source, destination)? {
        return Ok(());
    }
    std::fs::copy(source, destination)
        .with_context(|| format!("copying {} to {}", source.display(), destination.display()))?;
    Ok(())
}

fn same_contents(left: &Path, right: &Path) -> Result<bool> {
    if std::fs::metadata(left)?.len() != std::fs::metadata(right)?.len() {
        return Ok(false);
    }
    let mut left_file = std::io::BufReader::new(std::fs::File::open(left)?);
    let mut right_file = std::io::BufReader::new(std::fs::File::open(right)?);
    let mut left_buffer = [0u8; 64 * 1024];
    let mut right_buffer = [0u8; 64 * 1024];
    loop {
        let left_read = left_file.read(&mut left_buffer)?;
        let right_read = right_file.read(&mut right_buffer)?;
        if left_read != right_read || left_buffer[..left_read] != right_buffer[..right_read] {
            return Ok(false);
        }
        if left_read == 0 {
            return Ok(true);
        }
    }
}

pub(crate) fn run_gradle_task(
    android_dir: &Path,
    task: &str,
    properties: &[(&str, String)],
    json: bool,
) -> Result<()> {
    let mut command = if cfg!(windows) {
        let mut command = Command::new("cmd");
        command.args(["/c", "gradlew.bat", task]);
        command
    } else {
        let mut command = Command::new("./gradlew");
        command.arg(task);
        command
    };
    for (key, value) in properties {
        command.arg(format!("-P{key}={value}"));
    }
    command.current_dir(android_dir);
    run_command(&mut command, &format!("gradlew {task}"), json)
}

/// Gradle properties that keep the APK version metadata in sync with
/// `winia.toml` instead of the values baked into the generated build file.
pub(crate) fn version_properties(config: &WiniaConfig) -> Vec<(&'static str, String)> {
    vec![
        ("winiaVersionCode", config.android.version_code.to_string()),
        ("winiaVersionName", config.app.version.clone()),
    ]
}

fn run_gradle(
    android_dir: &Path,
    config: &WiniaConfig,
    release: bool,
    json: bool,
) -> Result<PathBuf> {
    let task = if release {
        "assembleRelease"
    } else {
        "assembleDebug"
    };
    run_gradle_task(android_dir, task, &version_properties(config), json)?;

    let output_dir =
        android_dir
            .join("app/build/outputs/apk")
            .join(if release { "release" } else { "debug" });

    let preferred: &[&str] = if release {
        &["app-release-unsigned.apk", "app-release.apk"]
    } else {
        &["app-debug.apk"]
    };
    for name in preferred {
        let candidate = output_dir.join(name);
        if candidate.is_file() {
            return Ok(candidate);
        }
    }

    let mut apks = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&output_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|extension| extension.to_str()) == Some("apk") {
                apks.push(path);
            }
        }
    }
    apks.sort_by_key(|path| {
        std::fs::metadata(path)
            .and_then(|metadata| metadata.modified())
            .ok()
    });
    apks.pop().ok_or_else(|| {
        anyhow::anyhow!(
            "Gradle finished but no APK was found in {}",
            output_dir.display()
        )
    })
}

fn ensure_rust_targets(abis: &[Abi]) -> Result<()> {
    let output = match Command::new("rustup")
        .args(["target", "list", "--installed"])
        .output()
    {
        Ok(output) if output.status.success() => output,
        _ => return Ok(()),
    };
    let installed: Vec<String> = String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_owned)
        .collect();
    let missing: Vec<&str> = abis
        .iter()
        .map(|abi| abi.rust_triple())
        .filter(|triple| !installed.iter().any(|item| item == triple))
        .collect();
    if missing.is_empty() {
        return Ok(());
    }
    bail!(
        "missing Rust targets: {} (run `rustup target add {}`)",
        missing.join(", "),
        missing.join(" ")
    )
}

struct AndroidToolchain {
    ndk: PathBuf,
    bin: PathBuf,
    min_sdk: u32,
}

impl AndroidToolchain {
    fn discover(config: &WiniaConfig) -> Result<Self> {
        let ndk = find_ndk()?;
        let bin = ndk_toolchain_bin(&ndk)?;
        Ok(Self {
            ndk,
            bin,
            min_sdk: config.android.min_sdk,
        })
    }

    /// Check that every selected ABI has an NDK clang wrapper for `min_sdk`.
    fn validate(&self, abis: &[Abi]) -> Result<()> {
        for abi in abis {
            let clang = self.clang(*abi, false);
            if clang.is_file() {
                continue;
            }
            let available = self.available_api_levels(*abi);
            let available = if available.is_empty() {
                "none".to_owned()
            } else {
                available.join(", ")
            };
            bail!(
                "the NDK at {} has no clang wrapper for {} API {} (available API levels: {})",
                self.ndk.display(),
                abi.rust_triple(),
                self.min_sdk,
                available
            );
        }
        Ok(())
    }

    fn available_api_levels(&self, abi: Abi) -> Vec<String> {
        let prefix = abi.clang_triple();
        let mut levels = Vec::new();
        if let Ok(entries) = std::fs::read_dir(&self.bin) {
            for entry in entries.flatten() {
                let name = entry.file_name();
                let Some(name) = name.to_str() else {
                    continue;
                };
                let Some(rest) = name.strip_prefix(prefix) else {
                    continue;
                };
                let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
                if digits.is_empty() || !rest[digits.len()..].starts_with("-clang") {
                    continue;
                }
                if !levels.contains(&digits) {
                    levels.push(digits);
                }
            }
        }
        levels.sort_by_key(|level| level.parse::<u32>().unwrap_or(0));
        levels
    }

    fn clang(&self, abi: Abi, cxx: bool) -> PathBuf {
        let mut name = format!(
            "{}{}-clang{}",
            abi.clang_triple(),
            self.min_sdk,
            if cxx { "++" } else { "" }
        );
        if cfg!(windows) {
            name.push_str(".cmd");
        }
        self.bin.join(name)
    }

    fn environment(&self, abi: Abi) -> Vec<(String, String)> {
        let triple = abi.rust_triple();
        let underscored = triple.replace('-', "_");
        let cargo_key = underscored.to_uppercase();
        let clang = self.clang(abi, false);
        let cxx = self.clang(abi, true);
        let ar = self.bin.join(if cfg!(windows) {
            "llvm-ar.exe"
        } else {
            "llvm-ar"
        });

        let mut environment = vec![
            (format!("CARGO_TARGET_{cargo_key}_LINKER"), display(&clang)),
            (format!("CC_{triple}"), display(&clang)),
            (format!("CC_{underscored}"), display(&clang)),
            (format!("CXX_{triple}"), display(&cxx)),
            (format!("CXX_{underscored}"), display(&cxx)),
            (format!("AR_{triple}"), display(&ar)),
            (format!("AR_{underscored}"), display(&ar)),
            ("ANDROID_NDK".to_owned(), display(&self.ndk)),
            ("ANDROID_NDK_HOME".to_owned(), display(&self.ndk)),
            ("ANDROID_NDK_ROOT".to_owned(), display(&self.ndk)),
        ];

        if let Some(llvm) = llvm_bin() {
            if std::env::var_os("LIBCLANG_PATH").is_none() {
                environment.push(("LIBCLANG_PATH".to_owned(), display(&llvm)));
            }
            if std::env::var_os("BINDGEN_EXTRA_CLANG_ARGS").is_none() {
                if let Some(resource_dir) = clang_resource_dir(&llvm) {
                    environment.push((
                        "BINDGEN_EXTRA_CLANG_ARGS".to_owned(),
                        format!("-resource-dir {}", display(&resource_dir)),
                    ));
                }
            }
        }

        if let Some(encoded) = std::env::var_os("CARGO_ENCODED_RUSTFLAGS") {
            let mut encoded = encoded.to_string_lossy().into_owned();
            if !encoded.contains("max-page-size") {
                encoded.push('\u{1f}');
                encoded.push_str("-C\u{1f}link-arg=-Wl,-z,max-page-size=16384");
            }
            environment.push(("CARGO_ENCODED_RUSTFLAGS".to_owned(), encoded));
        } else {
            let mut flags = std::env::var("RUSTFLAGS").unwrap_or_default();
            if !flags.contains("max-page-size") {
                flags.push_str(" -C link-arg=-Wl,-z,max-page-size=16384");
            }
            environment.push(("RUSTFLAGS".to_owned(), flags));
        }

        environment
    }
}

fn find_ndk() -> Result<PathBuf> {
    for variable in ["ANDROID_NDK", "ANDROID_NDK_HOME", "ANDROID_NDK_ROOT"] {
        if let Some(value) = std::env::var_os(variable).filter(|value| !value.is_empty()) {
            let path = PathBuf::from(value);
            if path.is_dir() {
                return Ok(path);
            }
        }
    }
    for variable in ["ANDROID_HOME", "ANDROID_SDK_ROOT"] {
        if let Some(value) = std::env::var_os(variable).filter(|value| !value.is_empty()) {
            let ndk_root = PathBuf::from(value).join("ndk");
            let mut versions: Vec<PathBuf> = std::fs::read_dir(&ndk_root)
                .map(|entries| {
                    entries
                        .flatten()
                        .map(|entry| entry.path())
                        .filter(|path| path.is_dir())
                        .collect()
                })
                .unwrap_or_default();
            versions.sort();
            if versions.iter().any(|path| {
                path.file_name().and_then(|name| name.to_str()) == Some("27.0.12077973")
            }) {
                return Ok(versions
                    .into_iter()
                    .find(|path| {
                        path.file_name().and_then(|name| name.to_str()) == Some("27.0.12077973")
                    })
                    .expect("just checked"));
            }
            if let Some(latest) = versions.pop() {
                return Ok(latest);
            }
        }
    }
    bail!("Android NDK not found; set ANDROID_NDK or install one under <sdk>/ndk")
}

fn ndk_toolchain_bin(ndk: &Path) -> Result<PathBuf> {
    let prebuilt = ndk.join("toolchains/llvm/prebuilt");
    let host_tag = if cfg!(windows) {
        "windows-x86_64"
    } else if cfg!(target_os = "macos") {
        "darwin-x86_64"
    } else {
        "linux-x86_64"
    };
    let preferred = prebuilt.join(host_tag).join("bin");
    if preferred.is_dir() {
        return Ok(preferred);
    }
    let mut candidates: Vec<PathBuf> = std::fs::read_dir(&prebuilt)
        .map(|entries| {
            entries
                .flatten()
                .map(|entry| entry.path().join("bin"))
                .filter(|path| path.is_dir())
                .collect()
        })
        .unwrap_or_default();
    candidates.sort();
    candidates
        .pop()
        .ok_or_else(|| anyhow::anyhow!("no LLVM toolchain found under {}", prebuilt.display()))
}

fn llvm_bin() -> Option<PathBuf> {
    if let Some(value) = std::env::var_os("LIBCLANG_PATH") {
        let path = PathBuf::from(value);
        if path.is_dir() {
            return Some(path);
        }
    }
    let candidates = if cfg!(windows) {
        vec![PathBuf::from("C:\\Program Files\\LLVM\\bin")]
    } else if cfg!(target_os = "macos") {
        vec![
            PathBuf::from("/opt/homebrew/opt/llvm/bin"),
            PathBuf::from("/usr/local/opt/llvm/bin"),
        ]
    } else {
        vec![
            PathBuf::from("/usr/lib/llvm-18/bin"),
            PathBuf::from("/usr/lib/llvm-17/bin"),
        ]
    };
    candidates.into_iter().find(|path| path.is_dir())
}

fn clang_resource_dir(llvm_bin: &Path) -> Option<PathBuf> {
    let clang_root = llvm_bin.parent()?.join("lib/clang");
    let mut versions: Vec<PathBuf> = std::fs::read_dir(&clang_root)
        .map(|entries| {
            entries
                .flatten()
                .map(|entry| entry.path())
                .filter(|path| path.is_dir())
                .collect()
        })
        .unwrap_or_default();
    versions.sort();
    versions.pop()
}

fn verify_alignment(toolchain: &AndroidToolchain, library: &Path) -> Result<()> {
    let readelf = toolchain.bin.join(if cfg!(windows) {
        "llvm-readelf.exe"
    } else {
        "llvm-readelf"
    });
    if !readelf.is_file() {
        return Ok(());
    }
    let output = Command::new(&readelf)
        .arg("-l")
        .arg(library)
        .output()
        .with_context(|| format!("running {}", display(&readelf)))?;
    if !output.status.success() {
        bail!(
            "llvm-readelf failed on {}: {}",
            library.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let mut saw_load = false;
    for line in text.lines() {
        let line = line.trim_start();
        if !line.starts_with("LOAD") {
            continue;
        }
        saw_load = true;
        let alignment = line.split_whitespace().last().unwrap_or_default();
        if alignment != "0x4000" {
            bail!(
                "{} has a LOAD segment aligned to {} (expected 0x4000)",
                library.display(),
                alignment
            );
        }
    }
    if !saw_load {
        bail!("no LOAD segments found in {}", library.display());
    }
    Ok(())
}

fn cargo_metadata(root: &Path) -> Result<ProjectMetadata> {
    let output = Command::new("cargo")
        .args(["metadata", "--format-version", "1", "--no-deps"])
        .current_dir(root)
        .output()
        .context("running cargo metadata")?;
    if !output.status.success() {
        bail!(
            "cargo metadata failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    let value: serde_json::Value =
        serde_json::from_slice(&output.stdout).context("parsing cargo metadata")?;
    let target_directory = PathBuf::from(
        value["target_directory"]
            .as_str()
            .context("cargo metadata: target_directory")?,
    );
    let packages = value["packages"]
        .as_array()
        .context("cargo metadata: packages")?;
    let manifest = root.join("Cargo.toml");
    let package = packages
        .iter()
        .find(|package| {
            package["manifest_path"]
                .as_str()
                .map(|path| Path::new(path) == manifest.as_path())
                .unwrap_or(false)
        })
        .or_else(|| packages.first())
        .context("cargo metadata: no packages")?;
    let targets = package["targets"]
        .as_array()
        .context("cargo metadata: targets")?;
    let binary_name = targets
        .iter()
        .find(|target| has_kind(target, "bin"))
        .and_then(|target| target["name"].as_str())
        .map(str::to_owned)
        .unwrap_or_else(|| package["name"].as_str().unwrap_or("app").replace('-', "_"));
    let library_name = targets
        .iter()
        .find(|target| has_kind(target, "cdylib") || has_kind(target, "lib"))
        .and_then(|target| target["name"].as_str())
        .map(|name| name.replace('-', "_"))
        .unwrap_or_else(|| binary_name.clone());
    Ok(ProjectMetadata {
        target_directory,
        binary_name,
        library_name,
    })
}

fn has_kind(target: &serde_json::Value, kind: &str) -> bool {
    target["kind"]
        .as_array()
        .map(|kinds| kinds.iter().any(|value| value.as_str() == Some(kind)))
        .unwrap_or(false)
}

fn run_command(command: &mut Command, label: &str, json: bool) -> Result<()> {
    if json {
        let output = command
            .output()
            .with_context(|| format!("running {label}"))?;
        if !output.status.success() {
            bail!(
                "{label} failed\nstdout:\n{}\nstderr:\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
        }
    } else {
        let status = command
            .status()
            .with_context(|| format!("running {label}"))?;
        if !status.success() {
            bail!("{label} failed");
        }
    }
    Ok(())
}

fn artifact(path: PathBuf) -> Result<Artifact> {
    let bytes = std::fs::metadata(&path)
        .with_context(|| format!("reading {}", path.display()))?
        .len();
    Ok(Artifact { path, bytes })
}

fn copy_artifacts(artifacts: &[&Artifact], out_dir: &Path) -> Result<()> {
    std::fs::create_dir_all(out_dir).with_context(|| format!("creating {}", out_dir.display()))?;
    for artifact in artifacts {
        let name = artifact
            .path
            .file_name()
            .context("artifact has no file name")?;
        let destination = out_dir.join(name);
        if artifact.path == destination {
            continue;
        }
        std::fs::copy(&artifact.path, &destination).with_context(|| {
            format!(
                "copying {} to {}",
                artifact.path.display(),
                destination.display()
            )
        })?;
    }
    Ok(())
}

fn report(
    root: &Path,
    built: &[(String, Vec<Artifact>)],
    profile: &str,
    elapsed: Duration,
    json: bool,
) -> Result<()> {
    if json {
        let artifact_values = |artifacts: &[Artifact]| -> Vec<serde_json::Value> {
            artifacts
                .iter()
                .map(|artifact| {
                    serde_json::json!({
                        "path": display_relative(root, &artifact.path).replace('\\', "/"),
                        "bytes": artifact.bytes,
                    })
                })
                .collect()
        };
        let value = if built.len() == 1 {
            let (target, artifacts) = &built[0];
            serde_json::json!({
                "target": target,
                "profile": profile,
                "artifacts": artifact_values(artifacts),
                "duration_ms": elapsed.as_millis(),
            })
        } else {
            let targets: Vec<_> = built
                .iter()
                .map(|(target, artifacts)| {
                    serde_json::json!({
                        "target": target,
                        "profile": profile,
                        "artifacts": artifact_values(artifacts),
                    })
                })
                .collect();
            serde_json::json!({
                "targets": targets,
                "duration_ms": elapsed.as_millis(),
            })
        };
        println!("{}", serde_json::to_string_pretty(&value)?);
        return Ok(());
    }

    for (target, artifacts) in built {
        println!("Built {target} ({profile})");
        for artifact in artifacts {
            println!(
                "  {} ({:.1} MB)",
                display_relative(root, &artifact.path),
                artifact.bytes as f64 / (1024.0 * 1024.0)
            );
        }
    }
    println!("Finished in {:.1}s", elapsed.as_secs_f64());
    Ok(())
}

fn display(path: &Path) -> String {
    path.display().to_string()
}

fn display_relative(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .map(|relative| relative.display().to_string())
        .unwrap_or_else(|_| path.display().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(target: &[&str], features: &[&str]) -> BuildArgs {
        BuildArgs {
            target: target.iter().map(|value| (*value).to_owned()).collect(),
            release: false,
            abi: Vec::new(),
            features: features.iter().map(|value| (*value).to_owned()).collect(),
            json: false,
            out_dir: None,
        }
    }

    #[test]
    fn merges_config_and_cli_features_without_duplicates() {
        let mut config = WiniaConfig::default();
        config.build.features = vec!["a".to_owned(), "b".to_owned()];
        let args = args(&[], &["b", "c"]);
        assert_eq!(effective_features(&config, &args), vec!["a", "b", "c"]);
    }

    #[test]
    fn all_expands_to_host_and_android() {
        let targets = resolve_targets(&["all".to_owned()]).unwrap();
        assert_eq!(targets.len(), 2);
        assert!(targets.contains(&host_target().to_owned()));
        assert!(targets.contains(&"android".to_owned()));
    }
}
