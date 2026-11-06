# 04 · Architecture design

> Status: every command in the CLI is implemented. This document describes what the code does today
> and marks the parts that remain placeholders.

## 1. Workspace structure

```text
winia-forge/
├── Cargo.toml                 # workspace, shared version and dependencies
├── crates/
│   ├── winia-cli/             # library + binary winia
│   │   ├── src/
│   │   │   ├── main.rs
│   │   │   ├── lib.rs
│   │   │   ├── cli.rs         # clap command surface
│   │   │   ├── config.rs      # winia.toml model
│   │   │   ├── icons.rs       # per-platform icon sets
│   │   │   ├── assets.rs      # asset collection, hashing, staging
│   │   │   ├── hooks.rs       # project hooks
│   │   │   ├── platform/      # ABI table, build-tools discovery
│   │   │   ├── commands/      # one module per command
│   │   │   └── templates/     # embedded blank template
│   │   ├── tests/help.rs      # command-surface guard
│   │   └── Cargo.toml
│   └── winia-pack/            # NSIS, MSI (WiX), deb
└── docs/
```

### 1.1 Crate responsibilities

| crate | Responsibility | Current state |
| --- | --- | --- |
| `winia-cli` | Command parsing, orchestration, Cargo/Gradle/adb/signing invocation, icons, assets, hooks | Every command implemented |
| `winia-pack` | Installer formats: NSIS script, WiX MSI, deb | Implemented for NSIS/MSI/deb; DMG/app/zip/tar.gz are built in `winia-cli` |

If Android's JNI and platform-bridge code grows large, split out a `winia-android` crate:

```text
winia-android/
├── jni/                 # JNI entry points
├── lifecycle.rs         # Activity lifecycle
├── insets.rs            # system insets
├── clipboard.rs         # ClipboardManager bridge
└── assets.rs            # AssetManager
```

It would only be depended on by the Android target and would never enter desktop builds.

## 2. CLI implementation conventions

### 2.1 Dependencies

- `clap` / `clap_complete`: command parsing, help and shell completions.
- `serde` + `toml` + `toml_edit`: read `winia.toml`; edit `Cargo.toml` without reformatting it.
- `serde_json`: `--json` output and the asset manifest.
- `anyhow`: error context.
- `sha2`: asset and installer checksums, icon cache markers.
- `image`, `resvg`, `ico`, `icns`: icon rendering and per-platform encoding.
- `notify` + `ctrlc`: `winia dev` file watching and clean shutdown.
- `semver`: config validation and release version bumps.

### 2.2 Principles

1. **Do not reimplement Cargo**: building, testing, and cleaning all call `cargo`.
2. **Do not reimplement Gradle**: Android resources, the Manifest, and the AAB are all left to Gradle.
3. **Do not reimplement packaging formats**: Windows calls WiX/NSIS, macOS calls hdiutil/codesign/notarytool,
   Linux calls rpmbuild/AppImage tooling; deb is the deliberate exception and is written directly
   as an ar archive (see docs/02).
4. **Machine-readable output where it matters**: `build`, `package`, `sign`, `release`, `doctor`,
   `test`, `clean`, `android sync`, `devices`, `screenshot` and `config show` support `--json`;
   `--dry-run` exists on `build`, `package`, `sign`, `release`, `clean` and `android sync --check`.
5. **Errors must be actionable**: say what is missing, how to install it, and which phase failed.
6. **Version checks**: `winia doctor` reports the project's `winia` dependency (declared per target,
   path validity, version drift). There is no automatic check on every command.

### 2.3 Command dispatch

```text
main.rs
  → cli::Cli::parse()
  → commands::dispatch()
      → one arm per command; every arm is implemented
```

One module per command keeps the CLI layer thin. Installer script generation lives in `winia-pack`
(NSIS/MSI/deb); everything else — build, packaging, icons, assets, signing — is in `winia-cli`.

## 3. Configuration model

`winia-cli/src/config.rs` defines `WiniaConfig`:

```text
WiniaConfig
├── app: AppConfig
├── window: WindowConfig
├── build: BuildConfig
├── package: PackageConfig
│   ├── windows: PlatformPackage
│   ├── macos: PlatformPackage
│   ├── linux: PlatformPackage
│   └── android: PlatformPackage
├── android: AndroidConfig
└── hooks: HooksConfig
```

Design points:

- Every field has a default; a missing section is not an error.
- `#[serde(default)]` guarantees backward compatibility.
- `validate()` checks the app id (reverse-DNS), `app.version` (semver), `android.package`
  (non-empty), `android.min_sdk >= 26` and every ABI name.
- A `schema_version` will be added later, with migration functions for older versions.
- Only `winia-cli` parses the file; `winia-pack` consumes the packaging fields without owning the model.

## 4. Template system

Implemented state:

1. One built-in template (`blank`) is compiled into the binary, so `winia new` works offline.
2. Variables use literal placeholder substitution (`__APP_NAME__`, `__APP_ID__`, `__VERSION__`,
   `__WINIA_DEP_*`); there is no template engine.
3. `--template` accepts only `blank`; remote templates are not implemented and the flag documents the
   future git-URL form.
4. `winia android sync` regenerates the managed Gradle subset from `winia.toml`; other template files
   are written once and then belong to the project.

Not implemented: additional built-in templates, remote templates with checksum verification, and a
template-declared `min_winia_version`.

## 5. Process orchestration

### 5.1 Build

```text
winia build
  → read and validate winia.toml
  → run before_build hooks
  → desktop: cargo build (host) with WINIA_ASSET_ROOT/TARGET/PROFILE/VERSION
  → android: NDK toolchain → cargo build --lib per ABI → copy jniLibs → gradlew assemble*
  → regenerate src/winia_config.rs (window values the app reads at runtime)
  → stage assets (desktop dir or Android asset dir) and icons
  → run after_build hooks
  → collect artifacts and sizes, print the report
```

### 5.2 Android

```text
winia build --target android
  → locate the SDK/NDK (prefers 27.0.12077973), validate the clang wrapper API level
  → per ABI: cargo build --lib with the NDK linker/CC/AR and RUSTFLAGS max-page-size=16384
  → copy <lib>.so into android/app/src/main/jniLibs/<abi>/
  → llvm-readelf check that every LOAD segment is 0x4000-aligned
  → gradlew assembleDebug / assembleRelease / bundleRelease with winiaVersionCode/Name
```

### 5.3 Run

```text
desktop: cargo run → child process
android: adb install -r → adb shell am start -W
```

Process-management notes:

- On Windows, terminating child processes requires a Job Object or `taskkill /T` to avoid orphans.
- On Android, `am force-stop` before restarting the Activity.
- Logs must be forwarded in real time, not printed only at the end.

## 6. Testing strategy

- Unit tests: configuration parsing and validation, command argument assembly, path computation,
  icon/asset rules, installer script generation (`cargo test --workspace`).
- Command-surface test (`crates/winia-cli/tests/help.rs`): every command is registered and has a
  description, and `upgrade` stays absent.
- CI (GitHub Actions, three platforms) builds the CLI, then packages a fixture app
  (`.github/fixtures/ci-app`) with zip, NSIS, MSI, dmg/codesign and deb/dpkg to prove the packaging
  paths end to end without compiling Skia.
- Not covered: a fixture that builds the framework itself (too slow), and Gradle/AGP upgrade tests.

## 7. Release and versioning

- `winia-cli` is not published to crates.io yet; CI and `winia ci init` install it from the git
  repository (`--winia-source`).
- The project's `winia` dependency is a path or git source; `winia doctor` reports drift between the
  declared dependency and this CLI.
- Before releasing, run `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings`
  and `cargo test --workspace`; CI does the same on Windows, macOS and Linux.
- `winia release` bumps `winia.toml`, packages/signs, tags, writes `dist/SHA256SUMS` and
  `dist/latest.json`, and can create a GitHub release (`--track github`).
- Commit messages follow conventional commits.
