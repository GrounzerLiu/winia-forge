//! Android ABI names and their Rust target triples.
//!
//! One place owns the mapping so `config`, `doctor`, `build`, and `package`
//! cannot drift apart.

use anyhow::{Result, bail};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Abi {
    Arm64V8a,
    ArmeabiV7a,
    X86_64,
    X86,
}

impl Abi {
    pub fn parse(raw: &str) -> Result<Self> {
        Ok(match raw {
            "arm64-v8a" => Self::Arm64V8a,
            "armeabi-v7a" => Self::ArmeabiV7a,
            "x86_64" => Self::X86_64,
            "x86" => Self::X86,
            other => bail!("unknown Android ABI: {other}"),
        })
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Arm64V8a => "arm64-v8a",
            Self::ArmeabiV7a => "armeabi-v7a",
            Self::X86_64 => "x86_64",
            Self::X86 => "x86",
        }
    }

    pub fn rust_triple(self) -> &'static str {
        match self {
            Self::Arm64V8a => "aarch64-linux-android",
            Self::ArmeabiV7a => "armv7-linux-androideabi",
            Self::X86_64 => "x86_64-linux-android",
            Self::X86 => "i686-linux-android",
        }
    }

    pub fn jni_libs_dir(self) -> &'static str {
        self.as_str()
    }

    pub fn is_legacy_32bit(self) -> bool {
        matches!(self, Self::ArmeabiV7a | Self::X86)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_round_trips_every_abi() {
        for (name, triple) in [
            ("arm64-v8a", "aarch64-linux-android"),
            ("armeabi-v7a", "armv7-linux-androideabi"),
            ("x86_64", "x86_64-linux-android"),
            ("x86", "i686-linux-android"),
        ] {
            let abi = Abi::parse(name).expect("known ABI");
            assert_eq!(abi.as_str(), name);
            assert_eq!(abi.rust_triple(), triple);
            assert_eq!(abi.jni_libs_dir(), name);
        }
    }

    #[test]
    fn rejects_unknown_abis() {
        assert!(Abi::parse("mips").is_err());
    }

    #[test]
    fn only_the_two_old_abis_are_legacy() {
        assert!(!Abi::Arm64V8a.is_legacy_32bit());
        assert!(!Abi::X86_64.is_legacy_32bit());
        assert!(Abi::ArmeabiV7a.is_legacy_32bit());
        assert!(Abi::X86.is_legacy_32bit());
    }
}
