//! Command dispatch.
//!
//! Every command in the CLI is implemented; commands that cannot work in the
//! current situation (iOS, release tracks other than GitHub, unsupported targets)
//! explain why instead of failing silently.

pub mod android;
pub mod assets;
pub mod build;
pub mod ci;
pub mod clean;
pub mod dev;
pub mod devices;
pub mod doctor;
pub mod emulators;
pub mod init;
pub mod inspect;
pub mod logcat;
pub mod new;
pub mod package;
pub mod release;
pub mod run;
pub mod screenshot;
pub mod sign;
pub mod sign_macos;
pub mod sign_windows;
pub mod test;

use crate::cli::{Cli, Command, CompletionShell, ConfigArgs, ConfigCommand, InfoArgs, TargetsArgs};
use crate::config::{WiniaConfig, find_config};
use anyhow::{Context, Result};
use clap::CommandFactory;
use std::path::PathBuf;

pub fn dispatch(cli: Cli) -> Result<()> {
    match cli.command {
        Command::New(args) => new::run(args),
        Command::Build(args) => build::run(args),
        Command::Run(args) => run::run(args),
        Command::Dev(args) => dev::run(args),
        Command::Inspect(args) => inspect::run(args),
        Command::Package(args) => package::run(args),
        Command::Release(args) => release::run(args),
        Command::Sign(args) => sign::run(args),
        Command::Test(args) => test::run(args),
        Command::Doctor(args) => doctor::run(args),
        Command::Clean(args) => clean::run(args),
        Command::Info(args) => info(args),
        Command::Targets(args) => targets(args),
        Command::Config(args) => config(args),
        Command::Completions(args) => completions(args),
        Command::Assets(args) => assets::run(args),
        Command::Ci(args) => ci::run(args),
        Command::Init(args) => init::run(args),
        Command::Emulators(args) => emulators::run(args),
        Command::Logcat(args) => logcat::run(args),
        Command::Screenshot(args) => screenshot::run(args),
        Command::Devices(args) => devices::run(args),
        Command::Android(args) => android::run(args),
    }
}

fn info(args: InfoArgs) -> Result<()> {
    let cwd = std::env::current_dir().context("reading the current directory")?;
    let config = find_config(&cwd);
    if args.json {
        let value = serde_json::json!({
            "package": "winia-cli",
            "version": env!("CARGO_PKG_VERSION"),
            "os": std::env::consts::OS,
            "arch": std::env::consts::ARCH,
            "cwd": cwd,
            "config": config.map(|path| path.display().to_string()),
        });
        println!("{}", serde_json::to_string_pretty(&value)?);
        return Ok(());
    }

    println!("Winia Forge");
    println!("  version : {}", env!("CARGO_PKG_VERSION"));
    println!("  os      : {}", std::env::consts::OS);
    println!("  arch    : {}", std::env::consts::ARCH);
    println!("  cwd     : {}", cwd.display());
    match config {
        Some(path) => println!("  config  : {}", path.display()),
        None => println!("  config  : not found (run 'winia new <name>' first)"),
    }
    Ok(())
}

fn targets(_args: TargetsArgs) -> Result<()> {
    println!("{:<10} {:<12} NOTES", "TARGET", "STATUS");
    println!("{:<10} {:<12} x86_64, aarch64", "windows", "supported");
    println!(
        "{:<10} {:<12} x86_64, aarch64, universal",
        "macos", "supported"
    );
    println!("{:<10} {:<12} x86_64, aarch64", "linux", "supported");
    println!(
        "{:<10} {:<12} aarch64, x86_64, armeabi-v7a (APK/AAB)",
        "android", "supported"
    );
    println!("{:<10} {:<12} not designed yet", "ios", "planned");
    Ok(())
}

fn config(args: ConfigArgs) -> Result<()> {
    let path = args
        .file
        .or_else(|| find_config(&std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))))
        .ok_or_else(|| anyhow::anyhow!("winia.toml not found"))?;
    let config = WiniaConfig::load(&path)?;
    match args.command {
        ConfigCommand::Show { json: true } => {
            println!("{}", serde_json::to_string_pretty(&config)?);
        }
        ConfigCommand::Show { json: false } => {
            print!("{}", toml::to_string_pretty(&config)?);
        }
        ConfigCommand::Get { key, json: true } => {
            let value = toml::Value::try_from(&config)?;
            let mut current = &value;
            for part in key.split('.') {
                current = current
                    .get(part)
                    .ok_or_else(|| anyhow::anyhow!("unknown config key: {key}"))?;
            }
            println!(
                "{}",
                serde_json::to_string_pretty(&serde_json::to_value(current)?)?
            );
        }
        ConfigCommand::Get { key, json: false } => {
            let value = toml::Value::try_from(&config)?;
            let mut current = &value;
            for part in key.split('.') {
                match current.get(part) {
                    Some(next) => current = next,
                    // Optional keys vanish from the serialised config when unset; saying "unknown"
                    // would be indistinguishable from a typo.
                    None if matches!(
                        key.as_str(),
                        "app.icon" | "app.license" | "android.label"
                    ) =>
                    {
                        println!();
                        return Ok(());
                    }
                    None => anyhow::bail!("unknown config key: {key}"),
                }
            }
            // Print scalars without TOML quotes so shell users can consume them directly.
            match current {
                toml::Value::String(text) => println!("{text}"),
                other => println!("{other}"),
            }
        }
    }
    Ok(())
}

fn completions(args: crate::cli::CompletionsArgs) -> Result<()> {
    use clap_complete::Shell;
    let shell = match args.shell {
        CompletionShell::Bash => Shell::Bash,
        CompletionShell::Zsh => Shell::Zsh,
        CompletionShell::Fish => Shell::Fish,
        CompletionShell::Powershell => Shell::PowerShell,
        CompletionShell::Elvish => Shell::Elvish,
    };
    let mut command = crate::cli::Cli::command();
    // Render first: piping into head closes stdout early, and clap_complete panics on that write.
    let mut buffer = Vec::new();
    clap_complete::generate(shell, &mut command, "winia", &mut buffer);
    use std::io::Write;
    match std::io::stdout().write_all(&buffer) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::BrokenPipe => Ok(()),
        Err(error) => Err(error).context("writing completions"),
    }
}
