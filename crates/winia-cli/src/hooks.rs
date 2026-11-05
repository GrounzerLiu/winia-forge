//! Project hooks declared in winia.toml.
//!
//! Hooks run through the platform shell in the project root. Their output is forwarded to stderr so
//! JSON output on stdout stays machine readable, and a failing hook aborts the command it belongs to.

use crate::config::WiniaConfig;
use anyhow::{Context, Result, bail};
use std::path::Path;
use std::process::Command;

/// Which point of a build or package run a hook belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    BeforeBuild,
    AfterBuild,
    BeforePackage,
    AfterPackage,
}

impl Phase {
    fn label(self) -> &'static str {
        match self {
            Self::BeforeBuild => "before_build",
            Self::AfterBuild => "after_build",
            Self::BeforePackage => "before_package",
            Self::AfterPackage => "after_package",
        }
    }

    fn commands(self, config: &WiniaConfig) -> &[String] {
        match self {
            Self::BeforeBuild => &config.hooks.before_build,
            Self::AfterBuild => &config.hooks.after_build,
            Self::BeforePackage => &config.hooks.before_package,
            Self::AfterPackage => &config.hooks.after_package,
        }
    }
}

/// The values exposed to a hook through the environment.
pub struct HookContext<'a> {
    pub target: &'a str,
    pub profile: &'a str,
    pub version: &'a str,
}

/// Run every command registered for one phase, in order.
pub fn run(
    root: &Path,
    config: &WiniaConfig,
    phase: Phase,
    context: &HookContext<'_>,
) -> Result<()> {
    let commands = phase.commands(config);
    if commands.is_empty() {
        return Ok(());
    }
    for command in commands {
        eprintln!("hook {}: {}", phase.label(), command);
        let mut process = shell(command);
        let output = process
            .current_dir(root)
            .env("WINIA_TARGET", context.target)
            .env("WINIA_PROFILE", context.profile)
            .env("WINIA_VERSION", context.version)
            .output()
            .with_context(|| format!("running hook '{}'", command))?;
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        if !stdout.trim().is_empty() {
            eprint!("{stdout}");
        }
        if !stderr.trim().is_empty() {
            eprint!("{stderr}");
        }
        if !output.status.success() {
            bail!(
                "hook {} failed: {} ({})",
                phase.label(),
                command,
                output.status
            );
        }
    }
    Ok(())
}

fn shell(command: &str) -> Command {
    if cfg!(windows) {
        let mut process = Command::new("cmd");
        process.arg("/C").arg(command);
        process
    } else {
        let mut process = Command::new("sh");
        process.arg("-c").arg(command);
        process
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config_with(commands: &[&str]) -> WiniaConfig {
        let mut config = WiniaConfig::default();
        config.hooks.before_build = commands.iter().map(|value| (*value).to_owned()).collect();
        config
    }

    fn context() -> HookContext<'static> {
        HookContext {
            target: "windows",
            profile: "debug",
            version: "0.1.0",
        }
    }

    #[test]
    fn runs_the_registered_commands_in_order() {
        let root = std::env::current_dir().unwrap();
        let config = config_with(&["echo one", "echo two"]);
        run(&root, &config, Phase::BeforeBuild, &context()).unwrap();
    }

    #[test]
    fn a_failing_command_stops_the_phase() {
        let root = std::env::current_dir().unwrap();
        let config = config_with(&["exit 3"]);
        let error = run(&root, &config, Phase::BeforeBuild, &context()).unwrap_err();
        assert!(error.to_string().contains("before_build"));
    }

    #[test]
    fn an_empty_phase_is_a_no_op() {
        let root = std::env::current_dir().unwrap();
        let config = WiniaConfig::default();
        run(&root, &config, Phase::BeforeBuild, &context()).unwrap();
    }
}
