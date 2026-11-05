//! Guard the command surface: every command must be listed and described.

use clap::CommandFactory;

const COMMANDS: &[&str] = &[
    "new",
    "init",
    "build",
    "run",
    "dev",
    "inspect",
    "test",
    "package",
    "sign",
    "release",
    "doctor",
    "add",
    "remove",
    "update",
    "assets",
    "ci",
    "clean",
    "info",
    "targets",
    "config",
    "devices",
    "emulators",
    "logcat",
    "screenshot",
    "platform",
    "android",
    "completions",
];

#[test]
fn help_lists_every_command() {
    let mut command = winia_cli::cli::Cli::command();
    let help = command.render_help().to_string();
    for name in COMMANDS {
        assert!(help.contains(name), "help is missing the '{name}' command");
    }
    command.build();
    let names: Vec<String> = command
        .get_subcommands()
        .map(|sub| sub.get_name().to_owned())
        .collect();
    for name in COMMANDS {
        assert!(
            names.iter().any(|listed| listed == name),
            "the '{name}' command is not registered"
        );
    }
    assert!(
        !names.iter().any(|listed| listed == "upgrade"),
        "upgrade was removed on purpose"
    );
}

#[test]
fn every_command_has_a_description() {
    let mut command = winia_cli::cli::Cli::command();
    command.build();
    for sub in command.get_subcommands() {
        let about = sub
            .get_about()
            .map(|about| about.to_string())
            .unwrap_or_default();
        assert!(
            !about.trim().is_empty(),
            "'{}' has no description in --help",
            sub.get_name()
        );
    }
}
