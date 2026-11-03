//! Cross-platform packaging primitives for Winia applications.
//!
//! The planned modules are:
//! - windows: zip, NSIS, MSI (WiX), MSIX;
//! - macos: .app, .dmg, .pkg;
//! - linux: .tar.gz, .deb, .rpm, .AppImage;
//! - android: APK and AAB.
//!
//! The crate is a placeholder so winia-cli can depend on a stable boundary
//! while the packaging work lands.

/// Artifact formats the packager intends to support.
pub const PLANNED_FORMATS: &[&str] = &[
    "zip", "nsis", "msi", "msix", "app", "dmg", "pkg", "deb", "rpm", "appimage", "apk", "aab",
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
