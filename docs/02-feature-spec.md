# 02 · Full feature list

> This is the Winia Forge product contract. The CLI skeleton already defines the command surface from
> this list; implementation status appears at the end of each section and in
> [05-roadmap.md](05-roadmap.md).

## 0. Command overview

| Command | Purpose | Current status |
| --- | --- | --- |
| `winia new` | Create a new project from a template | Implemented (blank template, desktop + Android) |
| `winia init` | Add Winia to an existing Rust project | Planned |
| `winia build` | Build the application | Implemented: host executable and Android APK (NDK driver + Gradle) |
| `winia run` | Build and run | Implemented: host launch, Android install/start |
| `winia dev` | Development mode: file watching + restart | Implemented (desktop restart, Android reinstall/restart, automatic debug-server) |
| `winia inspect` | Debug-server client | Implemented: info, tree, click, key, scroll, screenshot, registry/port discovery |
| `winia test` | Unit, UI, and device smoke tests | Planned |
| `winia package` | Package installers/archives/APK/AAB | Implemented for Android APK/AAB |
| `winia sign` | Code signing | Implemented for Android APKs |
| `winia release` | Version, tag, publish | Implemented for Android (local artifacts + GitHub upload) |
| `winia doctor` | Environment diagnostics | Implemented (Rust/MSRV, platform toolchains, Android SDK/NDK, `--fix`) |
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
- The `winia` crate is not published on crates.io yet, so `--winia-path` (or `WINIA_PATH`) is
  required; the generated `Cargo.toml` uses a path dependency to that checkout.
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
| Android | Built-in NDK driver + Gradle (no cargo-ndk; see 03-android-support.md) |

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

A single target uses the shape above. `--target all` emits
`{ "targets": [ { "target", "profile", "artifacts" }, ... ], "duration_ms": ... }`.

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
- Arguments after `--` are passed through to the desktop application. On Android they become string
  intent extras (`winia.args` holds the joined value and `winia.argN` each argument).
- Android: selects the device (or `--device`), reads `ro.product.cpu.abi`, builds only that ABI,
  installs with `adb install -r`, and starts the launcher activity parsed from the manifest with
  `am start -W`.

### 4.2 `winia dev`

- The watcher starts before the initial build and watches `src/`, `assets/`, `Cargo.toml`,
  `winia.toml`, `build.rs`, `android/app/src/`, and the Android Gradle files. `target/`, `dist/`,
  `.git/`, and Android build directories are never registered.
- Desktop: rebuild first; on Windows the executable runs from `target/winia-dev/` so Cargo can
  replace the build output. A successful build kills and restarts the old process; a failed build
  leaves the old app running.
- Android: recompile the device ABI, reinstall the APK, force-stop, and restart the Activity.
- If the desktop app exits on its own, dev reports it and waits for the next change.
- Build failures keep the watcher alive; the next edit retries.
- `--debounce <ms>` sets event coalescing (default 300); `--once` builds and runs once, waiting for a
  desktop app to close.
- Debug builds automatically enable the `winia/debug-server` feature. `--debug-port` is the starting
  port (default 9998); Winia scans up to +100 on a conflict and writes the actual desktop port to
  `target/winia-dev/debug-port-<pid>.txt`. Dev reads that file, waits for the WebSocket handshake, and
  prints `ws://127.0.0.1:<actual>`. `--no-debug-server` opts out; release builds disable it automatically.
- Android: Winia scans device ports from 9998. Dev picks a free host port (starting from `--debug-port`
  or 9998), reads the device port from the `RustStdoutStderr` logcat line, forwards
  `tcp:<host> tcp:<device>`, and prints the host URL. The debug manifest
  (`android/app/src/debug/AndroidManifest.xml`) needs `android.permission.INTERNET`; the blank
  template includes it and existing projects must add it. With `--once`, the forward is kept so the
  debug server remains reachable after the command returns; the command prints the removal command.
- Send `info` over the debug WebSocket to identify a port: it returns `app`, `version`, `id`,
  `target`, `platform`, `arch`, `pid`, and `port`. Desktop dev injects the metadata through
  environment variables; Android dev writes it to `files/winia-debug-info.json` in the app's private
  data directory, with the process package name as fallback.
- Note: Rust has no code hot-swapping; a "hot restart" is a process-level restart, not Flutter-style hot
  reload.

### 4.3 `winia test`

- Unit/layout/render tests: `cargo test -p <app>`.
- UI integration tests: with `--features debug-server`.
- Screenshot snapshots: on failure, print the diff and the actual image.
- Android smoke test: install, launch, check logcat for a panic, capture one screenshot.
- `--update-snapshots` updates the baselines.

### 4.4 Debugging tools

```bash
winia inspect info
winia inspect tree
winia inspect click 120 80
winia inspect key Return
winia inspect scroll 120
winia inspect screenshot --out inspect.png
winia inspect discover
```

- `--port <port>` selects the debug server (default 9998); `--json` emits compact output.
- `--timeout <ms>` sets the discovery/connection timeout (default 500).
- `discover` reads `target/winia-dev/inspect-port-<pid>.json` files written by `winia dev` and probes
  only those ports. Pass `--scan` to scan `--port` through `--port + 100` instead.
- `info` returns app, version, id, target, platform, arch, pid, and port; `tree` returns the layout
  tree JSON.
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

See [03-android-support.md](03-android-support.md). Implemented behaviour:

- `winia package --target android` builds a universal APK by default; `--split per-abi` builds one
  APK per configured ABI. AABs are produced with the Gradle bundle task.
- Artifacts land in `dist/` (or `--out-dir`) as
  `{app}-{version}-android-universal.apk`, `{app}-{version}-android-{abi}.apk`, and
  `{app}-{version}-android.aab`.
- `versionCode` / `versionName` from `winia.toml` are passed to Gradle as
  `winiaVersionCode` / `winiaVersionName` properties, so the APK metadata matches the artifact name.
- Every APK is checked for `zipalign -P 16` alignment, native ABIs (through `aapt2 dump badging`),
  version metadata, and (for debug builds) a valid signature. Release APKs stay unsigned until
  `winia sign`.
- `--json` and `--dry-run` describe the artifacts without building. `--json` emits
  `{ "target", "version", "formats", "split", "dry_run", "artifacts": [{ "path", "bytes" }],
  "duration_ms" }`.
- `--clean` removes only files matching `{app}-{version}-android-*.apk|aab` in the output
  directory; unrelated files are left alone.
- `--split` only affects the `apk` format; requesting it with `aab` alone prints a warning.
- AAB packaging builds the native libraries without assembling an intermediate APK, then runs the
  Gradle bundle task.
- `bundletool` validation runs when the tool is available; otherwise the command warns and continues.

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
- `winia sign --target android` signs every APK in `dist/` (or the `--apk` path) with
  `apksigner`, writing `*-signed.apk` copies unless `--in-place` is given. Relative `--apk` and
  `--keystore` paths resolve against the current directory.
- Passwords are read through `--ks-pass env:...`, so they never appear in the process list. The
  alias comes from `--alias` or `WINIA_ANDROID_KEY_ALIAS`.
- `--dry-run` lists the input/output pairs without touching the keystore; `--release` skips APKs
  that `aapt2` reports as debuggable.
- `apksigner` runs from `lib/apksigner.jar` through `java` when available, falling back to the
  batch launcher. This keeps SDK paths containing spaces intact.
- The command verifies the result with `apksigner verify --print-certs` and reports the certificate
  DN. AAB signing is not implemented yet.

### 6.4 `winia release`

```bash
winia release --target android --dry-run
winia release --target android --bump patch
winia release --target android --bump patch --push
winia release --target android --bump minor --track github
```

Implemented flow (Android only):

1. Refuse a dirty worktree unless `--allow-dirty`.
2. Validate the signing environment and, for GitHub, `gh auth status` plus a git remote before
   modifying the repository.
3. Bump `app.version`, `android.version_code`, and the `Cargo.toml` package version; commit
   `chore(release): vX.Y.Z`.
4. Generate notes from conventional commits since the previous tag.
5. Package the release APK/AAB through `winia package`.
6. Sign the APK when a keystore is configured; `latest.json` lists the signed APK.
7. Create the annotated tag unless `--no-tag`.
8. Push HEAD and the tag when `--push` is given, or automatically with `--track github`.
9. With `--track github`, upload the artifacts to a GitHub release (the tag must exist remotely).
10. Write `dist/latest.json` with the version, tag, and artifact sizes.

`--json` prints `{ target, version, tag, pushed, published, artifacts, notes, duration_ms }`.

Not implemented yet: Play track uploads, desktop/macOS releases, rollback after a failed build, and
checksums in `latest.json`.

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
prerequisites are reported as warnings; `--strict` turns warnings into failures. `--fix`
applies safe automatic fixes (installing missing Rust targets) and prints the exact commands
for everything that must be done manually.

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
- Android: SDK/NDK installation, Gradle caching, the built-in NDK driver.
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
| `winia new` blank template | Implemented; desktop + Android GameActivity scaffold |
| `winia doctor` environment checks | Implemented; covers MSRV, host platform toolchains, Android SDK/NDK/licenses, `winia.toml` validation, `--strict` and `--fix` |
| `winia info` / `targets` | Implemented |
| `winia config show/get` | Implemented |
| Shell completions | Implemented |
| `winia platform add android` | Placeholder directory |
| `winia build` | Implemented: host executable and Android APK |
| `winia run` | Implemented: host and Android |
| `winia package` | Implemented: Android APK (universal or per-ABI) and AAB, with alignment/ABI/signature checks |
| `winia sign` | Implemented: Android APK signing with apksigner |
| `winia release` | Implemented: Android bump/package/sign/tag/latest.json and optional GitHub upload |
| Template system, hooks, plugins | Planned |
