//! 'winia doctor' — check the host toolchain and platform SDKs.
//!
//! The report is read-only and intentionally conservative: it never claims an
//! environment is complete when a tool is missing. Checks are scoped by
//! `--target`: with `--target android` the Android prerequisites are required and
//! failures exit non-zero; without a target they are reported as warnings
//! because a desktop build does not need them.

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

pub fn run(args: DoctorArgs) -> Result<()> {
    let target = parse_target(args.target.as_deref())?;
    let android_required = target == Some(Target::Android);
    let checks = collect_checks(target, android_required);

    if args.json {
        print_json(&checks, target, args.strict);
    } else {
        print_report(&checks, target, args.fix);
    }

    if !passed(&checks, args.strict) {
        std::process::exit(1);
    }
    Ok(())
}

fn collect_checks(target: Option<Target>, android_required: bool) -> Vec<Check> {
    let mut checks = vec![
        command_check("rustc", &["--version"], true),
        command_check("cargo", &["--version"], true),
        command_check("git", &["--version"], false),
    ];

    match target {
        Some(Target::Android) | None => {
            checks.push(android_sdk_check(android_required));
            checks.push(android_ndk_check(android_required));
            checks.push(adb_check(android_required));
            checks.push(java_check(android_required));
        }
        Some(Target::Windows) | Some(Target::Macos) | Some(Target::Linux) => {}
    }

    checks.push(config_check());
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

fn print_report(checks: &[Check], target: Option<Target>, fix: bool) {
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
    if fix {
        println!("  Auto-fix is not implemented yet; install the missing tools manually.");
    }
}

fn print_json(checks: &[Check], target: Option<Target>, strict: bool) {
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
    let report = serde_json::json!({
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
    println!(
        "{}",
        serde_json::to_string_pretty(&report).expect("doctor report should serialise")
    );
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

    // Neither variable is set: scan the SDK for installed NDKs so the report can
    // suggest a concrete value instead of only saying "not set".
    if let Some(sdk) = android_sdk_root() {
        let ndk_dir = sdk.join("ndk");
        let mut installed = Vec::new();
        if let Ok(entries) = std::fs::read_dir(&ndk_dir) {
            for entry in entries.flatten() {
                let name = entry.file_name();
                if entry.path().is_dir() {
                    if let Some(name) = name.to_str() {
                        installed.push(name.to_owned());
                    }
                }
            }
        }
        installed.sort();
        if !installed.is_empty() {
            return Check::missing(
                "Android NDK",
                format!(
                    "ANDROID_NDK is not set; installed under {}: {} (set ANDROID_NDK to one of them)",
                    ndk_dir.display(),
                    installed.join(", ")
                ),
                required,
            );
        }
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
}
