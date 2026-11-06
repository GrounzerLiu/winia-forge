# 02 · Full feature list

> This is the Winia Forge product contract. The CLI skeleton already defines the command surface from
> this list; implementation status appears at the end of each section and in
> [05-roadmap.md](05-roadmap.md).

## 0. Command overview

| Command | Purpose | Current status |
| --- | --- | --- |
| `winia new` | Create a new project from a template | Implemented (blank template, desktop + Android) |
| `winia init` | Add Winia to an existing Rust project | Implemented: Cargo dependency, `winia.toml`, optional Android scaffolding |
| `winia build` | Build the application | Implemented: host executable and Android APK (NDK driver + Gradle) |
| `winia run` | Build and run | Implemented: host launch, Android install/start |
| `winia dev` | Development mode: file watching + restart | Implemented (desktop restart, Android reinstall/restart, automatic debug-server) |
| `winia inspect` | Debug-server client | Implemented: info, tree, click, drag, key, scroll, screenshot, registry/port discovery |
| `winia test` | Unit, UI, and device smoke tests | Implemented: host cargo tests, Android install/launch/logcat/screenshot smoke |
| `winia package` | Package installers/archives/APK/AAB | Implemented: desktop portable zip/tar.gz/app, Windows NSIS and MSI, macOS dmg (on macOS), Linux deb, Android APK/AAB; PKG/rpm/AppImage/MSIX pending |
| `winia sign` | Code signing | Implemented: Android APKs (apksigner), Windows Authenticode (signtool), macOS codesign |
| `winia release` | Version, tag, publish | Implemented for Android and desktop targets (local artifacts + GitHub upload) |
| `winia doctor` | Environment diagnostics | Implemented (Rust/MSRV, platform toolchains, packaging tools, Android SDK/NDK, the project's `winia` dependency, `--fix`) |
| `winia assets` | Icon and asset generation | Implemented: icon sets from PNG/SVG; automatic via `app.icon` |
| `winia ci` | Generate CI configuration | Implemented: `ci init` for GitHub Actions and GitLab CI |
| `winia clean` | Remove build artifacts | Implemented: target/dist/Android build outputs, `--all` Gradle caches, `--dry-run`, `--json` |
| `winia info` | Environment and project information | Implemented |
| `winia targets` | Supported/installed target platforms | Implemented |
| `winia config` | View/read `winia.toml` | show/get implemented |
| `winia devices` | List adb devices | Implemented (`--json`) |
| `winia emulators` | Android emulator management | Implemented: list/start/stop (headless start) |
| `winia logcat` | Android log filtering | Implemented: device, tag/package/pid, level, `--clear` |
| `winia screenshot` | Screenshots | Implemented: adb screencap to a PNG (`--device`, `--output`, `--json`) |
| `winia android` | Android-specific operations | Implemented: `init`, `sync` (managed Gradle files) |
| `winia completions` | Shell completions | Implemented |

## 1. Project creation and templates

### 1.1 Creating a project

```bash
winia new my-app
winia new my-app --platforms windows,macos,linux,android
winia new my-app --template blank --no-git
```

Requirements:

- The default directory is `./<name>`; `--path` overrides it.
- `winia` 0.1 is published on crates.io; the generated `Cargo.toml` uses `winia = "0.1"`.
  `--winia-path` (or `WINIA_PATH`) overrides it with a local checkout for framework development.
- Only the `blank` template exists today; an unknown template is rejected.
- A non-empty target directory is refused. There is no overwrite flag; choose another directory.
- `--no-git` skips `git init`; otherwise `new` initialises a repository unless it is already
  inside an existing git worktree.

### 1.2 Built-in templates

| Template | Contents |
| --- | --- |
| `blank` | Minimal runnable window |

Planned (not implemented): `counter`, `multi-window`, `navigation`, dashboard/settings, tray and
gallery templates; remote `--template <git-url>`; template cache and offline mode. The CLI currently
has no interactive wizard and no `--force` overwrite for `new`.

### 1.3 Generated contents

```text
my-app/
├── Cargo.toml
├── winia.toml
├── README.md
├── .gitignore
├── assets/
└── src/
    ├── main.rs          # desktop entry point
    ├── lib.rs           # Android entry point (when the Android target is enabled)
    └── ui.rs            # UI shared by desktop and mobile
```

Also generated when the Android target is enabled: `android/` with the Gradle wrapper and
activity sources, and a `src/lib.rs` Android entry point. A CI workflow can be generated separately
with `winia ci init`.

### 1.4 Adding Winia to an existing project

```bash
winia init
winia init --path ../my-app --platforms windows,android
```

Implemented behaviour:

- Adds the `winia` dependency under `[target.'cfg(not(target_os = "android"))'.dependencies]`
  (desktop) and `[target.'cfg(target_os = "android")'.dependencies]` (Android), editing
  `Cargo.toml` with `toml_edit` so comments and formatting survive. If the project already has a
  `winia` dependency, its source and extra features are preserved: a plain dependency is merged in
  place, and a missing target copies the existing target's source.
- Writes `winia.toml` from the Cargo package name and version, and creates `assets/`.
- With `--platforms ...,android`: adds `[lib] crate-type = ["rlib", "cdylib"]`, appends an
  `android_main` entry point and a `winia_config` module declaration to the existing library
  source (creating the lib file when absent), and generates the Gradle project plus the wrapper.
  An existing `android/` tree is refused unless `--force` is passed.
- Refuses to run twice (`winia.toml` already present) or when `Cargo.toml` is missing, and
  rejects a `--winia-path` that points at a workspace manifest instead of the crate.
- `ios` is rejected explicitly.

### 1.5 Adopting an existing project

```bash
winia init
winia init --platforms android
```

Behaviour:

- Read the existing `Cargo.toml` and add or merge the Winia dependency.
- Generate `winia.toml` and `assets/`.
- With Android, append the entry point/module declaration to the existing library source; never
  replace the whole file. Existing `android/` files are only rewritten with `--force`.
- A dry-run is not implemented yet.

### 1.6 Templates and version management

- Template versions follow the CLI version; the Winia version declared by a template must match the range
  supported by the CLI.
- Regenerating a template's files is a manual decision; nothing overwrites user files silently.
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

[assets]
include = ["assets/**"]
exclude = ["**/*.psd"]

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

`WiniaConfig::validate` runs before every build, package, release and doctor pass:

- `app.id` must be reverse-DNS: at least two dot-separated segments, each starting with a letter
  and containing only letters, digits or underscores.
- `app.version` must be semver (`major.minor.patch`, optional pre-release/build).
- `android.package` must not be empty.
- `android.min_sdk >= 26` (the Skia Android build baseline).
- Android ABIs may only be `arm64-v8a`, `armeabi-v7a`, `x86_64`, `x86`.
- `app.icon`, when set, is checked by the icon generator rather than by validation: a missing file
  fails the build with `app.icon not found: <path>`.

### 2.4 Field coverage

Applied by the Android build (passed to Gradle as `-P` properties, read by the generated
`app/build.gradle.kts`):

| Field | Property | Effect |
| --- | --- | --- |
| `app.version` | `winiaVersionName` | `versionName` in the APK |
| `android.version_code` | `winiaVersionCode` | `versionCode` in the APK |
| `android.min_sdk` | `winiaMinSdk` | `minSdkVersion`, and the NDK clang wrapper API level |
| `android.target_sdk` | `winiaTargetSdk` | `targetSdkVersion` |
| `android.compile_sdk` | `winiaCompileSdk` | `compileSdk` |
| `android.label` (falls back to `app.name`) | `winiaAppLabel` | `app_name` string resource behind `android:label="@string/app_name"` |
| `[window]` title/width/height | generated `src/winia_config.rs` | Written by `winia new`, `init` and every `build`; the template UI reads `WINDOW_TITLE`/`WINDOW_WIDTH`/`WINDOW_HEIGHT` |
| `[window]` resizable/decorations | generated `src/winia_config.rs` | `Window::resizable(bool)` / `Window::decorations(bool)` in winia (v2 `1a98797`), driven by `WINDOW_RESIZABLE`/`WINDOW_DECORATIONS` |
| `app.authors` (first) | packaging metadata | NSIS `CompanyName`, MSI `Manufacturer`, deb `Maintainer` and copyright contact; falls back to `app.name` when empty |
| `app.license` | packaging metadata | NSIS `LegalCopyright`, deb `/usr/share/doc/<pkg>/copyright`, macOS `NSHumanReadableCopyright`; MSI has no licence field |
| `android.activity` | scaffold-time choice | `game` generates a `MainActivity extends GameActivity` subclass; `native` declares `android.app.NativeActivity` and ships no Java source. Switching requires `winia android init --force`, which also flips the `winia`/`winia-winit` backend feature |
| `android.permissions` | removed from the schema | Declare `<uses-permission>` in `AndroidManifest.xml`; the template ships `INTERNET` |

`android.label` reaches the manifest through a generated string resource, so projects created
before this change keep their literal `android:label` until they adopt `@string/app_name`.

These are parsed and round-tripped by `winia config`, but nothing reads them when building or
packaging. They are kept in the schema so projects can declare intent without breaking:

| Field | Current reality |
| --- | --- |

## 3. Building

### 3.0 Renderer backends

The renderer is not a `winia.toml` setting; it is a cargo feature of the `winia` dependency.

- The template pins one backend per target to keep builds small: desktop `features = ["gl"]`,
  Android `features = ["android-…", "vulkan"]`. Android cannot use GL: no Skia prebuilt exists for
  that combination, so it would compile Skia from source.
- Add the other desktop backend without editing the manifest: `winia build --features winia/vulkan`
  (cargo features are additive).
- Pin a single backend: edit the desktop dependency's `features` list in `Cargo.toml`. A rebuild
  follows, because the backend is compiled in.
- At runtime `WINIA_RENDER_BACKEND=vulkan|gl|cpu` selects among the compiled backends; the auto
  order is vulkan → gl → cpu. A backend that was not compiled in cannot be selected.
- Skia itself is fixed: on Windows the prebuilt archive is compiled with d3d/gl/vulkan, on Android
  with vulkan, so `skiwin` is the only layer a project can choose.

### 3.1 Basic commands

```bash
winia build
winia build --release
winia build --target windows --release
winia build --target android --release --abi arm64-v8a,x86_64
winia build --target all --json
winia clean --all
```

### 3.2 Desktop builds

- Builds the debug profile by default and the release profile with `--release`; there are no other
  profile switches yet.
- Builds through Cargo; the compiler is never reimplemented.
- `--features` is passed straight to Cargo.
- Artifact paths and sizes are written to the report; `--json` emits machine-readable output.
- Windows resources (icon, version, company name, manifest) come from the template's `build.rs`
  (winres) and the generated icon set.
- Desktop builds stage `assets/` and `assets.manifest.json` next to the executable, so a direct
  `winia build` output can load assets without `run`/`package`. macOS app bundles place the same
  files under `Contents/Resources`; the runtime detects that layout.

### 3.3 Targets and cross-compilation

| Approach | Support |
| --- | --- |
| Native builds | Windows / macOS / Linux each build on their own host |
| Linux → Windows | Not implemented; build on Windows |
| Windows → Linux | Not implemented; build on Linux (WSL works) |
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

### 3.5 `winia clean`

```bash
winia clean
winia clean --dry-run
winia clean --all --json
```

- Removes `target/`, `dist/`, `android/app/build/`, and `android/build/` under the project root.
- `--all` additionally removes the project-local `android/.gradle/` cache.
- Paths that resolve outside the project root, or symlinks, are skipped.
- `--dry-run` reports what would be removed; `--json` returns per-path status, sizes, and
  `total_bytes`.
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

```bash
winia test
winia test --filter text_field
winia test --features winia/debug-server
winia test --target android --device 192.0.0.172:5555
winia test --json
```

- Host: runs `cargo test` in the project root; `--filter` becomes the Cargo test filter,
  `--release` selects the release profile, and `--features` is merged with `build.features`.
- UI integration tests run when the project's test targets require `winia/debug-server` and that
  feature is passed to `cargo test`.
- Android smoke: builds the APK, installs it, launches the activity, checks the app PID's logcat for
  panics, and writes `target/winia-test/android-smoke.png`.
- `--json` reports `{target, passed, failed, ignored, success, duration_ms}`; the Android form also
  includes `device` and `screenshot` (there is no `exit_code` field).
- `--update-snapshots` sets `WINIA_UPDATE_SNAPSHOTS=1`; snapshot diff/baseline handling is still
  owned by the project's test suite, not this command.

### 4.4 Device tools

```bash
winia devices
winia devices --json
winia logcat --device 192.0.0.172:5555 --level W
winia logcat --filter Winia
winia logcat --filter com.example.app --clear
```

- `winia devices` runs `adb devices -l` and prints SERIAL/STATE/MODEL/PRODUCT; `--json` emits
  `{ "devices": [{ "serial", "state", "model", "product", "transport_id" }] }`. Offline devices are
  listed, not hidden.
- `winia logcat` streams `adb logcat -v time` for the selected device (the only online device when
  `--device` is omitted). `--filter` is a tag, or a package name when it contains a dot, in which
  case the pid is resolved once through `adb shell pidof`; `--pid` takes precedence.
  `--level` accepts V/D/I/W/E/F/S, `--clear` empties the buffer first.
- Ctrl+C (exit 130/143 or a signalled child) ends the stream without an error.

```bash
winia emulators list
winia emulators start Pixel_7_API_34
winia emulators start Pixel_7_API_34 --headless
winia emulators stop Pixel_7_API_34
```

- `list` reads `emulator -list-avds` and cross-checks adb, so a running AVD shows its serial.
- `start` spawns the emulator detached and prints its pid; `--headless` adds
  `-no-window -no-audio -no-boot-anim` for CI.
- `stop` maps the AVD name to its emulator serial through `adb emu avd name` and sends
  `emu kill`. An AVD that is not running is an explicit error.

```bash
winia screenshot --output shot.png
winia screenshot --device 192.0.0.172:5555 --json
```

- `winia screenshot` captures `adb exec-out screencap -p` for the selected device, validates the
  PNG header, and reports the image size. It needs no debug server, so it works for any installed
  build; `winia inspect screenshot` stays the choice when the frame should come from the renderer.
- Adb output is drained from reader threads, so binary payloads larger than the pipe buffer do not
  deadlock.

### 4.5 Debugging tools

```bash
winia inspect info
winia inspect tree
winia inspect click 120 80
winia inspect drag 100 100 300 200 --steps 8
winia inspect key Return
winia inspect scroll 120
winia inspect screenshot --out inspect.png
winia inspect discover
```

- `--port <port>` selects the debug server (default 9998); `--json` emits compact output.
- `--timeout <ms>` sets the discovery/connection budget (default 500); once connected, socket I/O
  uses at least 30 seconds so large trees and screenshots can complete.
- `discover` reads `target/winia-dev/inspect-port-<pid>.json` files written by `winia dev` and probes
  only those ports. Pass `--scan` to scan `--port` through `--port + 100` instead.
- `info` returns app, version, id, target, platform, arch, pid, and port; `tree` returns the layout
  tree JSON. `drag x1 y1 x2 y2` sends pointer down, interpolated moves, and pointer up; use
  `--steps` and `--delay-ms` to control the gesture.
- `winia screenshot`: Android device screenshots only; desktop screenshots are not implemented.
- `winia trace` is not implemented yet.
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
winia package --target windows --release --format zip
winia package --target linux --format tar.gz
winia package --target macos --format app,zip
winia package --target android --format apk,aab --split per-abi

Implemented formats: Windows `zip`, `nsis`, `msi`; Linux `tar.gz`, `deb`; macOS `app`, `zip`,
`dmg`; Android `apk`, `aab`.
Installer formats still pending: MSIX, PKG, rpm, AppImage.
```

### 5.2 General requirements

- The version comes from `app.version` in `winia.toml`; Android's `versionCode` is maintained separately.
- Artifact naming template: `{app}-{version}-{os}-{arch}.{ext}`.
- The output directory defaults to `dist/`; `--out-dir` overrides it.
- Portable desktop artifacts include the executable and the project `assets/` directory when present.
- The build report includes each artifact's size, so accidental bloat is easy to spot.
- Every run writes `dist/SHA256SUMS` in the `sha256sum -c` format and includes each file
  artifact's `sha256` in `--json`. Directory artifacts (the macOS `.app` bundle) are not hashed
  yet. Installer formats whose external tool is missing (NSIS, WiX) are skipped with a note unless
  they were requested explicitly with `--format`, in which case they fail.

### 5.3 Windows packaging

Implemented: `zip` portable archive containing the executable and `assets/`, an `nsis`
installer (`{app}-{version}-windows-{arch}-setup.exe`) and an `msi` package
(`{app}-{version}-windows-{arch}.msi`). The executable embeds the generated `.ico` through
`build.rs`/`winres`.

- MSI: generated with the WiX 3 toolset (`candle` + `light`, found through `WIX`, `PATH`, or
  `%ProgramFiles(x86)%\WiX Toolset v3.14\bin`). The package installs per machine into
  `ProgramFiles64Folder` (`ProgramFilesFolder` for 32-bit payloads), ships one component per payload file with an auto-generated GUID, a
  Start Menu shortcut, and a MajorUpgrade rule. `ProductCode` is derived from the app id plus
  version; `UpgradeCode` is derived from the app id so upgrades chain correctly.

- Icons: multi-size `.ico` (16–256).
- Version resources: product name, company name, copyright, file version.
- NSIS: implemented with the NSIS defaults kept — `RequestExecutionLevel` is not overridden, so the
  installer requires administrator rights and installs per machine into `$PROGRAMFILES64\<app>`
  (64-bit payloads). Unicode and zlib compression are NSIS defaults too.
  Writes the Add/Remove Programs key under `HKLM` (`DisplayName`, `DisplayVersion`, `Publisher`,
  `DisplayIcon`, `InstallLocation`, `UninstallString`, `QuietUninstallString`, `NoModify`,
  `NoRepair`, `EstimatedSize`), creates a Start Menu shortcut for all users, and ships an
  `Uninstall.exe` that removes exactly the installed payload plus the registry key.
  Both the install and the uninstall sections run `taskkill /F /IM <exe>` before touching files,
  because a running copy locks its executable; reinstalling or uninstalling over a running app
  therefore succeeds instead of failing on a locked file.
  The generated script lives under `target/winia-package/`; `makensis` is required
  (`winget install NSIS.NSIS`, or `apt-get install nsis`) and is located through
  `MAKENSIS`/`NSISDIR`, `PATH`, the NSIS install directory, or its uninstall registry entry.
- MSI: WiX project, suitable for enterprise distribution.
- MSIX: optional; requires signing and a certificate.
- Dependencies: VC++ runtime detection or static-linking guidance.

### 5.4 macOS packaging

Implemented: `.app` bundle (`Contents/MacOS`, `Contents/Resources`, `Info.plist`), `zip`, and
`dmg` (hdiutil, UDZO). `dmg` reuses an existing bundle when there is one, so signing the app
before packaging keeps the disk image signed. Notarisation and PKG are pending.

- `.app`: `Info.plist`, bundle id, minimum system version, LSUIElement (tray applications).
- Icons: `.icns`.
- Universal binary: `--arch universal` builds x86_64 + arm64.
- DMG: drag-to-install layout, background image, volume label.
- PKG: install scripts, permissions, signing.
- Notarisation: `notarytool` submission, `stapler` stapling, `spctl` verification.

### 5.5 Linux packaging

Implemented: `tar.gz` portable directory and a `deb` package
(`{app}-{version}-linux-{arch}.deb`).

- `tar.gz`: portable directory.
- `deb`: assembled directly as an ar archive (`debian-binary`, `control.tar.gz`, `data.tar.gz`),
  without dpkg-deb. The payload installs under `/usr/lib/<exe stem>`, with a
  `/usr/bin/<exe stem>` symlink and a desktop entry in `/usr/share/applications`. The control
  file carries `Package` (from `app.id`), `Version`, `Architecture` (`amd64`/`arm64`),
  `Maintainer`, `Installed-Size`, `Depends`, `Section` and `Priority`. CI installs the package with
  `dpkg -i`, runs the binary, and removes it with `dpkg -r`.
- `rpm`: spec file, suitable for Fedora/RHEL.
- `AppImage`: single file with bundled dependencies.
- AppStream metadata is not implemented yet.
- The deb control file declares `Depends: libc6, libx11-6, libxkbcommon0, libfontconfig1`.

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

Implemented:

- `winia sign --target windows` signs every `.exe`/`.msi`/`.dll` in `dist/`, or the paths given
  with `--file` (comma-separated) or `--apk`. Signing happens in place, so the published artifact
  is the signed one.
- Credentials come from the environment: `WINIA_WINDOWS_CERT` (PFX) with
  `WINIA_WINDOWS_CERT_PASSWORD`, or `WINIA_WINDOWS_CERT_SHA1` for a certificate in the current
  user's store. `--keystore` also accepts a PFX path.
- Timestamp server defaults to `http://timestamp.digicert.com` and can be overridden with
  `WINIA_WINDOWS_TIMESTAMP`. Digest and RFC 3161 timestamp both use SHA-256.
- A PFX password has to reach `signtool` on its command line; Windows offers no environment or
  stdin channel for it. The CLI masks it in `--dry-run` and in error output, but a local process
  listing can still see it while `signtool` runs. Use `WINIA_WINDOWS_CERT_SHA1` (certificate
  store) when that matters.
- `signtool` is located through `WINIA_SIGNTOOL`, `PATH`, then the newest Windows SDK under
  `%ProgramFiles(x86)%\Windows Kits\10\bin<version>\x64`.
- After signing the command runs `signtool verify /pa`. A locally untrusted chain (self-signed test
  certificate, missing root) is reported as a note, not a failure.
- `--dry-run` prints the exact command. `winia release` signs desktop artifacts automatically when
  a Windows certificate is configured, and warns when it is not.

### 6.2 macOS

Implemented:

- `winia sign --target macos` signs every `.app` in `dist/` (or `--file` entries) with
  `codesign --force --deep`.
- Identity comes from `WINIA_MACOS_IDENTITY`; the default is an ad-hoc signature (`-`), which
  passes `codesign --verify --deep --strict` but is not distributable.
- With a real identity the command adds `--options runtime` and `--timestamp`; entitlements are
  read from `WINIA_MACOS_ENTITLEMENTS` when set.
- `release` signs the bundle before building the dmg when an identity is configured, so the image
  never contains an unsigned app.
- The command refuses to run off macOS with an explicit message.

Pending: `notarytool` notarisation, `stapler` stapling, `spctl --assess` verification, and PKG.

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
winia release --target windows --bump patch --track github
```

Implemented flow (Android and the desktop targets). With no `--target`, a project containing
`android/` releases Android; a desktop-only project releases the host:

1. Refuse a dirty worktree unless `--allow-dirty`.
2. Validate the signing environment and, for GitHub, `gh auth status` plus a git remote before
   modifying the repository.
3. Bump `app.version`, `android.version_code`, and the `Cargo.toml` package version (or
   `[workspace.package].version` when it is inherited); commit `chore(release): vX.Y.Z`.
4. Generate notes from conventional commits since the previous tag.
5. Package every requested target through `winia package`, using that target's
   `[package.<platform>].formats`. Installer formats whose tool is missing are dropped from the
   plan and the run, so a release does not fail because WiX or NSIS is absent.
6. Sign the APK when Android and a keystore are configured, and Windows installers when a
   certificate is configured (the portable zip is re-sealed with the signed executable). macOS
   signing is deferred until after the bundle is built.
7. Create the annotated tag unless `--no-tag`.
8. Push HEAD and the tag when `--push` is given, or automatically with `--track github`.
9. With `--track github`, upload the artifacts to a GitHub release (the tag must exist remotely).
10. Rewrite `dist/SHA256SUMS` for exactly the published files, then write `dist/latest.json`
    with the version, tag (`null` with `--no-tag`), and artifact sizes (including the checksum file).

`--json` prints `{ targets, version, tag, pushed, published, artifacts, notes, duration_ms }`.

Not implemented yet: Play track uploads, macOS signing/notarisation, rollback after a failed build,
and hashes inside `latest.json` (they live in `SHA256SUMS`).

## 7. Environment and toolchain

### 7.1 `winia doctor`

Checks:

- Rust: `rustc`, `cargo`, MSRV.
- Rust targets: `rustup target list --installed`.
- Packaging tools: `makensis` and WiX (Windows), `gh` for publishing. All are optional warnings
  with the exact install command, because packaging skips a format whose tool is absent. `dpkg-deb`
  is not needed: the deb backend writes the ar archive itself.
- `winia dependency`: reads `Cargo.toml` next to `winia.toml` and reports whether `winia` is
  declared (all targets, desktop and/or android), whether a `path` dependency still exists and
  points at the crate rather than a workspace manifest, whether `android/` exists without the
  Android dependency table, and whether a pinned `version` matches this CLI. Missing declarations
  and broken paths are failures; the rest are warnings.
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

## 8. Assets and icons

### 8.1 Runtime assets

```toml
[assets]
include = ["assets/**"]
exclude = ["**/*.psd"]
```

- `winia build` and `winia package` collect matching files into `assets.manifest.json`.
- Each entry records a stable `id`, logical path, packaged path, size, SHA-256, and optional
  density. Density variants use `logo@2x.png` or `2x/logo.png`; the `id` strips the marker
  (`assets/icons/logo.png`).
- A density token is any positive multiple of `x`: `@1.5x`, `@2x`, `2x/`, `0.75x/`. When a file
  carries both a directory segment and an `@` suffix, the suffix wins.
- Runtime `Image::asset` resolves the `id` against the window density: exact match, then the
  smallest density at or above the target (up-scaling slightly beats down-scaling), then the
  largest available density.
- The decode cache is keyed by `(id, chosen density)`, so 1x and 2x decodes of the same asset do
  not collide; `resolved_density()` reports which variant a lookup would pick.
- Desktop staging copies assets next to the executable; Android stages them under
  `android/app/src/main/assets/winia-assets/` so Gradle packs them into the APK/AAB.
- Staging verifies every copied file against its manifest hash.
- Runtime loading uses `Image::asset(logical_path)`; desktop `run`/`dev` stage the bundle under
  `target/winia/assets/desktop`, while Android reads `winia-assets/` through `AssetManager`.
- Asset loads verify the manifest's byte count and SHA-256; paths are canonicalized through the
  manifest so equivalent spellings share one decode-cache entry.
- The desktop window icon is loaded from the manifest's `window_icon` entry and set through
  `WindowAttributes::window_icon`.
- `winia clean` also removes the generated Android asset staging directory.

```bash
winia assets icon assets/icon.png
```

- `winia build` and `winia package` generate icons automatically from `app.icon`; `winia assets icon`
  regenerates them explicitly.
- From a PNG or SVG source, generate:
  - Windows `.ico` (16, 24, 32, 48, 64, 128, 256).
  - macOS `.icns`.
  - Linux hicolor multi-size PNGs.
  - Android mipmap densities (`ic_launcher`, `ic_launcher_round`) plus adaptive foreground and
    `mipmap-anydpi-v26` XML.
- Generate the 512×512 Play Store icon.
- Generated files live under `target/winia/icons/`; Android uses a Gradle res source directory.
- Icon validation warns about sources below 512 px and artwork touching the canvas edge, and
  rejects fully transparent artwork.
- Windows executables embed the generated icon through the template `build.rs` and `winres` (requires
  the Windows SDK `rc.exe`; a missing tool emits a build warning instead of failing).
- The Android manifest references `@mipmap/ic_launcher` and `@mipmap/ic_launcher_round`; when
  `app.icon` is absent, a built-in default icon is generated.
- macOS bundle signing is implemented (`winia sign --target macos`); notarisation and stapling
  are not.

## 9. CI/CD

```bash
winia ci init --provider github --platforms windows,macos,linux,android
```

Implemented: `winia ci init` writes `.github/workflows/ci.yml` (or `.gitlab-ci.yml`), refusing to
overwrite an existing file unless `--force` is passed. `winia ci` without a subcommand runs `init`
with the defaults.

- Platform defaults: windows, macOS, Linux, plus Android when the project has an `android/` directory.
- GitHub: one job per desktop platform plus an Android job. Each job installs Winia Forge,
  runs `cargo fmt`/`clippy`/`test`, packages with `winia package --release`, and uploads `dist/`.
  The Android job installs JDK 21, the Android SDK packages, and packaging uses the built-in NDK driver.
- GitLab: a `.desktop` template extended per platform, plus an Android job.
- The CLI is installed from `--winia-source` (default
  `https://github.com/GrounzerLiu/winia-forge`) because it is not published on crates.io yet.
- Not generated yet: signing/notarisation steps, sccache, and Gradle caching.

## 10. Extension mechanisms

### 10.1 hooks

```toml
[hooks]
before_build = ["echo building"]
after_build = []
before_package = []
after_package = []
```

Implemented:

- Run before/after the corresponding phase; a failure aborts the run.
- The working directory is the project root.
- The environment contains `WINIA_TARGET`, `WINIA_PROFILE`, `WINIA_VERSION`.
- Commands run through `cmd /C` on Windows and `sh -c` elsewhere, in declaration order.
- Hook output goes to stderr so `--json` on stdout stays parseable; the command line is echoed
  before it runs.
- `build` (and therefore `run`, `dev`, `package`, `release`) fires `before_build`/`after_build`
  once per target; `package` fires `before_package`/`after_package` once per package run.

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

There is no `winia upgrade` command, deliberately. Generated projects use `winia = "0.1"`;
updating means changing that requirement and letting cargo resolve it (`cargo update`). The part a
command would add — codemods for breaking
API changes — needs versioned migration data and a compatibility policy that do not exist yet, and
a half-working upgrade is worse than none.

When the framework starts shipping versioned releases, the intended shape is:

- Check the project's `winia` dependency against the CLI version and the `winia.toml` schema.
- Update the dependency declaration in `Cargo.toml`.
- Run codemods for breaking changes, with a backup or branch first.
- Print what changed and what still needs manual work.

## 12. Current scaffold implementation status

| Feature | Status |
| --- | --- |
| clap command surface | Implemented; covers every command in this document |
| `winia new` blank template | Implemented; desktop + Android GameActivity scaffold |
| `winia init` | Implemented; Cargo dependency, `winia.toml`, optional Android scaffold |
| `winia doctor` environment checks | Implemented; covers MSRV, host platform toolchains, Android SDK/NDK/licenses, `winia.toml` validation, `--strict` and `--fix` |
| `winia info` / `targets` | Implemented |
| `winia config show/get` | Implemented |
| Shell completions | Implemented |
| `winia build` | Implemented: host executable and Android APK |
| `winia run` | Implemented: host and Android |
| `winia package` | Implemented: desktop portable zip/tar.gz/app, Windows NSIS and MSI, Linux deb, macOS dmg; Android APK/AAB with alignment/ABI/signature checks; PKG/rpm/AppImage pending |
| `winia sign` | Implemented: Android APK signing with apksigner, Windows Authenticode via signtool, macOS codesign |
| `winia release` | Implemented: multi-target bump/package/sign/tag/SHA256SUMS/latest.json and optional GitHub upload |
| Project hooks | Implemented: `before_build`/`after_build` per target, `before_package`/`after_package` per run |
| Template system | Implemented: the embedded `blank` template; other built-in templates planned |
| External subcommands / plugin API | Planned |
