# Winia Forge

The official project toolchain for Winia: create, build, run, and package Winia applications.

> Status: scaffold stage. The command surface, configuration model, and design documents are in
> place; most commands currently fail with an explicit "not implemented in this scaffold" message.
> See [docs/05-roadmap.md](docs/05-roadmap.md) for progress.

## Quick start

```bash
cargo run -p winia-cli -- --help
cargo run -p winia-cli -- new my-app
cd my-app
cargo run -p winia-cli -- doctor
```

After installation the binary is named `winia`:

```bash
cargo install --path crates/winia-cli
winia new my-app
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

- `winia new`: generate a minimal Winia application (Cargo package, `winia.toml`, README)
- `winia doctor`: diagnose the Rust toolchain and MSRV, platform toolchains, the Android SDK/NDK, and `winia.toml` validation
- `winia info` / `winia targets`: environment and target platform information
- `winia config show/get`: read `winia.toml`
- `winia completions`: generate shell completions
- `winia platform add android`: create the Android scaffold placeholder directory

The remaining commands already have a complete clap surface and a documented contract; implementation
follows the roadmap.

## License

Apache-2.0
