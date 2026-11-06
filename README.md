# Winia Forge

[![CI](https://github.com/GrounzerLiu/winia-forge/actions/workflows/ci.yml/badge.svg)](https://github.com/GrounzerLiu/winia-forge/actions/workflows/ci.yml)

The official project toolchain for Winia: create, build, run, and package Winia applications.

> Status: the command surface is complete. Project creation, build/run/dev/test/clean, Android and
> desktop packaging (zip/tar.gz/app/dmg/NSIS/MSI/deb/APK/AAB), signing on all three desktop platforms
> and Android, releases, doctor, inspect, and asset/icon generation are implemented. Still pending:
> Play Store uploads, macOS notarisation, and the MSIX/PKG/rpm/AppImage formats.
> See [docs/05-roadmap.md](docs/05-roadmap.md) for progress.

## Quick start

```bash
cargo run -p winia-cli -- --help
cargo run -p winia-cli -- new my-app
cd my-app
cargo run -p winia-cli -- doctor
```

`winia` 0.1 is published on crates.io, so generated projects depend on it by default. Pass
`--winia-path` (or set `WINIA_PATH`) only when you want to develop against a local checkout,
for example `D:\Projects\winia\winia`.

Install the published CLI, then the binary is named `winia`:

```bash
cargo install winia-cli
winia new my-app
```

## Workspace

```text
winia-forge/
├── crates/
│   ├── winia-cli/      # CLI entry point, binary `winia`
│   ├── winia-build/    # placeholder; resource generation lives in winia-cli
│   └── winia-pack/     # NSIS, MSI (WiX) and deb generation
└── docs/
    ├── 01-naming.md
    ├── 02-feature-spec.md
    ├── 03-android-support.md
    ├── 04-architecture.md
    ├── 05-roadmap.md
    ├── 06-android-ime-and-apk.md
    └── 07-improvement-plan.md
```

## Documentation

| Document | Contents |
| --- | --- |
| [01-naming.md](docs/01-naming.md) | Naming decision, alternative names, brand check |
| [02-feature-spec.md](docs/02-feature-spec.md) | Full feature list, command model, winia.toml configuration |
| [03-android-support.md](docs/03-android-support.md) | Android support design, project layout, build pipeline, framework gaps |
| [04-architecture.md](docs/04-architecture.md) | Crate layout, implementation conventions, extension points |
| [05-roadmap.md](docs/05-roadmap.md) | P0–P4 roadmap and acceptance criteria |
| [06-android-ime-and-apk.md](docs/06-android-ime-and-apk.md) | GameActivity IME requirements, APK build pitfalls, verification checklist |
| [07-improvement-plan.md](docs/07-improvement-plan.md) | Prioritised follow-up plan: doctor, Android toolchain, ABI model, config boundary, docs |

## Current implementation

Implemented:

- `winia new`: generate a blank Winia application from the embedded template (desktop binary, shared UI, `winia.toml`, and a GameActivity Gradle project when Android is selected)
- `winia build`: build a host executable, or an Android APK through the built-in NDK driver and the project's Gradle wrapper (16 KB alignment is verified)
- `winia run`: launch the host executable, or install and start the APK on the connected Android device
- `winia dev`: watch sources and assets, rebuild and restart the host app or reinstall/restart the Android activity, and auto-start the debug server
- `winia devices` / `winia logcat` / `winia emulators` / `winia screenshot`: list adb devices,
  stream or filter the device log, start/stop emulators, and capture the screen over adb
- `winia test`: run host cargo tests and an Android install/launch/logcat/screenshot smoke test
- `winia clean`: remove `target/`, `dist/`, and Android build outputs, with `--dry-run`/`--all`
- `winia ci init`: write a ready-to-run GitHub Actions or GitLab CI workflow for the project
- `winia android sync`: regenerate the Gradle build files Winia owns (`--check`/`--force`)
- `winia init`: add the Winia dependency and configuration to an existing Cargo project, with optional Android scaffolding
- `winia assets icon`: generate Windows ICO, macOS ICNS, Linux hicolor, Android mipmap/adaptive, and store icons from one PNG/SVG; Windows executables embed the ICO via `build.rs`
- `winia inspect`: query the debug server for app identity, layout trees, input events, screenshots, and port discovery
- `winia package`: assemble portable desktop archives (Windows zip, Linux tar.gz, macOS app/zip/dmg), Windows NSIS and MSI installers, a Linux deb, and Android APKs/AABs into `dist/`, with asset manifests, SHA-256 checksums, Android alignment, ABIs, and debug signatures
- `winia sign --target windows`: Authenticode-sign desktop artifacts with signtool (PFX or store thumbprint)
- `winia sign`: sign Android APKs with `apksigner`, writing `*-signed.apk` copies and verifying the certificate
- `winia release`: bump the version, package any target, sign Android APKs and Windows installers (macOS bundles when an identity is set), tag, write `dist/SHA256SUMS` and `dist/latest.json`, and optionally publish a GitHub release
- `winia doctor`: diagnose the Rust toolchain and MSRV, platform toolchains, the Android SDK/NDK,
  packaging tools, the project's `winia` dependency, and `winia.toml` validation
- `winia info` / `winia targets`: environment and target platform information
- `winia config show/get`: read `winia.toml`
- `winia completions`: generate shell completions
- `winia android init`: add the Gradle project, Android dependency, crate type and entry point to an existing app
- `winia screenshot`: capture the device screen through adb, without the debug server

There is no `winia upgrade`: generated projects use a `winia = "0.1"` requirement, and updating
means changing that requirement and running `cargo update`. Migration codemods do not exist yet. See
[docs/02-feature-spec.md](docs/02-feature-spec.md) §11.

## License

Apache-2.0
