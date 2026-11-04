//! Winia Forge — the Winia project toolchain.
//!
//! The crate is split into a small library (for tests) and a thin binary named
//! `winia`. See `docs/02-feature-spec.md` for the product contract.

pub mod assets;
pub mod cli;
pub mod commands;
pub mod config;
pub mod icons;
pub mod platform;

use clap::Parser;

/// Parse the process arguments and dispatch to a command.
pub fn run() -> anyhow::Result<()> {
    commands::dispatch(cli::Cli::parse())
}

/// Parse an explicit argument list. Used by integration tests.
pub fn run_from<I, T>(args: I) -> anyhow::Result<()>
where
    I: IntoIterator<Item = T>,
    T: Into<std::ffi::OsString> + Clone,
{
    commands::dispatch(cli::Cli::parse_from(args))
}
