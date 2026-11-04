//! Command-line surface for Winia Forge.
//!
//! Naming rules:
//! - a platform is a `--target`, never part of the command name;
//! - an artifact is a `--format`;
//! - device operations stay at the top level (`devices`, `logcat`).

use clap::{Args, Parser, Subcommand, ValueEnum};
use std::path::PathBuf;

#[derive(Debug, Parser)]
#[command(
    name = "winia",
    version,
    about = "Winia Forge — create, build, run, and package Winia applications",
    long_about = None
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Create a new Winia application from a template.
    New(NewArgs),
    /// Add Winia support to an existing Rust project.
    Init(InitArgs),
    /// Build the application for one or more targets.
    Build(BuildArgs),
    /// Build and run the application.
    Run(RunArgs),
    /// Run in development mode with file watching and automatic restart.
    Dev(DevArgs),
    /// Inspect a running application through its debug server.
    Inspect(InspectArgs),
    /// Run unit, UI, and device smoke tests.
    Test(TestArgs),
    /// Package the application for distribution.
    Package(PackageArgs),
    /// Sign packaged artifacts.
    Sign(SignArgs),
    /// Bump, tag, build, and publish a release.
    Release(ReleaseArgs),
    /// Diagnose the toolchain and platform SDKs.
    Doctor(DoctorArgs),
    /// Add a dependency to the project.
    Add(AddArgs),
    /// Remove a dependency from the project.
    Remove(RemoveArgs),
    /// Update Winia and project dependencies.
    Update(UpdateArgs),
    /// Generate and manage application assets.
    Assets(AssetsArgs),
    /// Generate CI configuration.
    Ci(CiArgs),
    /// Upgrade the project to a newer Winia release.
    Upgrade(UpgradeArgs),
    /// Remove build artifacts.
    Clean(CleanArgs),
    /// Print environment and project information.
    Info(InfoArgs),
    /// List supported and installed targets.
    Targets(TargetsArgs),
    /// Read or inspect winia.toml.
    Config(ConfigArgs),
    /// List connected Android devices.
    Devices,
    /// Manage Android emulators.
    Emulators(EmulatorsArgs),
    /// Stream Android logcat output.
    Logcat(LogcatArgs),
    /// Capture a screenshot from a running application.
    Screenshot(ScreenshotArgs),
    /// Add or manage platform support.
    Platform(PlatformArgs),
    /// Android-specific project operations.
    Android(AndroidArgs),
    /// Generate shell completions.
    Completions(CompletionsArgs),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Platform {
    Windows,
    Macos,
    Linux,
    Android,
    Ios,
}

#[derive(Debug, Args)]
pub struct NewArgs {
    /// Application name; also the default directory name.
    pub name: String,
    /// Directory to create. Defaults to ./<name>.
    #[arg(long)]
    pub path: Option<PathBuf>,
    /// Target platforms, comma-separated. Defaults to windows,macos,linux.
    #[arg(long, value_delimiter = ',')]
    pub platforms: Vec<Platform>,
    /// Template name or a git URL once remote templates are supported.
    #[arg(long, default_value = "blank")]
    pub template: String,
    /// Path to a local winia checkout. Defaults to WINIA_PATH, then crates.io.
    #[arg(long)]
    pub winia_path: Option<PathBuf>,
    /// Do not initialize a git repository.
    #[arg(long)]
    pub no_git: bool,
}

#[derive(Debug, Args)]
pub struct InitArgs {
    /// Existing project directory. Defaults to the current directory.
    #[arg(long)]
    pub path: Option<PathBuf>,
    /// Target platforms to add, comma-separated.
    #[arg(long, value_delimiter = ',')]
    pub platforms: Vec<Platform>,
}

#[derive(Debug, Args)]
pub struct BuildArgs {
    /// Target platform(s), comma-separated: windows, macos, linux, android.
    #[arg(long, value_delimiter = ',')]
    pub target: Vec<String>,
    /// Build with the release profile.
    #[arg(long)]
    pub release: bool,
    /// Android ABI(s), comma-separated, e.g. arm64-v8a,x86_64.
    #[arg(long, value_delimiter = ',')]
    pub abi: Vec<String>,
    /// Cargo features to enable.
    #[arg(long, value_delimiter = ',')]
    pub features: Vec<String>,
    /// Emit machine-readable output.
    #[arg(long)]
    pub json: bool,
    /// Output directory for build artifacts.
    #[arg(long)]
    pub out_dir: Option<PathBuf>,
}

#[derive(Debug, Args)]
pub struct RunArgs {
    /// Target platform. Defaults to the current host.
    #[arg(long)]
    pub target: Option<String>,
    /// Device serial or emulator name (Android).
    #[arg(long)]
    pub device: Option<String>,
    /// Run the release build.
    #[arg(long)]
    pub release: bool,
    /// Cargo features to enable.
    #[arg(long, value_delimiter = ',')]
    pub features: Vec<String>,
    /// Arguments passed through to the application after `--`.
    #[arg(last = true)]
    pub args: Vec<String>,
}

#[derive(Debug, Args)]
pub struct DevArgs {
    /// Target platform. Defaults to the current host.
    #[arg(long)]
    pub target: Option<String>,
    /// Android device serial. Defaults to the only connected device.
    #[arg(long)]
    pub device: Option<String>,
    /// Build with the release profile.
    #[arg(long)]
    pub release: bool,
    /// Extra cargo features, comma-separated.
    #[arg(long, value_delimiter = ',')]
    pub features: Vec<String>,
    /// Debounce interval in milliseconds.
    #[arg(long, default_value_t = 300)]
    pub debounce: u64,
    /// Disable the automatic debug-server feature and port setup.
    #[arg(long)]
    pub no_debug_server: bool,
    /// Debug-server starting port; Android uses it as the host-side forward port.
    #[arg(long)]
    pub debug_port: Option<u16>,
    /// Build and run once, then exit (desktop waits for the app to close).
    #[arg(long)]
    pub once: bool,
    /// Arguments passed through to the application after `--`.
    #[arg(last = true)]
    pub args: Vec<String>,
}

#[derive(Debug, Args)]
pub struct InspectArgs {
    /// Debug-server port on the host. Defaults to 9998.
    #[arg(long, global = true, default_value_t = 9998)]
    pub port: u16,
    /// Emit machine-readable output.
    #[arg(long, global = true)]
    pub json: bool,
    /// Discovery timeout in milliseconds.
    #[arg(long, global = true, default_value_t = 500)]
    pub timeout: u64,
    #[command(subcommand)]
    pub command: Option<InspectCommand>,
}

#[derive(Debug, Subcommand)]
pub enum InspectCommand {
    /// Print application identity.
    Info,
    /// Print the layout tree.
    Tree,
    /// Click at logical coordinates.
    Click {
        /// Logical x coordinate.
        x: f32,
        /// Logical y coordinate.
        y: f32,
    },
    /// Send a key event, e.g. `key Return`.
    Key {
        /// Key names passed to the debug server.
        keys: Vec<String>,
    },
    /// Send a wheel event: `scroll <dy>` or `scroll <dx> <dy>`.
    Scroll {
        /// One value for vertical scroll, two for horizontal then vertical.
        values: Vec<f32>,
    },
    /// Drag from one logical point to another.
    Drag {
        /// Start x.
        x1: f32,
        /// Start y.
        y1: f32,
        /// End x.
        x2: f32,
        /// End y.
        y2: f32,
        /// Number of pointer-move steps.
        #[arg(long, default_value_t = 10)]
        steps: u32,
        /// Delay between pointer-move steps, in milliseconds.
        #[arg(long, default_value_t = 16)]
        delay_ms: u64,
    },
    /// Capture a frame and save it as PNG.
    Screenshot {
        /// Output path. Defaults to inspect-<port>.png.
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// Scan ports and print the applications found.
    Discover {
        /// Last port to scan (inclusive). Defaults to start + 100.
        #[arg(long)]
        end: Option<u16>,
        /// Scan the port range instead of reading the dev registry.
        #[arg(long)]
        scan: bool,
    },
}

#[derive(Debug, Args)]
pub struct TestArgs {
    /// Target platform. Defaults to the host test suite.
    #[arg(long)]
    pub target: Option<String>,
    /// Test name filter.
    #[arg(long)]
    pub filter: Option<String>,
    /// Android device serial for --target android.
    #[arg(long)]
    pub device: Option<String>,
    /// Use the release profile.
    #[arg(long)]
    pub release: bool,
    /// Extra cargo features, comma-separated.
    #[arg(long, value_delimiter = ',')]
    pub features: Vec<String>,
    /// Update screenshot snapshots.
    #[arg(long)]
    pub update_snapshots: bool,
    /// Emit machine-readable output.
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct PackageArgs {
    /// Target platform(s), comma-separated.
    #[arg(long, value_delimiter = ',')]
    pub target: Vec<String>,
    /// Artifact format(s), comma-separated: zip, nsis, msi, msix, app, dmg, pkg, deb, rpm, appimage, apk, aab.
    #[arg(long, value_delimiter = ',')]
    pub format: Vec<String>,
    /// Package the release build.
    #[arg(long)]
    pub release: bool,
    /// Output directory. Defaults to ./dist.
    #[arg(long)]
    pub out_dir: Option<PathBuf>,
    /// Android split mode.
    #[arg(long, value_enum)]
    pub split: Option<SplitMode>,
    /// Emit machine-readable output.
    #[arg(long)]
    pub json: bool,
    /// Print what would be produced without building.
    #[arg(long)]
    pub dry_run: bool,
    /// Remove existing files in the output directory before packaging.
    #[arg(long)]
    pub clean: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum SplitMode {
    Universal,
    PerAbi,
}

#[derive(Debug, Args)]
pub struct SignArgs {
    /// Target platform(s), comma-separated.
    #[arg(long, value_delimiter = ',')]
    pub target: Vec<String>,
    /// Signing material (keystore on Android, certificate on Windows/macOS).
    #[arg(long)]
    pub keystore: Option<PathBuf>,
    /// APK to sign. Defaults to every APK in dist/.
    #[arg(long)]
    pub apk: Option<PathBuf>,
    /// Key alias. Defaults to WINIA_ANDROID_KEY_ALIAS.
    #[arg(long)]
    pub alias: Option<String>,
    /// Overwrite the input APK instead of writing a *-signed.apk copy.
    #[arg(long)]
    pub in_place: bool,
    /// Print what would be signed without invoking apksigner.
    #[arg(long)]
    pub dry_run: bool,
    /// Only sign APKs that are not debuggable.
    #[arg(long)]
    pub release: bool,
    /// Emit machine-readable output.
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct ReleaseArgs {
    /// Target platform(s), comma-separated.
    #[arg(long, value_delimiter = ',')]
    pub target: Vec<String>,
    /// Release track: github (Android artifacts), or omit for local artifacts only.
    #[arg(long)]
    pub track: Option<String>,
    /// Version bump before releasing.
    #[arg(long, value_enum)]
    pub bump: Option<Bump>,
    /// Allow a dirty git worktree.
    #[arg(long)]
    pub allow_dirty: bool,
    /// Do not create a git tag.
    #[arg(long)]
    pub no_tag: bool,
    /// Push the release commit and tag after building.
    #[arg(long)]
    pub push: bool,
    /// Print the plan without modifying the repository or building.
    #[arg(long)]
    pub dry_run: bool,
    /// Emit machine-readable output.
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Bump {
    Major,
    Minor,
    Patch,
}

#[derive(Debug, Args)]
pub struct DoctorArgs {
    /// Focus the report on one target (windows, macos, linux, android).
    #[arg(long)]
    pub target: Option<String>,
    /// Apply safe automatic fixes and print the commands for manual steps.
    #[arg(long)]
    pub fix: bool,
    /// Emit machine-readable output.
    #[arg(long)]
    pub json: bool,
    /// Treat optional warnings as failures (useful in CI).
    #[arg(long)]
    pub strict: bool,
}

#[derive(Debug, Args)]
pub struct AddArgs {
    /// Crate name to add.
    pub package: String,
    /// Feature list, comma-separated.
    #[arg(long, value_delimiter = ',')]
    pub features: Vec<String>,
    /// Add as a development dependency.
    #[arg(long)]
    pub dev: bool,
}

#[derive(Debug, Args)]
pub struct RemoveArgs {
    /// Crate name to remove.
    pub package: String,
}

#[derive(Debug, Args)]
pub struct UpdateArgs {
    /// Check for updates without applying them.
    #[arg(long)]
    pub check: bool,
}

#[derive(Debug, Args)]
pub struct AssetsArgs {
    #[command(subcommand)]
    pub command: Option<AssetsCommand>,
}

#[derive(Debug, Subcommand)]
pub enum AssetsCommand {
    /// Generate platform icon sets from one source image.
    Icon(IconArgs),
}

#[derive(Debug, Args)]
pub struct IconArgs {
    /// Source PNG or SVG, preferably 1024x1024.
    pub source: PathBuf,
}

#[derive(Debug, Args)]
pub struct CiArgs {
    #[command(subcommand)]
    pub command: Option<CiCommand>,
}

#[derive(Debug, Subcommand)]
pub enum CiCommand {
    /// Write a CI workflow for the requested providers and platforms.
    Init(CiInitArgs),
}

#[derive(Debug, Args)]
pub struct CiInitArgs {
    /// CI provider: github, gitlab.
    #[arg(long, default_value = "github")]
    pub provider: String,
    /// Target platforms for the matrix.
    #[arg(long, value_delimiter = ',')]
    pub platforms: Vec<String>,
}

#[derive(Debug, Args)]
pub struct UpgradeArgs {
    /// Only check whether a newer Winia is available.
    #[arg(long)]
    pub check: bool,
}

#[derive(Debug, Args)]
pub struct CleanArgs {
    /// Also remove project-local Gradle caches.
    #[arg(long)]
    pub all: bool,
    /// Print what would be removed without deleting.
    #[arg(long)]
    pub dry_run: bool,
    /// Emit machine-readable output.
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct InfoArgs {
    /// Emit machine-readable output.
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct TargetsArgs {}

#[derive(Debug, Args)]
pub struct ConfigArgs {
    /// Path to winia.toml. Defaults to ./winia.toml.
    #[arg(long)]
    pub file: Option<PathBuf>,
    #[command(subcommand)]
    pub command: ConfigCommand,
}

#[derive(Debug, Subcommand)]
pub enum ConfigCommand {
    /// Print the effective configuration.
    Show,
    /// Print one key, using dot notation (for example: android.min_sdk).
    Get { key: String },
}

#[derive(Debug, Args)]
pub struct EmulatorsArgs {
    #[command(subcommand)]
    pub command: EmulatorsCommand,
}

#[derive(Debug, Subcommand)]
pub enum EmulatorsCommand {
    List,
    Start { name: String },
    Stop { name: String },
}

#[derive(Debug, Args)]
pub struct LogcatArgs {
    /// Filter by package name or tag.
    #[arg(long)]
    pub filter: Option<String>,
    /// Filter by process id.
    #[arg(long)]
    pub pid: Option<u32>,
    /// Clear the log buffer before streaming.
    #[arg(long)]
    pub clear: bool,
}

#[derive(Debug, Args)]
pub struct ScreenshotArgs {
    /// Target platform. Defaults to android.
    #[arg(long)]
    pub target: Option<String>,
    /// Output file. Defaults to screenshot.png.
    #[arg(long)]
    pub output: Option<PathBuf>,
}

#[derive(Debug, Args)]
pub struct PlatformArgs {
    #[command(subcommand)]
    pub command: PlatformCommand,
}

#[derive(Debug, Subcommand)]
pub enum PlatformCommand {
    /// Add a platform scaffold to an existing project.
    Add(PlatformAddArgs),
}

#[derive(Debug, Args)]
pub struct PlatformAddArgs {
    pub platform: Platform,
}

#[derive(Debug, Args)]
pub struct AndroidArgs {
    #[command(subcommand)]
    pub command: AndroidCommand,
}

#[derive(Debug, Subcommand)]
pub enum AndroidCommand {
    /// Generate the Android project for an existing Winia app.
    Init,
    /// Regenerate managed Android files from winia.toml.
    Sync,
    /// Open the generated android/ directory in Android Studio.
    Studio,
}

#[derive(Debug, Args)]
pub struct CompletionsArgs {
    /// Shell to generate completions for.
    #[arg(value_enum)]
    pub shell: CompletionShell,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum CompletionShell {
    Bash,
    Zsh,
    Fish,
    Powershell,
    Elvish,
}
