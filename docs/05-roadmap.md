# 05 · Roadmap

> Status: first draft, 2026-11-03.
> Principle: build the smallest end-to-end usable path first, then add platform depth. Do not start with a
> pile of independent half-finished commands.

## 1. Phase overview

| Phase | Goal | Main content | Acceptance |
| --- | --- | --- | --- |
| P0 | Usable scaffold | Workspace, CLI command surface, `winia.toml` model, minimal `winia new` template, `doctor` | `cargo test --workspace` passes; `winia new demo` generates a readable project |
| P1 | Desktop MVP | `build/run/dev/test`, native Windows/macOS/Linux builds, portable artifacts | One runnable application built on each of the three platforms |
| P2 | Desktop release | Icons, zip/NSIS/MSI, .app/DMG, deb/AppImage, signing | Installable artifacts produced and signature-verified |
| P3 | Android platform | winit entry point, GL/softbuffer, touch, Android project, APK/AAB, signing | Hello-world and a release APK running on an emulator/device |
| P4 | Full capability | IME, insets, lifecycle, clipboard, Play publishing, accessibility | TextField accepts input, rotation keeps state, AAB uploadable |

## 2. P0: scaffold (current)

Done:

- [x] Cargo workspace: `winia-cli`, `winia-build`, `winia-pack`.
- [x] clap command surface: new/init/build/run/dev/test/package/sign/release/doctor/add/remove/update/assets/ci/upgrade/clean/info/targets/config/devices/emulators/logcat/screenshot/platform/android/completions.
- [x] `winia.toml` configuration model and defaults.
- [x] Minimal `winia new` template.
- [x] Basic `winia doctor` environment checks.
- [x] `winia info` / `targets` / `config show/get` / `completions`.
- [x] Documentation: naming, features, Android, architecture, roadmap.

Remaining:

- [ ] Real templates (counter, multi-window, navigation, dashboard).
- [x] `winia init`: add Winia to an existing Cargo project.
- [ ] A unified `--json` / `--dry-run` output layer.
- [ ] Snapshot tests for command help.

## 3. P1: desktop MVP

- [ ] `winia build`: call Cargo, support target/profile/features, produce a build report.
- [ ] `winia run`: build and run, forwarding arguments.
- [x] `winia dev`: file watching, process restart, and automatic debug-server.
- [x] `winia test`: host cargo tests + Android install/launch/logcat/screenshot smoke.
- [x] `winia clean`.
- [x] Portable artifacts: Windows zip, macOS .app/zip, Linux tar.gz.
- [x] Windows NSIS installer.

Acceptance:

- `winia new` a project on Windows and `winia run` opens a window.
- `winia build --release` produces an executable.
- `winia dev` restarts automatically after a source change.

## 4. P2: desktop release

- [x] Icon generation (.ico, .icns, PNG, Android mipmaps) and Windows exe icon embedding.
- [x] Windows NSIS installer (`winia package --target windows --format nsis`).
- [x] Windows MSI (WiX).
- [ ] Optional MSIX.
- [x] macOS dmg (hdiutil, UDZO).
- [ ] macOS PKG and universal binary.
- [x] Linux deb (payload in /usr/lib, /usr/bin symlink, desktop entry).
- [ ] Linux rpm, AppImage, AppStream metadata.
- [x] Authenticode signing (`winia sign --target windows`, signtool).
- [x] macOS codesign (`winia sign --target macos`).
- [ ] macOS notarisation (notarytool + stapler).
- [x] `winia release`: version bump, package (Android and desktop), sign Android APKs, tag, checksums, GitHub Releases.
- [x] CI workflow generator (`winia ci init`: GitHub Actions, GitLab CI).

Acceptance:

- Each platform produces an installer that installs and uninstalls correctly.
- Windows signature verification passes; macOS notarisation passes.
- CI can upload artifacts.

## 5. P3: Android platform

### 5.1 Framework layer

- [ ] winit Android entry point: `android_main` + `with_android_app`.
- [ ] Single-window rendering: GL or softbuffer.
- [ ] Touch event path verified.
- [ ] Android target compilation configuration for winia/skiwin.
- [ ] Basic insets (status bar/navigation bar).
- [ ] Basic Activity lifecycle mapping.

### 5.2 Toolchain layer

- [ ] `winia platform add android` / `winia android init`: generate the Gradle project.
- [ ] `winia android sync`: rewrite managed files from `winia.toml`.
- [ ] `winia doctor --target android`: SDK/NDK/JDK/adb checks.
- [ ] `winia build --target android`: cargo ndk + Gradle assembleDebug.
- [ ] `winia run --target android`: adb install/start/logcat.
- [x] `winia devices` / `logcat`.
- [ ] `emulators` / standalone `screenshot` (covered by `winia inspect screenshot`).
- [ ] Debug APK generation and installation.

Acceptance:

- A hello-world interface appears on the emulator and responds to taps.
- Rotating the screen does not crash.
- `winia package --target android --format apk --release` produces an installable APK.

## 6. P4: full capability

- [ ] winit Android IME fork: GameActivity's TextEvent/TextAction → WindowEvent::Ime.
- [ ] TextField can input, select, and copy/paste on Android.
- [ ] Clipboard JNI bridge.
- [ ] AssetManager asset loading.
- [ ] SaveableStateRegistry writes to the Bundle; state is restored after Activity recreation.
- [ ] Permission declarations + runtime requests.
- [ ] AAB, Play App Signing, internal test track upload.
- [ ] TalkBack semantics-tree bridge (can be split out).
- [ ] Emulator automation tests.

Acceptance:

- TextField input works in Chinese and English, and the soft keyboard does not cover the field.
- State is restored after rotation or background kill.
- The AAB can be uploaded to the Google Play internal test track.

## 7. Main risks

| Risk | Impact | Mitigation |
| --- | --- | --- |
| Missing winit Android IME | TextField unusable | Do P0 rendering and touch first; the IME fork is a separate workstream |
| Slow first Skia Android build | Poor developer experience | `doctor` warns; cache Skia build outputs; prebuilt packages |
| 16 KB page requirement | Android 15+ refuses installation | NDK r27+ + `zipalign -P 16` + packaging verification |
| Gradle/NDK version fragmentation | Build failures | `doctor` pins a support matrix; templates lock the Gradle wrapper |
| Clipboard/assets/lifecycle gaps | Abnormal application behaviour | Itemise acceptance in P3/P4; test on real devices |
| Name clash with the appliance brand | Search/trademark risk | Use the Winia Forge brand externally; run a trademark search before release |

## 8. Next steps

The Android P0 spike has passed: GameActivity IME input, the lifecycle fixes, and a device-verified
APK build are recorded in [06-android-ime-and-apk.md](06-android-ime-and-apk.md).

The consolidated follow-up backlog — doctor correctness, the built-in NDK driver, the Android
template, the ABI model, and the documentation updates — lives in
[07-improvement-plan.md](07-improvement-plan.md).
