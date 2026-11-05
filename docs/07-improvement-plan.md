# 07 · Improvement plan

> Status: proposed, 2026-11-04.
> Scope: the follow-ups identified after [06-android-ime-and-apk.md](06-android-ime-and-apk.md) landed,
> the `winia doctor` audit, and the configuration/ABI review.

## 0. Context and decisions

Document 06 fixes the verified Android baseline (Gradle 9.5.0, AGP 9.3.2, JDK 21, NDK 27.0.12077973,
LLVM 18.1.8, games-activity 4.4.0) and the two-stage build: a native `.so` is produced first, copied
into `jniLibs`, and assembled into a GameActivity APK by Gradle.

Three decisions frame this plan:

1. **`winia.toml` stays the single source of truth** for project and packaging identity.
   `Cargo.toml` keeps crate concerns (crate type, features, dependencies) and Gradle files keep
   Gradle concerns. Android settings are not moved into `[package.metadata.android]`.
2. **`cargo-ndk` is not a dependency.** The CLI implements the NDK driver itself: per-ABI
   environment variables, `cargo build --target`, copy into `jniLibs`, alignment verification.
   A probe cdylib built for `aarch64-linux-android` with NDK environment variables only produced
   `libndk_probe.so` with every LOAD segment at `0x4000` in 3.5 s; `cargo-ndk` was not installed.
3. **`cargo-apk2` is optional.** The GameActivity path requires Gradle and cargo-apk2 cannot assemble
   an IME-capable APK. It may remain as a `--pure-rust` convenience path, never as the default.

## 1. P0 — doctor correctness

The command is implemented, but its Android scope is incomplete. Five changes, all inside
`crates/winia-cli/src/commands/doctor.rs`:

| Item | Current gap | Change | Acceptance |
| --- | --- | --- | --- |
| LLVM in the Android scope | `llvm_check` lives in `windows_checks()`, so `--target android` never checks libclang, although skia-bindings needs it | Move LLVM to the common checks; detect the default install path; when libclang exists but `LIBCLANG_PATH` is unset, report a warning with the exact command | `doctor --target android` prints an LLVM row; a host with LLVM installed but `LIBCLANG_PATH` unset gets an actionable warning |
| NDK baseline preference | The suggestion takes the lexicographically last installed NDK (30.x) | Accept `ANDROID_NDK_ROOT`; prefer the documented `27.0.12077973`, otherwise the highest installed version | The report suggests 27.x when present and explains why |
| JDK version | Only checks that `JAVA_HOME` names a directory | Parse `java -version` (stderr) and compare with 17 minimum / 21 baseline | JDK 8 is reported as missing with an upgrade hint |
| Gradle and AGP versions | Not checked | Read `gradle-wrapper.properties` and `app/build.gradle.kts` when `android/` exists; report the versions; a missing project is a warning | The report states wrapper 9.5.0 / AGP 9.3.2, or "no Android project" |
| 16 KB alignment status | Nothing is reported | State that builds inject `max-page-size=16384`; warn when the existing `RUSTFLAGS` contains a conflicting value | Matches 06 §6.3 |

## 2. P1 — Android toolchain

### 2.1 One ABI model

ABI knowledge currently exists in three places: `AndroidConfig::validate` (hard-coded match),
`doctor::abi_triples` (mapping table), and the `docs/03` table. Adding an ABI means editing all
three, and drift is silent.

Introduce `crates/winia-cli/src/platform/android.rs`:

```rust
pub enum Abi { Arm64V8a, ArmeabiV7a, X86_64, X86 }

impl Abi {
    pub fn parse(raw: &str) -> anyhow::Result<Self>;
    pub fn as_str(self) -> &'static str;        // arm64-v8a
    pub fn rust_triple(self) -> &'static str;   // aarch64-linux-android
    pub fn jni_libs_dir(self) -> &'static str;  // arm64-v8a
    pub fn is_legacy_32bit(self) -> bool;
}
```

| Android ABI | Rust target | jniLibs directory | Notes |
| --- | --- | --- | --- |
| `arm64-v8a` | `aarch64-linux-android` | `arm64-v8a` | Default; real devices |
| `x86_64` | `x86_64-linux-android` | `x86_64` | Default; emulators |
| `armeabi-v7a` | `armv7-linux-androideabi` | `armeabi-v7a` | Legacy 32-bit, opt-in |
| `x86` | `i686-linux-android` | `x86` | Legacy 32-bit, opt-in |

`AndroidConfig::validate`, `doctor`, and later `build`/`run`/`package` all use this type.

### 2.2 Built-in NDK driver

`winia build --target android` performs, for each selected ABI:

1. Verify the Rust target is installed; otherwise fail with the exact `rustup target add` command.
2. Select the NDK wrapper for `min_sdk` (for example `aarch64-linux-android26-clang[.cmd]`).
3. Export `CARGO_TARGET_<TRIPLE>_LINKER`, `CC_*`, `AR_*`, `ANDROID_NDK`, `LIBCLANG_PATH`,
   `BINDGEN_EXTRA_CLANG_ARGS`, and merged `RUSTFLAGS` (append `max-page-size=16384` instead of
   overwriting the user value).
4. Run `cargo build --target <triple> --lib`.
5. Copy `lib<app>.so` into `android/app/src/main/jniLibs/<abi>/`.
6. Verify every LOAD segment with `llvm-readelf -l`; fail on any alignment other than `0x4000`.

ABIs build sequentially by default because Cargo locks `target/`. Parallel builds would need one
`CARGO_TARGET_DIR` per ABI and are out of scope for the first version.

`--abi` overrides `[android].abis` for one invocation (replace, never merge) and accepts `all`
with a warning about build time and disk usage.

### 2.3 Project template and sync

`winia new --platforms android` must generate:

- `android/settings.gradle.kts` with repositories only in `pluginManagement` and
  `dependencyResolutionManagement`.
- `android/build.gradle.kts` pinning AGP 9.3.2.
- `android/app/build.gradle.kts` with `namespace`, `compileSdk 36`, `minSdk 26`, `targetSdk 36`,
  the `games-activity 4.4.0` pin, explicit `appcompat` and `core`, and the Kotlin stdlib exclusions.
- `android/gradle/wrapper/` plus `gradlew`/`gradlew.bat` for Gradle 9.5.0.
- `android/app/src/main/AndroidManifest.xml` with `configChanges` and
  `windowSoftInputMode="adjustResize"`.
- `android/app/src/main/java/<package>/MainActivity.java` extending `GameActivity`.
- `src/lib.rs` with `#[no_mangle] fn android_main(...)` calling `winia::app::run_app_android`, and
  `Cargo.toml` with the Android feature selection described in 06.

`winia android sync` rewrites only managed files and preserves user code. Managed files carry a
generated header so users know where to edit.

### 2.4 Packaging and running

| Command | Work | Acceptance |
| --- | --- | --- |
| `winia package --target android` | Universal APK (`...-android-universal.apk`), per-ABI split (`...-android-{abi}.apk`), `zipalign -P 16`, `apksigner verify`, `unzip -l` check for the `.so` | Artifacts are named per ABI and contain the expected libraries |
| `winia run --target android` | Read `ro.product.cpu.abi` from the device, build or pick only the matching ABI | Emulator and device work without editing the configuration |

### 2.5 End-to-end acceptance

```bash
winia new demo --platforms android
cd demo
winia build --target android
./android/gradlew -p android assembleDebug
adb install -r android/app/build/outputs/apk/debug/app-debug.apk
```

The app launches, and a TextField accepts input through ADB Keyboard. Every packaged library reports
16 KB alignment.

## 3. P2 — configuration and architecture

### 3.1 Configuration boundary

| Setting | Home | Reason |
| --- | --- | --- |
| App identity, SDK levels, activity mode, ABIs, version code | `winia.toml` `[android]` | Shared by `config`, `doctor`, `build`, `release` |
| Permissions, intent filters, services, metadata | `AndroidManifest.xml` | Gradle merges library manifests; entries carry attributes a config key cannot express |
| `crate-type`, features, dependencies | `Cargo.toml` | Cargo owns these |
| NDK and JDK pins | Optional `[android]` fields | `build`/`doctor` need to locate the toolchain |
| Gradle and AGP versions | Gradle wrapper and version catalog | Gradle owns these; duplicating them invites drift |
| Linker, `CC`/`AR`, `RUSTFLAGS`, `LIBCLANG_PATH` | Computed per build | 06 shows `.cargo/config.toml` values can be overridden by wrapper tools |
| Gradle dependencies, packaging, signing | `android/app/build.gradle.kts` | Gradle extension points |

### 3.2 Desktop architectures

- Add `--arch x86_64|aarch64` (default: host) to `build` and `package`.
- macOS `universal` builds both architectures and runs `lipo`; it is only available on macOS.
- Windows `aarch64` requires a Skia prebuilt for that target; fail explicitly when unavailable.
- Linux `aarch64` requires a cross toolchain (zigbuild/cross); `doctor` should say so.

### 3.3 Honest target reporting

`winia targets` currently prints `aarch64` for Windows and Linux although no `--arch` flag exists.
Either implement the flag or mark those architectures as planned.

### 3.4 Scaffold debts

- ~~`winia new --template` is accepted and silently ignored.~~ Fixed: an unknown template is now an
  error (`unknown template '<name>'; the only built-in template is 'blank'`). Remote templates are
  still unimplemented.
- `winia new --force` is still documented in the roadmap but missing from the CLI.
- ~~The generated project lacks `assets/` and `src/lib.rs`.~~ Fixed: `new` creates `assets/` and
  writes `src/lib.rs` (the Android entry point; also present for desktop builds so `cargo test`
  sees the shared UI).
- The generated project still has no `rust-toolchain.toml` or `.cargo/config.toml`.
- ~~`winia platform add android` writes into the current directory instead of the project root
  found through `winia.toml`.~~ Fixed: it now delegates to `android init`, which resolves the
  project root and refuses to run outside one.

## 4. P3 — documentation and tests

| Area | Work |
| --- | --- |
| `02-feature-spec.md` | §1.3 template contents (Gradle files, `lib.rs`), §3.3 two-stage build without a mandatory cargo-ndk, §7.1 doctor checks (LLVM, JDK, Gradle) |
| `03-android-support.md` | Done: §8 is now a status table (implemented / partial / pending) covering winit, entry point, Skia, clipboard, insets, IME, lifecycle, input, debug channel, assets, accessibility |
| `05-roadmap.md` | Update P3/P4 status, replace the "winit Android IME missing" risk with AGP/Gradle cache poisoning, libclang/NDK mismatch, and silently ignored 16 KB flags |
| `06-android-ime-and-apk.md` | Use ``` fences for consistency with the other documents |
| Tests | `Abi` parsing and mapping unit tests; doctor scenarios with fake NDK/LLVM/Gradle layouts; an integration test for `winia new`; help snapshot tests |

## 5. Non-goals

- No hard dependency on `cargo-ndk`.
- No requirement for `cargo-apk2`; it stays an optional convenience path.
- `[package.metadata.android]` never becomes the canonical configuration.
- All ABIs are never built by default.

## 6. Execution order

1. P0 doctor changes (one file, verifiable on the spot).
2. ABI model, then the built-in NDK driver (native-library loop first, Gradle second).
3. Template and `android sync`.
4. Packaging and running.
5. Configuration/architecture cleanup and documentation.

## 7. Verification

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace

winia doctor
winia doctor --target android
winia doctor --target android --json --fix
```

The Android loop is only considered verified when the generated project produces an installable APK
whose native libraries are 16 KB aligned and whose TextField receives text from ADB Keyboard.
