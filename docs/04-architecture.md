# 04 · Architecture design

> Status: the CLI scaffold is in place; this document describes the target architecture and the parts that
> have already landed.

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
│   │   │   └── commands/      # one module per command
│   │   └── Cargo.toml
│   ├── winia-build/           # build.rs resource generation
│   └── winia-pack/            # cross-platform packaging
└── docs/
```

### 1.1 Crate responsibilities

| crate | Responsibility | Current state |
| --- | --- | --- |
| `winia-cli` | Command parsing, flow orchestration, invoking Cargo/Gradle/adb/signing tools | Skeleton |
| `winia-build` | Windows .rc, macOS Info.plist/.icns, Linux .desktop, Android manifest metadata | Placeholder |
| `winia-pack` | Per-platform packaging logic (Windows/macOS/Linux/Android modules) | Placeholder |

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

- `clap`: command parsing and help/completions.
- `serde` + `toml`: `winia.toml`.
- `serde_json`: `--json` output.
- `anyhow`: error context.
- Possible future additions: `cargo_metadata` (project metadata), `camino` (UTF-8 paths),
  `dialoguer` (interactive wizard), `indicatif` (progress bars).

### 2.2 Principles

1. **Do not reimplement Cargo**: building, testing, and cleaning all call `cargo`.
2. **Do not reimplement Gradle**: Android resources, the Manifest, and the AAB are all left to Gradle.
3. **Do not reimplement packaging formats**: Windows calls WiX/NSIS, macOS calls hdiutil/codesign/notarytool,
   Linux calls dpkg-deb/rpmbuild/AppImage tooling.
4. **Every command supports `--json` and `--dry-run`**; this is the foundation for IDE and CI integration.
5. **Errors must be actionable**: say what is missing, how to install it, and which phase failed.
6. **Version checks**: on startup the CLI reads the project's `winia.toml` and `Cargo.toml` and gives a clear
   warning when the Winia version does not match the range the CLI supports.

### 2.3 Command dispatch

```text
main.rs
  → cli::Cli::parse()
  → commands::dispatch()
      → new / doctor / info / config / completions
      → other commands: return a readable error while unimplemented
```

One module per command keeps the CLI layer thin; the real build/package logic belongs in `winia-build` /
`winia-pack`.

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
- `validate()` performs cross-field checks (app id, Android min_sdk, ABI).
- A `schema_version` will be added later, with migration functions for older versions.
- The same model is shared by the CLI and `winia-build`, so there is only one configuration parser.

## 4. Template system

Recommended implementation, in priority order:

1. Built-in templates are compiled into the binary with `include_str!` / `include_dir!`, guaranteeing
   offline availability.
2. Template variables use simple placeholder substitution (`__APP_NAME__`, `__APP_ID__`, `__VERSION__`)
   rather than a full template engine.
3. Introduce `minijinja` or `handlebars` only when conditional sections are needed.
4. Remote templates are downloaded into a cache directory, checksum-verified, and then copied.

Template versions are tied to the CLI version; a template declares `min_winia_version`, which the CLI checks
at startup.

## 5. Process orchestration

### 5.1 Build

```text
winia build
  → read winia.toml
  → select target/profile
  → cargo build (desktop) or cargo ndk (Android)
  → winia-build generates platform resources
  → collect artifacts, produce the report
```

### 5.2 Android

```text
winia build --target android
  → check SDK/NDK/JDK
  → cargo ndk -t <abi> -o android/app/src/main/jniLibs build --release --lib
  → gradle assembleDebug / assembleRelease / bundleRelease
  → collect APK/AAB
  → optional apksigner/zipalign verification
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

- Unit tests: configuration parsing, command argument combinations, path computation.
- Snapshot tests: the output of `winia --help` and subcommand help.
- Integration tests: run `winia new` in a temporary directory and inspect the generated files.
- Later: fixture projects to test build/package argument assembly (without actually compiling).

## 7. Release and versioning

- The compatible range between the CLI version and the Winia framework version is recorded in `Cargo.toml`
  and the `winia.toml` schema.
- `winia-cli` is published to crates.io; binaries may also be distributed through GitHub Releases.
- Before releasing, run `cargo fmt --check`, `cargo clippy -- -D warnings`, `cargo test --workspace`.
- Commit messages follow conventional commits.
