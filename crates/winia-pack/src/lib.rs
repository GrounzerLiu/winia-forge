//! Cross-platform packaging primitives for Winia applications.
//!
//! Implemented here: NSIS and MSI for Windows, and Debian packages assembled as plain ar
//! archives. Zip, .app/.dmg, tar.gz and APK/AAB are produced by winia-cli, and MSIX, PKG, RPM and
//! AppImage are not implemented yet.

pub mod deb;
pub mod msi;
pub mod nsis;

/// Reject names that cannot be used as a Windows folder or shortcut name. Packaging runs the same
/// check for every target so a project fails before a backend produces a broken installer.
pub fn validate_app_name(name: &str) -> anyhow::Result<()> {
    if name.trim().is_empty() {
        anyhow::bail!("app.name must not be empty");
    }
    if name.chars().any(|c| {
        c.is_control() || matches!(c, ':' | '|' | '?' | '*' | '/' | '\\' | '"' | '<' | '>')
    }) {
        anyhow::bail!(
            "app.name '{name}' contains a character that is not valid in a Windows file or folder name"
        );
    }
    if matches!(name, "." | "..") {
        anyhow::bail!("app.name '{name}' is not a valid folder name");
    }
    Ok(())
}

/// Artifact formats the packager intends to support.
pub const PLANNED_FORMATS: &[&str] = &[
    "zip", "nsis", "msi", "msix", "app", "dmg", "pkg", "tar.gz", "deb", "rpm", "appimage", "apk",
    "aab",
];

/// Return the planned artifact formats.
pub fn planned_formats() -> &'static [&'static str] {
    PLANNED_FORMATS
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn android_formats_are_in_the_planned_set() {
        assert!(planned_formats().contains(&"apk"));
        assert!(planned_formats().contains(&"aab"));
    }
}
