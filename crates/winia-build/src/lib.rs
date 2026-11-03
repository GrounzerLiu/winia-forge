//! Build-script helpers for Winia applications.
//!
//! This crate is the future home of the platform resource generation used by
//! winia build: Windows .rc resources, macOS Info.plist and .icns, Linux
//! .desktop and icon staging, and Android manifest metadata.
//!
//! The crate exists now so the CLI's dependency graph matches the documented
//! architecture; the generators are not implemented yet.

/// Names of the platform resource generators planned for this crate.
pub const PLANNED_GENERATORS: &[&str] = &[
    "windows-resource",
    "macos-bundle",
    "linux-desktop",
    "android-manifest",
];

/// Return the planned generator names.
pub fn planned_generators() -> &'static [&'static str] {
    PLANNED_GENERATORS
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_planned_generator_list_is_stable() {
        assert!(planned_generators().contains(&"windows-resource"));
        assert!(planned_generators().contains(&"android-manifest"));
    }
}
