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
- [ ] `winia init`.
- [ ] A unified `--json` / `--dry-run` output layer.
- [ ] Snapshot tests for command help.

## 3. P1: desktop MVP

- [ ] `winia build`: call Cargo, support target/profile/features, produce a build report.
- [ ] `winia run`: build and run, forwarding arguments.
- [ ] `winia dev`: file watching, process restart, debug-server.
- [ ] `winia test`: unit tests + UI tests (`--features debug-server`).
- [ ] `winia clean`.
- [ ] Portable artifacts for the three desktop platforms: Windows zip, macOS .app/zip, Linux tar.gz.

Acceptance:

- `winia new` a project on Windows and `winia run` opens a window.
- `winia build --release` produces an executable.
- `winia dev` restarts automatically after a source change.

## 4. P2: desktop release

- [ ] Icon generation (.ico, .icns, PNG).
- [ ] Windows NSIS, MSI (WiX), optional MSIX.
- [ ] macOS DMG, PKG, universal binary.
- [ ] Linux deb, rpm, AppImage, .desktop, AppStream.
- [ ] Authenticode signing, macOS codesign/notarize.
- [ ] `winia release`: version, changelog, tag, GitHub Releases.
- [ ] GitHub Actions three-platform matrix template.

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
- [ ] `winia devices` / `emulators` / `logcat` / `screenshot`.
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

An **Android P0 spike** is the recommended next step:

1. Build a minimal example in `D:\Projects\winia` using winit's `android-game-activity` feature.
2. Manually produce an APK with `cargo ndk` + Gradle.
3. Verify window, GL/softbuffer, and touch on an emulator/device.
4. Write the results back into this roadmap and `03-android-support.md`.

The CLI's Android features should wait until the P0 validation passes; otherwise the platform-layer
uncertainty leaks into the toolchain.
