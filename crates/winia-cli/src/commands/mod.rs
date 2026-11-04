//! Command dispatch.
//!
//! Only the project-creation and environment-inspection commands are
//! implemented in this scaffold; every other command fails with an explicit
//! "not implemented yet" message that points at the written contract.

pub mod assets;
pub mod build;
pub mod clean;
pub mod dev;
pub mod doctor;
pub mod inspect;
pub mod new;
pub mod package;
pub mod release;
pub mod run;
pub mod sign;
pub mod test;

use crate::cli::{
    CiArgs, Cli, Command, CompletionShell, ConfigArgs, ConfigCommand, EmulatorsArgs, InfoArgs,
    LogcatArgs, PlatformArgs, PlatformCommand, ScreenshotArgs, TargetsArgs,
};
use crate::config::{WiniaConfig, find_config};
use anyhow::{Context, Result, bail};
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
        Command::Platform(args) => platform(args),
        Command::Assets(args) => assets::run(args),
        Command::Ci(args) => ci(args),
        Command::Emulators(args) => emulators(args),
        Command::Logcat(args) => logcat(args),
        Command::Screenshot(args) => screenshot(args),
        Command::Devices => planned("devices"),
        Command::Android(_) => planned("android"),
        other => planned(command_name(&other)),
    }
}

fn planned(name: &str) -> Result<()> {
    bail!(
        "winia {name} is not implemented in this scaffold; the intended contract is \
         documented in docs/02-feature-spec.md"
    )
}

fn command_name(command: &Command) -> &'static str {
    match command {
        Command::New(_) => "new",
        Command::Init(_) => "init",
        Command::Build(_) => "build",
        Command::Run(_) => "run",
        Command::Dev(_) => "dev",
        Command::Inspect(_) => "inspect",
        Command::Test(_) => "test",
        Command::Package(_) => "package",
        Command::Sign(_) => "sign",
        Command::Release(_) => "release",
        Command::Doctor(_) => "doctor",
        Command::Add(_) => "add",
        Command::Remove(_) => "remove",
        Command::Update(_) => "update",
        Command::Assets(_) => "assets",
        Command::Ci(_) => "ci",
        Command::Upgrade(_) => "upgrade",
        Command::Clean(_) => "clean",
        Command::Info(_) => "info",
        Command::Targets(_) => "targets",
        Command::Config(_) => "config",
        Command::Devices => "devices",
        Command::Emulators(_) => "emulators",
        Command::Logcat(_) => "logcat",
        Command::Screenshot(_) => "screenshot",
        Command::Platform(_) => "platform",
        Command::Android(_) => "android",
        Command::Completions(_) => "completions",
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
        "{:<10} {:<12} requires the Android platform layer",
        "android", "planned"
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
        ConfigCommand::Show => {
            print!("{}", toml::to_string_pretty(&config)?);
        }
        ConfigCommand::Get { key } => {
            let value = toml::Value::try_from(&config)?;
            let mut current = &value;
            for part in key.split('.') {
                current = current
                    .get(part)
                    .ok_or_else(|| anyhow::anyhow!("unknown config key: {key}"))?;
            }
            println!("{current}");
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
    clap_complete::generate(shell, &mut command, "winia", &mut std::io::stdout());
    Ok(())
}

fn platform(args: PlatformArgs) -> Result<()> {
    match args.command {
        PlatformCommand::Add(add) => {
            let (dir, note) = match add.platform {
                crate::cli::Platform::Android => (
                    "android",
                    "Android scaffold placeholder; see docs/03-android-support.md",
                ),
                crate::cli::Platform::Ios => ("ios", "iOS support is not designed yet"),
                other => return planned(&format!("platform add {other:?}")),
            };
            std::fs::create_dir_all(dir)?;
            std::fs::write(
                PathBuf::from(dir).join("README.md"),
                format!("# {dir}\n\n{note}\n"),
            )?;
            println!("created {dir}/README.md");
            Ok(())
        }
    }
}

fn ci(_args: CiArgs) -> Result<()> {
    planned("ci")
}

fn emulators(_args: EmulatorsArgs) -> Result<()> {
    planned("emulators")
}

fn logcat(_args: LogcatArgs) -> Result<()> {
    planned("logcat")
}

fn screenshot(_args: ScreenshotArgs) -> Result<()> {
    planned("screenshot")
}
