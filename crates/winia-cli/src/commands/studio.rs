//! 'winia android studio' opens the generated Android project in Android Studio.

use anyhow::{Context, Result, bail};
use std::path::{Path, PathBuf};

pub fn open(android_dir: &Path) -> Result<()> {
    if !android_dir.is_dir() {
        bail!(
            "{} does not exist; create the Android project first",
            android_dir.display()
        );
    }
    let studio = match find_studio() {
        Some(path) => path,
        None => bail!(
            "Android Studio was not found; set WINIA_ANDROID_STUDIO to the studio executable or open {} manually",
            android_dir.display()
        ),
    };
    let mut child = std::process::Command::new(&studio)
        .arg(android_dir)
        .spawn()
        .with_context(|| format!("launching {}", studio.display()))?;
    std::thread::sleep(std::time::Duration::from_millis(700));
    if let Some(status) = child.try_wait()? {
        bail!(
            "Android Studio exited immediately with {status}; check the launcher at {}",
            studio.display()
        );
    }
    println!("Opened {} in Android Studio", android_dir.display());
    Ok(())
}

/// WINIA_ANDROID_STUDIO, then PATH, then the default install locations per platform.
fn find_studio() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("WINIA_ANDROID_STUDIO") {
        let path = PathBuf::from(path);
        if path.is_file() {
            return Some(path);
        }
    }
    let names: &[&str] = if cfg!(windows) {
        &["studio64.exe", "studio.exe"]
    } else {
        &["studio"]
    };
    if let Some(path) = std::env::var_os("PATH") {
        for directory in std::env::split_paths(&path) {
            for name in names {
                let candidate = directory.join(name);
                if candidate.is_file() {
                    return Some(candidate);
                }
            }
        }
    }
    let mut candidates: Vec<PathBuf> = Vec::new();
    if cfg!(windows) {
        for variable in ["LOCALAPPDATA", "ProgramFiles", "ProgramFiles(x86)"] {
            if let Some(base) = std::env::var_os(variable) {
                let base = PathBuf::from(base);
                candidates.push(base.join("Programs/Android Studio/bin/studio64.exe"));
                candidates.push(base.join("Android/Android Studio/bin/studio64.exe"));
            }
        }
    } else if cfg!(target_os = "macos") {
        candidates.push(PathBuf::from(
            "/Applications/Android Studio.app/Contents/MacOS/studio",
        ));
    } else {
        candidates.push(PathBuf::from("/usr/local/bin/studio"));
        candidates.push(PathBuf::from("/opt/android-studio/bin/studio.sh"));
    }
    candidates.into_iter().find(|path| path.is_file())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_directory_is_reported() {
        let error = open(Path::new("definitely-not-here")).unwrap_err();
        assert!(error.to_string().contains("does not exist"));
    }
}
