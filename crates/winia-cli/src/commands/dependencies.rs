//! 'winia add' / 'remove' / 'update' — project dependency management.
//!
//! Cargo already owns this, and reimplementing resolution here would only drift from it. The
//! commands therefore resolve the project root through winia.toml and delegate to cargo, which
//! keeps the lockfile, feature unification and registry handling exactly as users expect.

use crate::cli::{AddArgs, RemoveArgs, UpdateArgs};
use anyhow::{Context, Result, bail};
use std::path::{Path, PathBuf};
use std::process::Command;

pub fn add(args: AddArgs) -> Result<()> {
    let root = project_root()?;
    let arguments = add_arguments(&args);
    run_cargo(&root, &arguments)
}

fn add_arguments(args: &AddArgs) -> Vec<String> {
    let mut arguments = vec!["add".to_owned(), args.package.clone()];
    if !args.features.is_empty() {
        arguments.push("--features".to_owned());
        arguments.push(args.features.join(","));
    }
    if args.dev {
        arguments.push("--dev".to_owned());
    }
    arguments
}

fn update_arguments(check: bool) -> Vec<String> {
    let mut arguments = vec!["update".to_owned()];
    if check {
        arguments.push("--dry-run".to_owned());
    }
    arguments
}

pub fn remove(args: RemoveArgs) -> Result<()> {
    let root = project_root()?;
    run_cargo(&root, &["remove".to_owned(), args.package.clone()])
}

pub fn update(args: UpdateArgs) -> Result<()> {
    let root = project_root()?;
    let arguments = update_arguments(args.check);
    run_cargo(&root, &arguments)
}

fn project_root() -> Result<PathBuf> {
    let cwd = std::env::current_dir().context("reading the current directory")?;
    let config = crate::config::find_config(&cwd)
        .ok_or_else(|| anyhow::anyhow!("winia.toml not found; run this inside a Winia project"))?;
    Ok(config
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .to_path_buf())
}

/// Stream cargo's output so progress and errors look exactly like running it directly.
fn run_cargo(root: &Path, arguments: &[String]) -> Result<()> {
    if which("cargo").is_none() {
        bail!("cargo was not found on PATH; install the Rust toolchain first");
    }
    let status = Command::new("cargo")
        .args(arguments)
        .current_dir(root)
        .status()
        .context("running cargo")?;
    if !status.success() {
        bail!("cargo {} failed ({status})", arguments.join(" "));
    }
    Ok(())
}

fn which(program: &str) -> Option<PathBuf> {
    let paths = std::env::var_os("PATH")?;
    for directory in std::env::split_paths(&paths) {
        let candidate = directory.join(program);
        if candidate.is_file() {
            return Some(candidate);
        }
        #[cfg(windows)]
        {
            let with_exe = directory.join(format!("{program}.exe"));
            if with_exe.is_file() {
                return Some(with_exe);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn add_arguments_include_features_and_dev() {
        let args = AddArgs {
            package: "serde".to_owned(),
            features: vec!["derive".to_owned(), "rc".to_owned()],
            dev: true,
        };
        let arguments = add_arguments(&args);
        assert_eq!(
            arguments,
            vec!["add", "serde", "--features", "derive,rc", "--dev"]
        );
    }

    #[test]
    fn update_check_uses_a_dry_run() {
        let arguments = update_arguments(true);
        assert_eq!(arguments, vec!["update", "--dry-run"]);
        assert_eq!(update_arguments(false), vec!["update"]);
    }
}
