# Winia Forge

[![CI](https://github.com/GrounzerLiu/winia-forge/actions/workflows/ci.yml/badge.svg)](https://github.com/GrounzerLiu/winia-forge/actions/workflows/ci.yml)

The official project toolchain for Winia: create, build, run, and package Winia applications.

> Status: active development. Project creation, build/run/dev/test/clean, Android APK/AAB
> packaging and signing, Android/GitHub releases, desktop portable archives, doctor, inspect, and
> asset/icon generation, and a Windows NSIS installer are implemented. MSI/DMG/deb installers and
> Play Store uploads are pending.
> See [docs/05-roadmap.md](docs/05-roadmap.md) for progress.

## Quick start

```bash
cargo run -p winia-cli -- --help
cargo run -p winia-cli -- new my-app --winia-path D:\Projects\winia\winia
cd my-app
cargo run -p winia-cli -- doctor
```

`winia` is not published on crates.io yet, so `--winia-path` (or the `WINIA_PATH` environment
variable) is required when creating a project. It points at the `winia` crate, for example
`D:\Projects\winia\winia`.

After installation the binary is named `winia`:

```bash
cargo install --path crates/winia-cli
winia new my-app --winia-path D:\Projects\winia\winia
```

## Workspace

```text
winia-forge/
├── crates/
│   ├── winia-cli/      # CLI entry point, binary `winia`
│   ├── winia-build/    # build.rs resource generation (placeholder, not implemented)
│   └── winia-pack/     # cross-platform packaging (placeholder, not implemented)
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
- `winia test`: run host cargo tests and an Android install/launch/logcat/screenshot smoke test
- `winia clean`: remove `target/`, `dist/`, and Android build outputs, with `--dry-run`/`--all`
- `winia ci init`: write a ready-to-run GitHub Actions or GitLab CI workflow for the project
- `winia init`: add the Winia dependency and configuration to an existing Cargo project, with optional Android scaffolding
- `winia assets icon`: generate Windows ICO, macOS ICNS, Linux hicolor, Android mipmap/adaptive, and store icons from one PNG/SVG; Windows executables embed the ICO via `build.rs`
- `winia inspect`: query the debug server for app identity, layout trees, input events, screenshots, and port discovery
- `winia package`: assemble portable desktop archives (Windows zip, Linux tar.gz, macOS app/zip), a Windows NSIS installer, and Android APKs/AABs into `dist/`, with asset manifests, Android alignment, ABIs, and debug signatures
- `winia sign`: sign Android APKs with `apksigner`, writing `*-signed.apk` copies and verifying the certificate
- `winia release`: bump the version, package, sign, tag, write `dist/latest.json`, and optionally publish a GitHub release
- `winia doctor`: diagnose the Rust toolchain and MSRV, platform toolchains, the Android SDK/NDK, and `winia.toml` validation
- `winia info` / `winia targets`: environment and target platform information
- `winia config show/get`: read `winia.toml`
- `winia completions`: generate shell completions
- `winia platform add android`: create the Android scaffold placeholder directory

The remaining commands already have a complete clap surface and a documented contract; implementation
follows the roadmap.

## License

Apache-2.0
