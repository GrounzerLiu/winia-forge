//! 'winia doctor' — check the host toolchain and platform SDKs.
//!
//! The report is read-only and intentionally conservative: it never claims an
//! environment is complete when a tool is missing. Checks are scoped to the
//! selected target: `--target android` requires the Android prerequisites while
//! platform checks for other platforms are skipped; without a target the host
//! platform and the optional Android prerequisites are reported (Android items
//! as warnings). `--json` and `--strict` make the command usable from CI, and
//! `--fix` applies safe fixes (installing missing Rust targets) while printing
//! the exact command for everything that needs manual work.

use crate::cli::DoctorArgs;
use crate::config::WiniaConfig;
use anyhow::{Result, bail};
use std::path::{Path, PathBuf};
use std::process::Command;

/// The platform whose prerequisites the report treats as required.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Target {
    Windows,
    Macos,
    Linux,
    Android,
}

impl Target {
    fn as_str(self) -> &'static str {
        match self {
            Self::Windows => "windows",
            Self::Macos => "macos",
            Self::Linux => "linux",
            Self::Android => "android",
        }
    }
}

/// Outcome of a single check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Status {
    Ok,
    /// Not configured, but not required for the selected target.
    Warning,
    Missing,
}

impl Status {
    fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Warning => "warning",
            Self::Missing => "missing",
        }
    }
}

#[derive(Debug)]
struct Check {
    name: &'static str,
    status: Status,
    detail: String,
    /// A missing required check makes the command exit non-zero.
    required: bool,
}

impl Check {
    fn ok(name: &'static str, detail: impl Into<String>, required: bool) -> Self {
        Self {
            name,
            status: Status::Ok,
            detail: detail.into(),
            required,
        }
    }

    fn missing(name: &'static str, detail: impl Into<String>, required: bool) -> Self {
        Self {
            name,
            status: if required {
                Status::Missing
            } else {
                Status::Warning
            },
            detail: detail.into(),
            required,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Summary {
    total: usize,
    ok: usize,
    warning: usize,
    missing: usize,
}

impl Summary {
    fn from_checks(checks: &[Check]) -> Self {
        let mut summary = Self {
            total: checks.len(),
            ok: 0,
            warning: 0,
            missing: 0,
        };
        for check in checks {
            match check.status {
                Status::Ok => summary.ok += 1,
                Status::Warning => summary.warning += 1,
                Status::Missing => summary.missing += 1,
            }
        }
        summary
    }
}

/// A remediation derived from a failed or warning check.
#[derive(Debug)]
enum FixKind {
    /// Safe to execute without further input.
    Run { program: String, args: Vec<String> },
    /// Requires the user; the command is printed verbatim.
    Manual,
}

#[derive(Debug)]
struct FixAction {
    check: &'static str,
    kind: FixKind,
    command: String,
}

impl FixAction {
    fn run(check: &'static str, program: &str, args: Vec<String>) -> Self {
        Self {
            check,
            command: format!("{program} {}", args.join(" ")),
            kind: FixKind::Run {
                program: program.to_owned(),
                args,
            },
        }
    }

    fn manual(check: &'static str, command: String) -> Self {
        Self {
            check,
            command,
            kind: FixKind::Manual,
        }
    }

    fn mode(&self) -> &'static str {
        match &self.kind {
            FixKind::Run { .. } => "run",
            FixKind::Manual => "manual",
        }
    }
}

pub fn run(args: DoctorArgs) -> Result<()> {
    let target = parse_target(args.target.as_deref())?;
    let android_required = target == Some(Target::Android);
    let mut checks = collect_checks(target, android_required);
    let mut fixes = None;

    if args.fix {
        let planned = plan_fixes(&checks, target);
        apply_fixes(&planned, args.json);
        checks = collect_checks(target, android_required);
        fixes = Some(planned);
    }

    if args.json {
        print_json(&checks, target, args.strict, fixes.as_deref());
    } else {
        print_report(&checks, target);
    }

    if !passed(&checks, args.strict) {
        std::process::exit(1);
    }
    Ok(())
}

fn collect_checks(target: Option<Target>, android_required: bool) -> Vec<Check> {
    let mut checks = vec![
        rustc_check(),
        command_check("cargo", &["--version"], true),
        command_check("git", &["--version"], false),
        rustup_targets_check(target),
    ];

    if target == Some(Target::Windows) || (target.is_none() && cfg!(windows)) {
        if cfg!(windows) {
            checks.extend(windows_checks());
        } else {
            checks.push(Check {
                name: "Windows toolchain",
                status: Status::Warning,
                detail: "Windows target builds need a Windows host or cargo-xwin; local MSVC checks skipped"
                    .to_owned(),
                required: false,
            });
        }
    }

    if target == Some(Target::Macos) || (target.is_none() && cfg!(target_os = "macos")) {
        if cfg!(target_os = "macos") {
            checks.push(macos_check());
        } else {
            checks.push(Check {
                name: "macOS toolchain",
                status: Status::Warning,
                detail: "macOS applications can only be built on macOS; Xcode checks skipped"
                    .to_owned(),
                required: false,
            });
        }
    }

    if target == Some(Target::Linux) || (target.is_none() && cfg!(target_os = "linux")) {
        if cfg!(target_os = "linux") {
            checks.extend(linux_checks());
        } else {
            checks.push(Check {
                name: "Linux toolchain",
                status: Status::Warning,
                detail: "Linux target builds need a Linux host or a cross toolchain; pkg-config checks skipped"
                    .to_owned(),
                required: false,
            });
        }
    }

    match target {
        Some(Target::Android) | None => {
            checks.push(android_sdk_check(android_required));
            checks.push(android_licenses_check(android_required));
            checks.push(android_ndk_check(android_required));
            checks.push(adb_check(android_required));
            checks.push(java_check(android_required));
        }
        Some(Target::Windows) | Some(Target::Macos) | Some(Target::Linux) => {}
    }

    checks.extend(packaging_checks(target));
    checks.push(config_check());
    checks
}

/// Installer toolchains. Missing tools are warnings, because packaging skips those formats
/// unless they were requested explicitly.
fn packaging_checks(target: Option<Target>) -> Vec<Check> {
    let mut checks = Vec::new();
    let windows = target == Some(Target::Windows) || (target.is_none() && cfg!(windows));
    let linux = target == Some(Target::Linux) || (target.is_none() && cfg!(target_os = "linux"));

    if windows {
        checks.push(match winia_pack::nsis::find_makensis() {
            Ok(path) => Check::ok("makensis", path.display().to_string(), false),
            Err(_) => Check::missing(
                "makensis",
                "not found; NSIS installers are skipped (winget install NSIS.NSIS)",
                false,
            ),
        });
        if cfg!(windows) {
            checks.push(if winia_pack::msi::wix_available() {
                Check::ok("WiX", "candle and light are available", false)
            } else {
                Check::missing(
                    "WiX",
                    "not found; MSI packages are skipped (choco install wixtoolset)",
                    false,
                )
            });
        }
    }

    if linux {
        checks.push(command_check("dpkg-deb", &["--version"], false));
    }

    checks.push(command_check("gh", &["--version"], false));
    checks
}

/// A report passes when no required check is missing; `--strict` also fails on
/// warnings (optional checks that are not configured).
fn passed(checks: &[Check], strict: bool) -> bool {
    let has_missing = checks
        .iter()
        .any(|check| check.required && check.status == Status::Missing);
    let has_warning = checks.iter().any(|check| check.status == Status::Warning);
    !has_missing && !(strict && has_warning)
}

fn print_report(checks: &[Check], target: Option<Target>) {
    println!("Winia Forge environment check");
    if let Some(target) = target {
        println!("Target focus: {}", target.as_str());
    }
    println!();
    for check in checks {
        println!(
            "  {:<18} {:<8} {}",
            check.name,
            check.status.as_str(),
            check.detail
        );
    }
    println!();
    let summary = Summary::from_checks(checks);
    println!(
        "  {} checks: {} ok, {} warning, {} missing",
        summary.total, summary.ok, summary.warning, summary.missing
    );
    if !passed(checks, false) {
        println!("  Required checks are missing; install the tools above and run again.");
    } else if summary.warning > 0 {
        println!(
            "  All required checks passed; warnings are optional for this target (use --strict to fail on them)."
        );
    } else {
        println!("  All required checks passed.");
    }
}

fn print_json(checks: &[Check], target: Option<Target>, strict: bool, fixes: Option<&[FixAction]>) {
    let summary = Summary::from_checks(checks);
    let values: Vec<_> = checks
        .iter()
        .map(|check| {
            serde_json::json!({
                "name": check.name,
                "status": check.status.as_str(),
                "detail": check.detail,
                "required": check.required,
            })
        })
        .collect();
    let mut report = serde_json::json!({
        "target": target.map(Target::as_str),
        "checks": values,
        "summary": {
            "total": summary.total,
            "ok": summary.ok,
            "warning": summary.warning,
            "missing": summary.missing,
            "passed": passed(checks, strict),
        },
    });
    if let Some(fixes) = fixes {
        report["fixes"] = serde_json::json!(
            fixes
                .iter()
                .map(|fix| serde_json::json!({
                    "check": fix.check,
                    "command": fix.command,
                    "mode": fix.mode(),
                }))
                .collect::<Vec<_>>()
        );
    }
    println!(
        "{}",
        serde_json::to_string_pretty(&report).expect("doctor report should serialise")
    );
}

fn plan_fixes(checks: &[Check], target: Option<Target>) -> Vec<FixAction> {
    let mut fixes = Vec::new();
    for check in checks {
        if check.status == Status::Ok {
            continue;
        }
        match check.name {
            "Rust targets" if target == Some(Target::Android) => {
                let installed = installed_rust_targets().unwrap_or_default();
                let missing = missing_android_targets(&installed);
                if !missing.is_empty() {
                    let mut args = vec!["target".to_owned(), "add".to_owned()];
                    args.extend(missing.iter().cloned());
                    fixes.push(FixAction::run("Rust targets", "rustup", args));
                }
            }
            "SDK licenses" => {
                fixes.push(FixAction::manual(
                    "SDK licenses",
                    sdkmanager_license_command(),
                ));
            }
            "Android NDK" => {
                if let Some(path) = installed_ndks().last() {
                    fixes.push(FixAction::manual("Android NDK", ndk_env_command(path)));
                }
            }
            _ => {}
        }
    }
    fixes
}

fn apply_fixes(fixes: &[FixAction], quiet: bool) {
    if fixes.is_empty() {
        if !quiet {
            println!("No automatic fixes are available for the current findings.");
        }
        return;
    }
    if !quiet {
        println!("Applying fixes:");
    }
    for fix in fixes {
        if !quiet {
            println!("  [{}] {}", fix.check, fix.command);
        }
        match &fix.kind {
            FixKind::Run { program, args } => match Command::new(program).args(args).output() {
                Ok(output) if output.status.success() => {
                    if !quiet {
                        println!("      done");
                    }
                }
                Ok(output) => {
                    if !quiet {
                        let detail = first_non_empty(&output.stderr)
                            .or_else(|| first_non_empty(&output.stdout))
                            .unwrap_or_default();
                        println!("      failed ({}): {detail}", output.status);
                    }
                }
                Err(error) => {
                    if !quiet {
                        println!("      failed: {error}");
                    }
                }
            },
            FixKind::Manual => {
                if !quiet {
                    println!("      run this command manually");
                }
            }
        }
    }
    if !quiet {
        println!();
    }
}

fn parse_target(raw: Option<&str>) -> Result<Option<Target>> {
    Ok(match raw {
        None => None,
        Some("windows") => Some(Target::Windows),
        Some("macos") => Some(Target::Macos),
        Some("linux") => Some(Target::Linux),
        Some("android") => Some(Target::Android),
        Some(other) => {
            bail!("unknown target '{other}'; expected windows, macos, linux, or android")
        }
    })
}

fn rustc_check() -> Check {
    let required = env!("CARGO_PKG_RUST_VERSION");
    match version_line(Path::new("rustc"), &["--version"]) {
        Ok(detail) => match (parse_version(&detail), parse_version(required)) {
            (Some(actual), Some(minimum)) if actual >= minimum => {
                Check::ok("rustc", format!("{detail} (MSRV {required})"), true)
            }
            (Some(_), Some(_)) => Check::missing(
                "rustc",
                format!("{detail} is older than the required MSRV {required}"),
                true,
            ),
            _ => Check {
                name: "rustc",
                status: Status::Warning,
                detail: format!("{detail} (could not compare with MSRV {required})"),
                required: true,
            },
        },
        Err(detail) => Check::missing("rustc", detail, true),
    }
}

fn rustup_targets_check(target: Option<Target>) -> Check {
    match installed_rust_targets() {
        Some(targets) => {
            if target == Some(Target::Android) {
                let missing = missing_android_targets(&targets);
                if missing.is_empty() {
                    Check::ok(
                        "Rust targets",
                        format!("{} installed; Android targets present", targets.len()),
                        true,
                    )
                } else {
                    Check::missing(
                        "Rust targets",
                        format!(
                            "missing Android targets: {} (run `rustup target add {}`)",
                            missing.join(", "),
                            missing.join(" ")
                        ),
                        true,
                    )
                }
            } else {
                Check::ok(
                    "Rust targets",
                    format!("{} installed", targets.len()),
                    false,
                )
            }
        }
        None => Check {
            name: "Rust targets",
            status: Status::Warning,
            detail: "rustup is not available; install targets with your Rust distribution"
                .to_owned(),
            required: false,
        },
    }
}

fn windows_checks() -> Vec<Check> {
    vec![
        msvc_check(),
        windows_sdk_check(),
        llvm_check(),
        vulkan_check(),
    ]
}

fn msvc_check() -> Check {
    let Some(vswhere) = vswhere_path() else {
        return Check::missing(
            "MSVC Build Tools",
            "vswhere.exe not found; install Visual Studio Build Tools with the C++ workload",
            true,
        );
    };
    let args = [
        "-latest",
        "-products",
        "*",
        "-requires",
        "Microsoft.VisualStudio.Component.VC.Tools.x86.x64",
        "-property",
        "installationPath",
    ];
    match version_line(&vswhere, &args) {
        Ok(detail) => Check::ok("MSVC Build Tools", detail, true),
        Err(_) => Check::missing(
            "MSVC Build Tools",
            "no Visual Studio installation with the C++ build tools was found",
            true,
        ),
    }
}

fn windows_sdk_check() -> Check {
    let Some(root) = windows_sdk_include_root() else {
        return Check::missing(
            "Windows SDK",
            "Windows 10/11 SDK not found under Program Files (x86)\\Windows Kits\\10\\Include",
            true,
        );
    };
    let mut versions = installed_subdirs(&root);
    versions.sort();
    match versions.last() {
        Some(latest) => Check::ok(
            "Windows SDK",
            format!("{} (latest {latest})", root.display()),
            true,
        ),
        None => Check::missing(
            "Windows SDK",
            format!("{} contains no SDK versions", root.display()),
            true,
        ),
    }
}

fn llvm_check() -> Check {
    if let Ok(detail) = version_line(Path::new("clang"), &["--version"]) {
        return Check::ok("LLVM", detail, false);
    }
    if let Ok(detail) = version_line(Path::new("llvm-config"), &["--version"]) {
        return Check::ok("LLVM", detail, false);
    }
    if let Some(path) = std::env::var_os("LIBCLANG_PATH")
        .map(PathBuf::from)
        .filter(|path| path.is_dir())
    {
        return Check::ok("LLVM", format!("LIBCLANG_PATH = {}", path.display()), false);
    }
    Check {
        name: "LLVM",
        status: Status::Warning,
        detail: "clang/llvm-config not found and LIBCLANG_PATH is not set; needed when Skia builds from source"
            .to_owned(),
        required: false,
    }
}

fn vulkan_check() -> Check {
    if let Some(path) = std::env::var_os("VULKAN_SDK")
        .map(PathBuf::from)
        .filter(|path| path.is_dir())
    {
        return Check::ok(
            "Vulkan SDK",
            format!("VULKAN_SDK = {}", path.display()),
            false,
        );
    }
    Check {
        name: "Vulkan SDK",
        status: Status::Warning,
        detail: "VULKAN_SDK is not set (optional: only needed for the Vulkan backend)".to_owned(),
        required: false,
    }
}

fn macos_check() -> Check {
    match version_line(Path::new("xcode-select"), &["-p"]) {
        Ok(detail) => Check::ok("Xcode CLT", detail, true),
        Err(detail) => Check::missing("Xcode CLT", detail, true),
    }
}

fn linux_checks() -> Vec<Check> {
    let mut checks = Vec::new();
    checks.push(
        match version_line(Path::new("pkg-config"), &["--version"]) {
            Ok(detail) => Check::ok("pkg-config", detail, true),
            Err(detail) => Check::missing("pkg-config", detail, true),
        },
    );
    for (name, package) in [("fontconfig", "fontconfig"), ("libX11", "x11")] {
        checks.push(
            match version_line(Path::new("pkg-config"), &["--modversion", package]) {
                Ok(detail) => Check::ok(name, format!("{package} {detail}"), true),
                Err(_) => Check::missing(name, format!("{package} not found via pkg-config"), true),
            },
        );
    }
    checks
}

fn android_sdk_check(required: bool) -> Check {
    let paths = android_sdk_paths();
    if paths.is_empty() {
        return Check::missing(
            "Android SDK",
            "ANDROID_HOME (or ANDROID_SDK_ROOT) is not set",
            required,
        );
    }
    if let Some((var, path)) = paths.iter().find(|(_, path)| path.is_dir()) {
        return Check::ok(
            "Android SDK",
            format!("{var} = {}", path.display()),
            required,
        );
    }
    let (var, path) = &paths[0];
    Check::missing(
        "Android SDK",
        format!("{var} points to a missing directory: {}", path.display()),
        required,
    )
}

fn android_licenses_check(required: bool) -> Check {
    let Some(sdk) = android_sdk_root() else {
        return Check::missing(
            "SDK licenses",
            "Android SDK not found, licenses cannot be checked",
            required,
        );
    };
    let license = sdk.join("licenses").join("android-sdk-license");
    if license.is_file() {
        Check::ok("SDK licenses", license.display().to_string(), required)
    } else {
        Check::missing(
            "SDK licenses",
            format!(
                "{} is missing; run `sdkmanager --licenses`",
                license.display()
            ),
            required,
        )
    }
}

fn android_ndk_check(required: bool) -> Check {
    let candidates: Vec<(&'static str, PathBuf)> = ["ANDROID_NDK", "ANDROID_NDK_HOME"]
        .iter()
        .filter_map(|var| {
            std::env::var_os(var)
                .filter(|value| !value.is_empty())
                .map(|value| (*var, PathBuf::from(value)))
        })
        .collect();

    if let Some((var, path)) = candidates.iter().find(|(_, path)| path.is_dir()) {
        return Check::ok(
            "Android NDK",
            format!("{var} = {}", path.display()),
            required,
        );
    }
    if let Some((var, path)) = candidates.first() {
        return Check::missing(
            "Android NDK",
            format!("{var} points to a missing directory: {}", path.display()),
            required,
        );
    }

    let installed = installed_ndks();
    if !installed.is_empty() {
        let names: Vec<String> = installed
            .iter()
            .map(|path| {
                path.file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_else(|| path.display().to_string())
            })
            .collect();
        let parent = installed
            .first()
            .and_then(|path| path.parent())
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| "the Android SDK".to_owned());
        return Check::missing(
            "Android NDK",
            format!(
                "ANDROID_NDK is not set; installed under {parent}: {} (set ANDROID_NDK to one of them)",
                names.join(", ")
            ),
            required,
        );
    }

    Check::missing(
        "Android NDK",
        "ANDROID_NDK is not set and no NDK is installed in the Android SDK",
        required,
    )
}

fn adb_check(required: bool) -> Check {
    if let Ok(detail) = version_line(Path::new("adb"), &["version"]) {
        return Check::ok("adb", detail, required);
    }
    if let Some(sdk) = android_sdk_root() {
        let executable =
            sdk.join("platform-tools")
                .join(if cfg!(windows) { "adb.exe" } else { "adb" });
        if executable.is_file() {
            if let Ok(detail) = version_line(&executable, &["version"]) {
                return Check::ok(
                    "adb",
                    format!("{detail} (from {}, not on PATH)", executable.display()),
                    required,
                );
            }
        }
    }
    Check::missing(
        "adb",
        "adb is not on PATH and was not found in the Android SDK platform-tools",
        required,
    )
}

fn java_check(required: bool) -> Check {
    match std::env::var_os("JAVA_HOME").filter(|value| !value.is_empty()) {
        Some(value) => {
            let path = PathBuf::from(value);
            if path.is_dir() {
                Check::ok("JAVA_HOME", path.display().to_string(), required)
            } else {
                Check::missing(
                    "JAVA_HOME",
                    format!(
                        "JAVA_HOME points to a missing directory: {}",
                        path.display()
                    ),
                    required,
                )
            }
        }
        None => Check::missing("JAVA_HOME", "JAVA_HOME is not set", required),
    }
}

fn config_check() -> Check {
    let cwd = std::env::current_dir().unwrap_or_else(|_| Path::new(".").to_path_buf());
    match crate::config::find_config(&cwd) {
        None => Check {
            name: "winia.toml",
            status: Status::Warning,
            detail: "not found in this directory tree (run inside a project or create one with 'winia new <name>')"
                .to_owned(),
            required: false,
        },
        Some(path) => match load_and_validate(&path) {
            Ok(()) => Check::ok("winia.toml", path.display().to_string(), true),
            Err(error) => Check::missing(
                "winia.toml",
                format!("{}: {error:#}", path.display()),
                true,
            ),
        },
    }
}

fn load_and_validate(path: &Path) -> Result<()> {
    let config = WiniaConfig::load(path)?;
    config.validate()?;
    Ok(())
}

fn command_check(name: &'static str, args: &[&str], required: bool) -> Check {
    match version_line(Path::new(name), args) {
        Ok(detail) => Check::ok(name, detail, required),
        Err(detail) => Check::missing(name, detail, required),
    }
}

/// Run a version command and return its first non-empty output line. Some tools
/// (for example `java -version`) write the version to stderr, so both streams
/// are considered.
fn version_line(program: &Path, args: &[&str]) -> std::result::Result<String, String> {
    match Command::new(program).args(args).output() {
        Ok(output) => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);
            let line = stdout
                .lines()
                .chain(stderr.lines())
                .map(str::trim)
                .find(|line| !line.is_empty());
            match (output.status.success(), line) {
                (true, Some(line)) => Ok(line.to_owned()),
                (_, Some(line)) => Err(line.to_owned()),
                (false, None) => Err(format!(
                    "`{}` exited with {}",
                    program.display(),
                    output.status
                )),
                (true, None) => Err(format!(
                    "`{}` produced no version output",
                    program.display()
                )),
            }
        }
        Err(error) => Err(error.to_string()),
    }
}

/// Run a command and return every non-empty stdout line.
fn output_lines(program: &Path, args: &[&str]) -> std::result::Result<Vec<String>, String> {
    match Command::new(program).args(args).output() {
        Ok(output) if output.status.success() => Ok(String::from_utf8_lossy(&output.stdout)
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(str::to_owned)
            .collect()),
        Ok(output) => Err(first_non_empty(&output.stderr)
            .or_else(|| first_non_empty(&output.stdout))
            .unwrap_or_else(|| format!("`{}` exited with {}", program.display(), output.status))),
        Err(error) => Err(error.to_string()),
    }
}

fn first_non_empty(bytes: &[u8]) -> Option<String> {
    String::from_utf8_lossy(bytes)
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(str::to_owned)
}

fn parse_version(text: &str) -> Option<(u32, u32, u32)> {
    let token = text
        .split_whitespace()
        .find(|word| word.chars().next().is_some_and(|c| c.is_ascii_digit()))?;
    let mut parts = token.trim_start_matches('v').split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next().unwrap_or("0").parse().ok()?;
    let patch = parts
        .next()
        .unwrap_or("0")
        .split(|c: char| !c.is_ascii_digit())
        .next()
        .unwrap_or("0")
        .parse()
        .ok()?;
    Some((major, minor, patch))
}

fn android_sdk_paths() -> Vec<(&'static str, PathBuf)> {
    ["ANDROID_HOME", "ANDROID_SDK_ROOT"]
        .iter()
        .filter_map(|var| {
            std::env::var_os(var)
                .filter(|value| !value.is_empty())
                .map(|value| (*var, PathBuf::from(value)))
        })
        .collect()
}

fn android_sdk_root() -> Option<PathBuf> {
    let paths = android_sdk_paths();
    paths
        .iter()
        .map(|(_, path)| path.clone())
        .find(|path| path.is_dir())
        .or_else(|| paths.first().map(|(_, path)| path.clone()))
}

fn installed_ndks() -> Vec<PathBuf> {
    let Some(sdk) = android_sdk_root() else {
        return Vec::new();
    };
    let ndk_root = sdk.join("ndk");
    let mut ndks: Vec<PathBuf> = installed_subdirs(&ndk_root)
        .into_iter()
        .map(|name| ndk_root.join(name))
        .filter(|path| path.is_dir())
        .collect();
    ndks.sort();
    ndks
}

fn installed_subdirs(root: &Path) -> Vec<String> {
    let mut names = Vec::new();
    if let Ok(entries) = std::fs::read_dir(root) {
        for entry in entries.flatten() {
            if entry.path().is_dir() {
                if let Some(name) = entry.file_name().to_str() {
                    names.push(name.to_owned());
                }
            }
        }
    }
    names
}

fn project_abis() -> Vec<String> {
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    if let Some(path) = crate::config::find_config(&cwd) {
        if let Ok(config) = WiniaConfig::load(&path) {
            if !config.android.abis.is_empty() {
                return config.android.abis;
            }
        }
    }
    vec!["arm64-v8a".to_owned(), "x86_64".to_owned()]
}

fn abi_triples(abis: &[String]) -> Vec<String> {
    abis.iter()
        .filter_map(|abi| {
            crate::platform::android::Abi::parse(abi)
                .ok()
                .map(|abi| abi.rust_triple().to_owned())
        })
        .collect()
}

fn installed_rust_targets() -> Option<Vec<String>> {
    output_lines(Path::new("rustup"), &["target", "list", "--installed"]).ok()
}

fn missing_android_targets(installed: &[String]) -> Vec<String> {
    abi_triples(&project_abis())
        .into_iter()
        .filter(|triple| !installed.iter().any(|item| item == triple))
        .collect()
}

fn vswhere_path() -> Option<PathBuf> {
    let base = std::env::var_os("ProgramFiles(x86)")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("C:\\Program Files (x86)"));
    let path = base
        .join("Microsoft Visual Studio")
        .join("Installer")
        .join("vswhere.exe");
    path.is_file().then_some(path)
}

fn windows_sdk_include_root() -> Option<PathBuf> {
    let base = std::env::var_os("ProgramFiles(x86)")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("C:\\Program Files (x86)"));
    let root = base.join("Windows Kits").join("10").join("Include");
    root.is_dir().then_some(root)
}

fn sdkmanager_path() -> Option<PathBuf> {
    let sdk = android_sdk_root()?;
    let executable = if cfg!(windows) {
        "sdkmanager.bat"
    } else {
        "sdkmanager"
    };
    let mut candidates = installed_subdirs(&sdk.join("cmdline-tools"));
    candidates.sort();
    for version in candidates.into_iter().rev() {
        let path = sdk
            .join("cmdline-tools")
            .join(version)
            .join("bin")
            .join(executable);
        if path.is_file() {
            return Some(path);
        }
    }
    let legacy = sdk.join("tools").join("bin").join(executable);
    legacy.is_file().then_some(legacy)
}

fn sdkmanager_license_command() -> String {
    match sdkmanager_path() {
        Some(path) => format!("\"{}\" --licenses", path.display()),
        None => "sdkmanager --licenses (install the Android cmdline-tools first)".to_owned(),
    }
}

fn ndk_env_command(path: &Path) -> String {
    if cfg!(windows) {
        format!("setx ANDROID_NDK \"{}\"", path.display())
    } else {
        format!("export ANDROID_NDK=\"{}\"", path.display())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn check(name: &'static str, status: Status, required: bool) -> Check {
        Check {
            name,
            status,
            detail: String::new(),
            required,
        }
    }

    #[test]
    fn summary_counts_each_status() {
        let checks = [
            check("a", Status::Ok, true),
            check("b", Status::Warning, false),
            check("c", Status::Missing, true),
        ];
        let summary = Summary::from_checks(&checks);
        assert_eq!(summary.total, 3);
        assert_eq!(summary.ok, 1);
        assert_eq!(summary.warning, 1);
        assert_eq!(summary.missing, 1);
    }

    #[test]
    fn a_missing_required_check_fails_the_report() {
        let checks = [
            check("a", Status::Ok, true),
            check("b", Status::Missing, true),
        ];
        assert!(!passed(&checks, false));
    }

    #[test]
    fn warnings_only_fail_in_strict_mode() {
        let checks = [
            check("a", Status::Ok, true),
            check("b", Status::Warning, false),
        ];
        assert!(passed(&checks, false));
        assert!(!passed(&checks, true));
    }

    #[test]
    fn parses_supported_targets_and_rejects_unknown_ones() {
        assert_eq!(parse_target(None).unwrap(), None);
        assert_eq!(
            parse_target(Some("android")).unwrap(),
            Some(Target::Android)
        );
        assert_eq!(
            parse_target(Some("windows")).unwrap(),
            Some(Target::Windows)
        );
        assert!(parse_target(Some("ios")).is_err());
    }

    #[test]
    fn parses_rustc_and_manifest_versions() {
        assert_eq!(
            parse_version("rustc 1.99.1 (58cf8f9c3 2026-10-13)"),
            Some((1, 99, 1))
        );
        assert_eq!(parse_version("1.85"), Some((1, 85, 0)));
        assert_eq!(
            parse_version("rustc 1.90.0-nightly (abc 2026-01-01)"),
            Some((1, 90, 0))
        );
    }

    #[test]
    fn maps_android_abis_to_rust_triples() {
        let abis = vec![
            "arm64-v8a".to_owned(),
            "x86_64".to_owned(),
            "armeabi-v7a".to_owned(),
            "x86".to_owned(),
            "unknown".to_owned(),
        ];
        assert_eq!(
            abi_triples(&abis),
            vec![
                "aarch64-linux-android",
                "x86_64-linux-android",
                "armv7-linux-androideabi",
                "i686-linux-android",
            ]
        );
    }
}
