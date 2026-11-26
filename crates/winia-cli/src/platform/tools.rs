//! Android SDK tool discovery shared by packaging and signing.

use anyhow::{Result, bail};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

pub(crate) const PYTHON3_REQUIREMENT: &str = "Skia's GN build invokes `python3`; install Python 3 and make sure `python3 --version` succeeds on PATH";

/// Probe the exact command Skia's generated Ninja rules invoke. On Windows this goes through
/// `cmd.exe`, so a `python3.cmd` shim works and a broken WindowsApps alias is reported as a failure.
pub(crate) fn python3_version() -> std::result::Result<String, String> {
    let mut command = if cfg!(windows) {
        let mut command = Command::new("cmd");
        command.args(["/c", "python3", "--version"]);
        command
    } else {
        let mut command = Command::new("python3");
        command.arg("--version");
        command
    };
    let output = command
        .output()
        .map_err(|error| format!("running `python3 --version`: {error}"))?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let line = stdout
        .lines()
        .chain(stderr.lines())
        .map(str::trim)
        .find(|line| !line.is_empty())
        .ok_or_else(|| "`python3 --version` produced no output".to_owned())?;
    if !output.status.success() {
        return Err(format!(
            "{line} (`python3 --version` exited with {})",
            output.status
        ));
    }
    if !is_python3_version(line) {
        return Err(format!("python3 did not report Python 3: {line}"));
    }
    Ok(line.to_owned())
}

fn is_python3_version(line: &str) -> bool {
    line.starts_with("Python 3.")
}

/// The newest installed build-tools directory, ordered by numeric version parts.
pub(crate) fn build_tools_dir() -> Option<PathBuf> {
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
            versions.sort_by_key(|path| version_key(path));
            if let Some(latest) = versions.pop() {
                return Some(latest);
            }
        }
    }
    None
}

/// Build a command that runs `apksigner`. The jar is preferred (it avoids
/// `cmd.exe` re-parsing paths with spaces); the batch launcher is the fallback.
pub(crate) fn apksigner_command(build_tools: &Path) -> Result<Command> {
    let jar = build_tools.join("lib").join("apksigner.jar");
    if jar.is_file()
        && let Some(java) = java_program()
    {
        let mut command = Command::new(java);
        command.arg("-jar").arg(jar);
        return Ok(command);
    }
    let launcher = build_tools.join(if cfg!(windows) {
        "apksigner.bat"
    } else {
        "apksigner"
    });
    if !launcher.is_file() {
        bail!("apksigner not found under {}", build_tools.display());
    }
    if cfg!(windows) {
        // cmd /c treats a quoted batch path with additional arguments as its own command line
        // and can drop those arguments. "call" preserves the batch arguments while still
        // accepting a launcher path that contains spaces.
        let mut command = Command::new("cmd");
        command.args(["/c", "call"]).arg(launcher);
        Ok(command)
    } else {
        Ok(Command::new(launcher))
    }
}

fn java_program() -> Option<PathBuf> {
    if let Some(home) = std::env::var_os("JAVA_HOME").filter(|value| !value.is_empty()) {
        let candidate =
            PathBuf::from(home)
                .join("bin")
                .join(if cfg!(windows) { "java.exe" } else { "java" });
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    if Command::new("java")
        .arg("-version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
    {
        return Some(PathBuf::from("java"));
    }
    None
}

pub(crate) fn executable(name: &str) -> String {
    if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_owned()
    }
}

fn version_key(path: &Path) -> Vec<u32> {
    path.file_name()
        .and_then(|name| name.to_str())
        .map(|name| {
            name.split('.')
                .map(|part| part.parse::<u32>().unwrap_or(0))
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn python3_version_requires_a_python_3_banner() {
        assert!(is_python3_version("Python 3.14.10"));
        assert!(!is_python3_version("Python 2.7.18"));
        assert!(!is_python3_version("python3: command not found"));
        assert!(!is_python3_version(""));
    }

    #[test]
    fn build_tools_versions_sort_numerically() {
        let mut versions = vec![
            PathBuf::from("10.0.0"),
            PathBuf::from("9.0.0"),
            PathBuf::from("36.1.0"),
        ];
        versions.sort_by_key(|path| version_key(path));
        assert_eq!(
            versions,
            vec![
                PathBuf::from("9.0.0"),
                PathBuf::from("10.0.0"),
                PathBuf::from("36.1.0"),
            ]
        );
    }
}
