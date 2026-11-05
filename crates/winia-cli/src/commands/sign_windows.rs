//! Windows Authenticode signing through signtool.
//!
//! Credentials come from the environment: either a PFX file plus its password, or the SHA-1
//! thumbprint of a certificate in the current user's store. Without credentials the command fails
//! with the exact variables to set, so CI logs say what is missing.
//!
//! Signing happens in place: the artifact set produced by packaging is what gets published, and a
//! second copy would only invite the release step to upload the unsigned one.

use crate::cli::SignArgs;
use anyhow::{Context, Result, bail};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

const DEFAULT_TIMESTAMP: &str = "http://timestamp.digicert.com";

/// Sign every desktop artifact in the project with signtool.
pub fn run(root: &Path, cwd: &Path, args: &SignArgs) -> Result<()> {
    let started = Instant::now();
    let signtool = find_signtool()?;
    let credentials = Credentials::from_env(args.keystore.as_deref(), cwd)?;
    let files = resolve_files(root, cwd, args)?;
    if files.is_empty() {
        bail!(
            "no Windows artifacts to sign; run 'winia package --target windows' first or pass --file"
        );
    }

    if args.dry_run {
        for file in &files {
            println!("Would sign {}", relative(root, file));
            println!("  {}", render_command(&signtool, &credentials, file));
        }
        return Ok(());
    }

    let mut signed = Vec::new();
    for file in &files {
        let output = Command::new(&signtool)
            .args(credentials.arguments())
            .arg(file)
            .output()
            .with_context(|| format!("running {}", signtool.display()))?;
        if !output.status.success() {
            bail!(
                "signtool failed for {}: {}{}",
                file.display(),
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
        }
        let verified = verify(&signtool, file);
        if !verified {
            eprintln!(
                "note: {} is signed but the chain does not verify locally (self-signed or missing root)",
                relative(root, file)
            );
        }
        signed.push(file.clone());
    }

    if args.json {
        let values: Vec<_> = signed
            .iter()
            .map(|path| {
                serde_json::json!({
                    "path": relative(root, path),
                    "bytes": std::fs::metadata(path).map(|meta| meta.len()).unwrap_or(0),
                })
            })
            .collect();
        let value = serde_json::json!({
            "target": "windows",
            "artifacts": values,
            "duration_ms": started.elapsed().as_millis(),
        });
        println!("{}", serde_json::to_string_pretty(&value)?);
        return Ok(());
    }

    for file in &signed {
        println!("Signed {}", relative(root, file));
    }
    println!(
        "Signed {} artifact(s) in {:.1}s",
        signed.len(),
        started.elapsed().as_secs_f64()
    );
    Ok(())
}

struct Credentials {
    /// PFX path plus password, or a store thumbprint.
    source: Source,
    timestamp: String,
}

enum Source {
    Pfx { path: PathBuf, password: String },
    Thumbprint(String),
}

impl Credentials {
    fn from_env(keystore: Option<&Path>, cwd: &Path) -> Result<Self> {
        let timestamp = std::env::var("WINIA_WINDOWS_TIMESTAMP")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| DEFAULT_TIMESTAMP.to_owned());
        if let Some(thumbprint) = std::env::var("WINIA_WINDOWS_CERT_SHA1")
            .ok()
            .map(|value| value.trim().replace(' ', ""))
            .filter(|value| !value.is_empty())
        {
            return Ok(Self {
                source: Source::Thumbprint(thumbprint),
                timestamp,
            });
        }
        let pfx = std::env::var_os("WINIA_WINDOWS_CERT")
            .map(PathBuf::from)
            .or_else(|| keystore.map(|path| absolute(cwd, path)));
        let Some(pfx) = pfx else {
            bail!(
                "no Windows certificate configured; set WINIA_WINDOWS_CERT to a PFX (with WINIA_WINDOWS_CERT_PASSWORD) or WINIA_WINDOWS_CERT_SHA1 to a store thumbprint"
            );
        };
        if !pfx.is_file() {
            bail!("certificate not found: {}", pfx.display());
        }
        let password = std::env::var("WINIA_WINDOWS_CERT_PASSWORD").unwrap_or_default();
        Ok(Self {
            source: Source::Pfx {
                path: pfx,
                password,
            },
            timestamp,
        })
    }

    fn arguments(&self) -> Vec<String> {
        let mut arguments: Vec<String> =
            vec!["sign".to_owned(), "/fd".to_owned(), "sha256".to_owned()];
        match &self.source {
            Source::Pfx { path, password } => {
                arguments.push("/f".to_owned());
                arguments.push(path.display().to_string());
                if !password.is_empty() {
                    arguments.push("/p".to_owned());
                    arguments.push(password.clone());
                }
            }
            Source::Thumbprint(thumbprint) => {
                arguments.push("/sha1".to_owned());
                arguments.push(thumbprint.clone());
            }
        }
        arguments.push("/tr".to_owned());
        arguments.push(self.timestamp.clone());
        arguments.push("/td".to_owned());
        arguments.push("sha256".to_owned());
        arguments
    }
}

fn render_command(signtool: &Path, credentials: &Credentials, file: &Path) -> String {
    let mut parts = vec![signtool.display().to_string()];
    for argument in credentials.arguments() {
        // Quote only values that would otherwise split: switches, tokens and URLs stay as they are.
        if argument.contains(' ') || argument.contains('\\') {
            parts.push(format!("\"{argument}\""));
        } else {
            parts.push(argument);
        }
    }
    parts.push(format!("\"{}\"", file.display()));
    parts.join(" ")
}

fn verify(signtool: &Path, file: &Path) -> bool {
    Command::new(signtool)
        .arg("verify")
        .arg("/pa")
        .arg("/q")
        .arg(file)
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

/// Artifacts to sign: explicit --file entries, then --apk, otherwise every desktop package in dist.
fn resolve_files(root: &Path, cwd: &Path, args: &SignArgs) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    for path in &args.file {
        let path = absolute(cwd, path);
        if !path.is_file() {
            bail!("{} does not exist", path.display());
        }
        files.push(path);
    }
    if let Some(apk) = &args.apk {
        let path = absolute(cwd, apk);
        if !path.is_file() {
            bail!("{} does not exist", path.display());
        }
        files.push(path);
    }
    if files.is_empty() {
        let dist = root.join("dist");
        if !dist.is_dir() {
            return Ok(files);
        }
        for entry in
            std::fs::read_dir(&dist).with_context(|| format!("reading {}", dist.display()))?
        {
            let path = entry?.path();
            let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
                continue;
            };
            let lower = name.to_ascii_lowercase();
            if lower.ends_with(".exe") || lower.ends_with(".msi") || lower.ends_with(".dll") {
                files.push(path);
            }
        }
        files.sort();
    }
    files.dedup();
    Ok(files)
}

/// Locate signtool: explicit override, PATH, then the newest Windows SDK.
pub fn find_signtool() -> Result<PathBuf> {
    if let Some(path) = std::env::var_os("WINIA_SIGNTOOL") {
        let path = PathBuf::from(path);
        if path.is_file() {
            return Ok(path);
        }
    }
    if let Some(path) = std::env::var_os("PATH") {
        for directory in std::env::split_paths(&path) {
            let candidate = directory.join("signtool.exe");
            if candidate.is_file() {
                return Ok(candidate);
            }
        }
    }
    if cfg!(windows) {
        if let Some(kits) = std::env::var_os("ProgramFiles(x86)") {
            let base = PathBuf::from(kits).join("Windows Kits/10/bin");
            if let Ok(entries) = std::fs::read_dir(&base) {
                let mut versions: Vec<PathBuf> =
                    entries.flatten().map(|entry| entry.path()).collect();
                versions.sort();
                versions.reverse();
                for version in versions {
                    for arch in ["x64", "x86"] {
                        let candidate = version.join(arch).join("signtool.exe");
                        if candidate.is_file() {
                            return Ok(candidate);
                        }
                    }
                }
            }
        }
    }
    bail!("signtool not found; install the Windows SDK or set WINIA_SIGNTOOL")
}

fn absolute(cwd: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        cwd.join(path)
    }
}

fn relative(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .display()
        .to_string()
        .replace('\\', "/")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pfx_arguments_carry_the_certificate_and_timestamp() {
        let credentials = Credentials {
            source: Source::Pfx {
                path: PathBuf::from("cert.pfx"),
                password: "secret".to_owned(),
            },
            timestamp: DEFAULT_TIMESTAMP.to_owned(),
        };
        let arguments = credentials.arguments();
        assert_eq!(arguments[0], "sign");
        assert!(arguments.contains(&"/f".to_owned()));
        assert!(arguments.contains(&"cert.pfx".to_owned()));
        assert!(arguments.contains(&"/p".to_owned()));
        assert!(arguments.contains(&"/td".to_owned()));
    }

    #[test]
    fn thumbprint_arguments_skip_the_file() {
        let credentials = Credentials {
            source: Source::Thumbprint("ABC123".to_owned()),
            timestamp: DEFAULT_TIMESTAMP.to_owned(),
        };
        let arguments = credentials.arguments();
        assert!(arguments.contains(&"/sha1".to_owned()));
        assert!(!arguments.contains(&"/f".to_owned()));
    }

    #[test]
    fn resolves_artifacts_from_dist() {
        let root = std::env::current_dir()
            .unwrap()
            .join("target")
            .join(format!("sign-{}", std::process::id()));
        let dist = root.join("dist");
        std::fs::create_dir_all(&dist).unwrap();
        std::fs::write(dist.join("app-setup.exe"), b"exe").unwrap();
        std::fs::write(dist.join("app.msi"), b"msi").unwrap();
        std::fs::write(dist.join("app.zip"), b"zip").unwrap();
        let args = SignArgs {
            target: vec!["windows".to_owned()],
            keystore: None,
            apk: None,
            file: Vec::new(),
            alias: None,
            in_place: false,
            dry_run: true,
            release: false,
            json: false,
        };
        let files = resolve_files(&root, &root, &args).unwrap();
        assert_eq!(files.len(), 2);
        std::fs::remove_dir_all(root).unwrap();
    }
}
