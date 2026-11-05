//! Incremental edits to the project's AndroidManifest.xml.
//!
//! The manifest belongs to the project: users add activities, services, deep links and their own
//! permissions. 'winia android sync' therefore only rewrites the block between two markers and
//! leaves every other byte alone. Without the markers the manifest is reported as skipped.

use anyhow::{Result, bail};

pub const BEGIN: &str = "<!-- winia:permissions:begin -->";
pub const END: &str = "<!-- winia:permissions:end -->";

/// Permissions a Winia app needs even when '[android].permissions' is empty: the debug server
/// binds a socket, so INTERNET is always required.
const DEFAULT_PERMISSIONS: &[&str] = &["android.permission.INTERNET"];

/// Turn short names ("INTERNET") into fully qualified ones and keep qualified names as written.
pub fn qualified(permissions: &[String]) -> Result<Vec<String>> {
    let mut names = Vec::new();
    for permission in permissions {
        let trimmed = permission.trim();
        if trimmed.is_empty() {
            continue;
        }
        if !trimmed
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.')
        {
            bail!("invalid permission name '{permission}'; use letters, digits, '_' and '.'");
        }
        let name = if trimmed.contains('.') && trimmed.starts_with("android.permission.") {
            trimmed.to_owned()
        } else if trimmed.contains('.') {
            // A fully qualified name from another package, for example a vendor permission.
            trimmed.to_owned()
        } else {
            format!("android.permission.{trimmed}")
        };
        if !names.contains(&name) {
            names.push(name);
        }
    }
    if names.is_empty() {
        return Ok(DEFAULT_PERMISSIONS
            .iter()
            .map(|name| (*name).to_owned())
            .collect());
    }
    Ok(names)
}

/// The replacement block, indentation included for the inner lines.
fn block(permissions: &[String]) -> Result<String> {
    let names = qualified(permissions)?;
    let mut block = String::new();
    block.push_str(BEGIN);
    block.push('\n');
    for name in names {
        block.push_str("    <uses-permission android:name=\"");
        block.push_str(&name);
        block.push_str("\" />\n");
    }
    block.push_str("    ");
    block.push_str(END);
    Ok(block)
}

/// Replace the marked block. Returns None when the manifest has no markers, so the caller can
/// report it as skipped instead of rewriting a hand-written manifest.
pub fn merge_permissions(manifest: &str, permissions: &[String]) -> Result<Option<String>> {
    let Some(start) = manifest.find(BEGIN) else {
        return Ok(None);
    };
    let Some(end_offset) = manifest[start..].find(END) else {
        bail!("AndroidManifest.xml has a winia:permissions begin marker but no end marker");
    };
    let end = start + end_offset + END.len();
    let replacement = block(permissions)?;
    let mut merged = String::with_capacity(manifest.len() + replacement.len());
    merged.push_str(&manifest[..start]);
    merged.push_str(&replacement);
    merged.push_str(&manifest[end..]);
    Ok(Some(merged))
}

#[cfg(test)]
mod tests {
    use super::*;

    const MANIFEST: &str = "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<manifest xmlns:android=\"http://schemas.android.com/apk/res/android\">\n    <!-- winia:permissions:begin -->\n    <uses-permission android:name=\"android.permission.INTERNET\" />\n    <!-- winia:permissions:end -->\n    <application android:label=\"Demo\" />\n</manifest>\n";

    #[test]
    fn qualifies_short_names_and_dedupes() {
        let names = qualified(&[
            "INTERNET".into(),
            "android.permission.CAMERA".into(),
            "INTERNET".into(),
        ])
        .unwrap();
        assert_eq!(
            names,
            vec!["android.permission.INTERNET", "android.permission.CAMERA"]
        );
    }

    #[test]
    fn empty_config_keeps_internet() {
        assert_eq!(qualified(&[]).unwrap(), vec!["android.permission.INTERNET"]);
    }

    #[test]
    fn rejects_names_with_spaces() {
        assert!(qualified(&["READ SMS".into()]).is_err());
    }

    #[test]
    fn replaces_only_the_marked_block() {
        let merged = merge_permissions(MANIFEST, &["INTERNET".into(), "CAMERA".into()])
            .unwrap()
            .unwrap();
        assert!(merged.contains("android.permission.CAMERA"));
        assert_eq!(merged.matches("android.permission.INTERNET").count(), 1);
        assert!(merged.contains("<application android:label=\"Demo\" />"));
        assert!(merged.contains("winia:permissions:begin"));
    }

    #[test]
    fn is_idempotent() {
        let first = merge_permissions(MANIFEST, &["CAMERA".into()])
            .unwrap()
            .unwrap();
        let second = merge_permissions(&first, &["CAMERA".into()])
            .unwrap()
            .unwrap();
        assert_eq!(first, second);
    }

    #[test]
    fn reports_a_manifest_without_markers() {
        let plain = "<manifest><application /></manifest>\n";
        assert!(
            merge_permissions(plain, &["CAMERA".into()])
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn reports_a_missing_end_marker() {
        let broken = "<manifest>\n<!-- winia:permissions:begin -->\n</manifest>";
        assert!(merge_permissions(broken, &[]).is_err());
    }
}
