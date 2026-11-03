# 02 · Full feature list

> This is the Winia Forge product contract. The CLI skeleton already defines the command surface from
> this list; implementation status appears at the end of each section and in
> [05-roadmap.md](05-roadmap.md).

## 0. Command overview

| Command | Purpose | Current status |
| --- | --- | --- |
| `winia new` | Create a new project from a template | Minimal version implemented (blank template) |
| `winia init` | Add Winia to an existing Rust project | Planned |
| `winia build` | Build the application | Planned |
| `winia run` | Build and run | Planned |
| `winia dev` | Development mode: file watching + debug server | Planned |
| `winia test` | Unit, UI, and device smoke tests | Planned |
| `winia package` | Package installers/archives/APK/AAB | Planned |
| `winia sign` | Code signing | Planned |
| `winia release` | Version, tag, publish | Planned |
| `winia doctor` | Environment diagnostics | Basic version implemented |
| `winia add` / `remove` / `update` | Dependency management | Planned |
| `winia assets` | Icon and asset generation | Planned |
| `winia ci` | Generate CI configuration | Planned |
| `winia upgrade` | Framework upgrade and code migration | Planned |
| `winia clean` | Remove build artifacts | Planned |
| `winia info` | Environment and project information | Implemented |
| `winia targets` | Supported/installed target platforms | Implemented |
| `winia config` | View/read `winia.toml` | show/get implemented |
| `winia devices` | List adb devices | Planned |
| `winia emulators` | Android emulator management | Planned |
| `winia logcat` | Android log filtering | Planned |
| `winia screenshot` | Screenshots | Planned |
| `winia platform` | Add platform support | Planned (android placeholder) |
| `winia android` | Android-specific operations | Planned |
| `winia completions` | Shell completions | Implemented |

## 1. Project creation and templates

### 1.1 Creating a project

```bash
winia new my-app
winia new my-app --platforms windows,macos,linux,android
winia new my-app --template counter --no-git
```

Requirements:

- The default directory is `./<name>`; `--path` overrides it.
- When developing Winia itself, point at a local crate with `--winia-path D:\Projects\winia\winia` or the
  `WINIA_PATH` environment variable; without it, `winia = "0.2"` is generated.
- Interactive wizard: template, target platforms, Rust edition, render backend, whether to initialise git.
- Non-interactive mode: every option can be passed as a flag, suitable for scripts and CI.
- A non-empty target directory is refused by default; with `--force`, the files that would be overwritten
  are listed first.

### 1.2 Built-in templates

| Template | Contents |
| --- | --- |
| `blank` | Minimal runnable window |
| `counter` | Introductory example with state, a button, and text |
| `multi-window` | Multi-window application |
| `navigation` | Navigation 3 page stack |
| `dashboard` / `settings` | Full application skeleton with a sidebar and pages |
| `tray` | Tray/background application (needs window icons and the hidden-main-window API) |
| `gallery` | Component browser and debugging surface |

Later support:

- `--template <git-url>` to pull a template from a remote.
- Template variables: application name, package name, author, license, version, target platforms.
- Template cache and offline mode.

### 1.3 Generated contents

```text
my-app/
├── Cargo.toml
├── winia.toml
├── README.md
├── .gitignore
├── assets/
│   └── icon.png
└── src/
    ├── main.rs          # desktop entry point
    ├── lib.rs           # Android entry point (when the Android target is enabled)
    └── ui.rs            # UI shared by desktop and mobile
```

Also generated:

- `rust-toolchain.toml`: pins the MSRV.
- `.cargo/config.toml`: optional settings such as sccache.
- CI workflow (optional).
- Recommended VSCode settings (optional).

### 1.4 Adopting an existing project

```bash
winia init
winia init --platforms android
```

Behaviour:

- Read the existing `Cargo.toml` and add the Winia dependency.
- Generate `winia.toml` and `assets/`.
- Do not overwrite existing source; only add files and a small amount of configuration.
- `--dry-run` shows the files that would be added or modified.

### 1.5 Templates and version management

- Template versions follow the CLI version; the Winia version declared by a template must match the range
  supported by the CLI.
- Upgrade templates with `winia upgrade`; never overwrite user files directly.
- Remote templates need signature or checksum verification to avoid supply-chain risk.

## 2. Configuration: `winia.toml`

### 2.1 Complete example

```toml
[app]
name = "My App"
id = "com.example.myapp"
version = "0.1.0"
authors = ["Alice <alice@example.com>"]
license = "Apache-2.0"
icon = "assets/icon.png"

[window]
title = "My App"
width = 1200
height = 800
resizable = true
decorations = true

[build]
features = ["accessibility"]
backend = "auto"          # auto | vulkan | gl | cpu
profile = "dev"           # dev | release | release-lto | minsize

[package.windows]
formats = ["zip", "nsis", "msi"]

[package.macos]
formats = ["app", "dmg"]

[package.linux]
formats = ["tar.gz", "deb", "appimage"]

[package.android]
formats = ["apk", "aab"]

[android]
package = "com.example.myapp"
label = "My App"
min_sdk = 26
target_sdk = 36
compile_sdk = 36
activity = "game"          # game | native
version_code = 1
abis = ["arm64-v8a", "x86_64"]
orientation = "unspecified" # portrait | landscape | unspecified
permissions = ["INTERNET"]

[hooks]
before_build = []
after_build = []
before_package = []
after_package = []
```

### 2.2 Configuration reading rules

- The CLI searches upwards from the current directory for `winia.toml`, up to the filesystem root.
- Every field has a default; a missing section is not an error.
- `winia config show` prints the merged effective configuration.
- `winia config get android.min_sdk` reads a single key (dot path).
- Environment variables may override a few machine-specific settings (such as `WINIA_RENDER_BACKEND`), but
  project identity settings must not be overridable from the environment.
- The file carries a `schema_version` (added later); upgrades migrate it automatically.

### 2.3 Validation rules

- `app.id` must be reverse-DNS.
- `android.min_sdk >= 26` (the Skia Android build baseline).
- Android ABIs may only be `arm64-v8a`, `armeabi-v7a`, `x86_64`, `x86`.
- `app.version` must be semver.
- The icon file must exist and be readable.

## 3. Building

### 3.1 Basic commands

```bash
winia build
winia build --release
winia build --target windows --release
winia build --target android --release --abi arm64-v8a,x86_64
winia build --target all --json
winia check
winia clean --all
```

### 3.2 Desktop builds

- Supports four profiles: `dev`, `release`, `release-lto`, `minsize`.
- Builds through Cargo; the compiler is never reimplemented.
- `--features` is passed straight to Cargo.
- Artifact paths and sizes are written to the report; `--json` emits machine-readable output.
- Windows resources (icon, version, company name, manifest) are generated by `winia-build` in the
  `build.rs` phase.

### 3.3 Targets and cross-compilation

| Approach | Support |
| --- | --- |
| Native builds | Windows / macOS / Linux each build on their own host |
| Linux → Windows | `cargo-xwin` |
| Windows → Linux | WSL / `cargo-zigbuild` |
| macOS | Can only be built on macOS (the tool detects this and says so) |
| Android | `cargo-ndk` + Gradle (see 03-android-support.md) |

`winia build --target all` builds the target matrix one by one; when a toolchain is missing on the host it
reports a clear error instead of silently skipping.

### 3.4 Build report

```json
{
  "target": "windows",
  "profile": "release",
  "artifacts": [
    {"path": "target/release/my-app.exe", "bytes": 12345678}
  ],
  "duration_ms": 34567
}
```

## 4. Running, development, and testing

```bash
winia run
winia run --release
winia run --target android --device emulator-5554
winia dev
winia dev --target android
winia test
winia test --filter scrollbar
```

### 4.1 `winia run`

- Picks the current platform automatically; `--target` overrides it.
- `--release` uses the release profile.
- Arguments after `--` are passed through to the application.
- Android: build a debug APK, `adb install -r`, `am start -W`, and wait for the launch result.

### 4.2 `winia dev`

- File watching: `.rs`, `winia.toml`, `assets/`.
- Desktop: process-level restart.
- Android: recompile the `.so`, reinstall the APK, restart the Activity.
- Starts the debug-server automatically and opens the `winia inspect` WebSocket connection.
- Android runs `adb reverse` automatically.
- Note: Rust has no code hot-swapping; a "hot restart" is a process-level restart, not Flutter-style hot
  reload.

### 4.3 `winia test`

- Unit/layout/render tests: `cargo test -p <app>`.
- UI integration tests: with `--features debug-server`.
- Screenshot snapshots: on failure, print the diff and the actual image.
- Android smoke test: install, launch, check logcat for a panic, capture one screenshot.
- `--update-snapshots` updates the baselines.

### 4.4 Debugging tools

- `winia inspect`: connect to the debug-server to inspect the layout tree, simulate clicks, and capture
  screenshots.
- `winia screenshot`: desktop or device screenshots.
- `winia trace`: enable `anim-trace` and emit per-frame animation traces.
- Windows UIA verification (later): check the accessibility tree.

## 5. Packaging

### 5.1 Platforms and formats

| Platform | Portable artifact | Installers |
| --- | --- | --- |
| Windows | `zip` portable directory | NSIS `setup.exe`, MSI (WiX), MSIX (optional) |
| macOS | `.app`, `zip` | `dmg`, `pkg` |
| Linux | `tar.gz` | `deb`, `rpm`, `AppImage` (Flatpak later) |
| Android | APK (debug/release) | AAB (Play Store), split APKs |

```bash
winia package
winia package --release --format zip
winia package --target windows --format msi,nsis
winia package --target android --format apk,aab --split per-abi
```

### 5.2 General requirements

- The version comes from `app.version` in `winia.toml`; Android's `versionCode` is maintained separately.
- Artifact naming template: `{app}-{version}-{os}-{arch}.{ext}`.
- The output directory defaults to `dist/`; `--out-dir` overrides it.
- Generates SHA256 checksums and a JSON manifest.
- Optionally generates a third-party license list.
- The build report includes each artifact's size, so accidental bloat is easy to spot.

### 5.3 Windows packaging

- Icons: multi-size `.ico` (16–256).
- Version resources: product name, company name, copyright, file version.
- NSIS: install directory, Start menu, uninstaller, optional run-at-startup.
- MSI: WiX project, suitable for enterprise distribution.
- MSIX: optional; requires signing and a certificate.
- Dependencies: VC++ runtime detection or static-linking guidance.

### 5.4 macOS packaging

- `.app`: `Info.plist`, bundle id, minimum system version, LSUIElement (tray applications).
- Icons: `.icns`.
- Universal binary: `--arch universal` builds x86_64 + arm64.
- DMG: drag-to-install layout, background image, volume label.
- PKG: install scripts, permissions, signing.
- Notarisation: `notarytool` submission, `stapler` stapling, `spctl` verification.

### 5.5 Linux packaging

- `tar.gz`: portable directory.
- `deb`: control file, dependencies, maintainer scripts, desktop file, icons.
- `rpm`: spec file, suitable for Fedora/RHEL.
- `AppImage`: single file with bundled dependencies.
- `.desktop` and AppStream metadata.
- Dependency declarations: `libx11`, `libvulkan1` (optional), `libfontconfig1`, and so on.

### 5.6 Android packaging

See [03-android-support.md](03-android-support.md). Key points:

- APK and AAB.
- ABI split / universal.
- `zipalign -P 16` to satisfy the 16 KB page requirement on Android 15+.
- `apksigner` signing and verification.
- `bundletool` AAB validation.

## 6. Signing and publishing

### 6.1 Windows

- Authenticode: `signtool`, with PFX and certificate-store support.
- Timestamp server.
- Verify with `signtool verify` after signing.

### 6.2 macOS

- `codesign`: Developer ID, entitlements, hardened runtime.
- `notarytool` notarisation, `stapler` stapling.
- `spctl --assess` verification.

### 6.3 Android

- Keystore creation and secure storage.
- Environment variables:
  - `WINIA_ANDROID_KEYSTORE`
  - `WINIA_ANDROID_KEY_ALIAS`
  - `WINIA_ANDROID_KEYSTORE_PASSWORD`
  - `WINIA_ANDROID_KEY_PASSWORD`
- Google Play App Signing: the upload key and the Play signing key are kept separate.
- `winia sign --target android` wraps `apksigner` and verifies the certificate.

### 6.4 `winia release`

```bash
winia release --dry-run
winia release --target windows,macos,linux --track github
winia release --target android --track internal
```

Flow:

1. Verify the worktree is clean (`--allow-dirty` skips this).
2. Bump the semantic version (`--bump major|minor|patch`).
3. Generate a changelog from conventional commits.
4. Create the git tag.
5. Build artifacts for every platform.
6. Sign/notarise.
7. Upload to GitHub Releases or Google Play.
8. Generate `latest.json` / the update manifest (when auto-update is enabled).

## 7. Environment and toolchain

### 7.1 `winia doctor`

Checks:

- Rust: `rustc`, `cargo`, MSRV.
- Rust targets: `rustup target list --installed`.
- Windows: MSVC Build Tools, Windows SDK, LLVM (needed by Skia), Vulkan SDK (optional).
- macOS: Xcode Command Line Tools.
- Linux: `pkg-config`, `fontconfig`, `libx11`.
- Android: `ANDROID_HOME` / `ANDROID_SDK_ROOT`, `ANDROID_NDK`, `ANDROID_NDK_HOME`, `JAVA_HOME`, `adb`,
  SDK licenses.
- Project: whether `winia.toml` exists and validates.

```bash
winia doctor
winia doctor --target android
winia doctor --target android --json
winia doctor --strict
winia doctor --fix
```

`doctor` exits non-zero when a required check is missing. Without `--target`, Android
prerequisites are reported as warnings; `--strict` turns warnings into failures.

### 7.2 Toolchain management

- `winia targets`: list supported/installed targets.
- `winia setup`: first-run installation wizard (planned).
- `winia info`: version, OS, architecture, configuration path.
- `winia upgrade`: upgrade the Winia dependency and run codemods.

## 8. Assets and icons

```bash
winia assets icon assets/icon.png
```

- From a 1024×1024 PNG/SVG, generate:
  - Windows `.ico` (16, 24, 32, 48, 64, 128, 256).
  - macOS `.icns`.
  - Linux hicolor multi-size PNGs.
  - Android adaptive icons: foreground/background layers + mipmap densities.
- Generate the 512×512 Play Store icon.
- Validate icon transparency, size, and safe margins.
- Sync assets into each platform's packaging directory.

## 9. CI/CD

```bash
winia ci init --provider github --platforms windows,macos,linux,android
```

The generated GitHub Actions workflow includes:

- A three-platform matrix: `windows-latest`, `macos-latest`, `ubuntu-latest`.
- Rust caching, sccache.
- Android: SDK/NDK installation, Gradle caching, `cargo-ndk`.
- build + test + package.
- Artifact upload.
- Signing/notarisation/Play upload on release (using Secrets).

GitLab CI emits an isomorphic stage/job configuration. Every command supports `--json` so CI can be
customised freely.

## 10. Extension mechanisms

### 10.1 hooks

```toml
[hooks]
before_build = ["echo building"]
after_build = []
before_package = []
after_package = []
```

- Run before/after the corresponding phase; a failure aborts the run.
- The working directory is the project root.
- The environment contains `WINIA_TARGET`, `WINIA_PROFILE`, `WINIA_VERSION`.

### 10.2 Custom templates

```bash
winia template add https://github.com/example/winia-template
winia template list
winia template update
```

- Templates live in the user configuration directory, never inside the project.
- Supports checksums and pinning to a git tag.

### 10.3 External subcommands

- An executable named `winia-<name>` automatically becomes the `winia <name>` subcommand (like cargo
  plugins).
- External subcommands receive the project path and configuration through environment variables.

### 10.4 Custom packagers

- `winia.toml` can name an external command for a given format.
- The CLI passes arguments, collects artifacts, and checks the exit code.

## 11. Upgrades and migration

```bash
winia upgrade --check
winia upgrade
```

- Checks the latest Winia version and the `winia.toml` schema.
- Updates the `Cargo.toml` dependency.
- Runs codemods for breaking API changes.
- Creates a git branch or backup before migrating.
- Prints the change list and the items that need manual confirmation.

## 12. Current scaffold implementation status

| Feature | Status |
| --- | --- |
| clap command surface | Implemented; covers every command in this document |
| Minimal `winia new` template | Implemented |
| `winia doctor` environment checks | Basic version implemented |
| `winia info` / `targets` | Implemented |
| `winia config show/get` | Implemented |
| Shell completions | Implemented |
| `winia platform add android` | Placeholder directory |
| Build, run, package, sign, release | Planned |
| Template system, hooks, plugins | Planned |
