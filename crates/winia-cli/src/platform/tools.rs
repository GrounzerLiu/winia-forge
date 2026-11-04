//! Android SDK tool discovery shared by packaging and signing.

use std::path::{Path, PathBuf};

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
