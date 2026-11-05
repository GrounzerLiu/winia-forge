# 01 · Naming decision

> Date: 2026-11-03
> Status: final, but a crates.io and trademark check is still required before release.

## 1. Final naming

| Layer | Name |
| --- | --- |
| Product/toolchain brand | **Winia Forge** |
| CLI command | **`winia`** |
| Rust crate (binary package) | **`winia-cli`** |
| Project configuration file | **`winia.toml`** |
| build.rs helper library | **`winia-build`** |
| Cross-platform packaging library | **`winia-pack`** |
| Optional cargo compatibility layer | **`cargo-winia`** (enables `cargo winia ...`, not published yet) |

How the command line reads:

```bash
cargo install winia-cli

winia new my-app
cd my-app
winia run
winia build --release
winia package --format msi,nsis
winia doctor
```

## 2. Why these names

1. **The command shares the framework name.** Users only have to remember one word. Flutter, Dioxus, and
   Bevy all follow this pattern: the framework name is the command name.
2. **The product name carries the toolchain identity.** `Winia Forge` reads like a real product rather than
   "the winia command-line tool". Forge emphasises "craft, build, package", which matches the toolchain's job.
3. **Crate name and command name are separate.** The framework library stays `winia`; the CLI is published as
   `winia-cli` and installs the `winia` command. This avoids a clash with the library crate.
4. **A single source of truth for configuration.** `winia.toml` serves the CLI, build.rs, and the packager at
   once: the application name, version, icon, packaging formats, and signing configuration are written only once.
5. **The toolchain layers are extensible.** winia-build generates resources and winia-pack packages; the CLI
   only orchestrates. Adding Android/iOS later will not entangle the desktop logic.

## 3. Alternative names

| Name | Assessment |
| --- | --- |
| `winforge` | Good semantics (forge = craft), but the "win" prefix is easily misread as Windows-only, which conflicts with the cross-platform positioning. Fine as an internal codename, not recommended as the primary command. |
| `winia-tool` | Neutral and conservative, suitable for an internal tool; the public command should still be `winia`. |
| `wbuild` / `wpack` | Too narrow; they would no longer fit once templates, publishing, and upgrades are added. |
| `winia-dev` | Only fits a development-tool positioning; the tool also packages and publishes, so the name would limit its scope. |

**Not recommended:**

- `winx`: a Windows API crate with that name already exists on crates.io.
- `winkit`: a Win32 wrapper crate already exists.
- `winio`: a Rust Windows I/O crate already exists; the spelling and pronunciation are too close.
- `winara`: no clear semantics; sounds like a home-appliance brand.
- `winandroid` / `winpack-android`: puts the platform in the tool name; every new platform would need another
  command, which does not scale.

## 4. Availability check

Queried crates.io on 2026-11-03:

- `winia`, `winia-cli`, `cargo-winia`, and `winforge` have no same-named crate (the API returned 404;
  confirm once more before publishing).
- `winx` and `winkit` are taken.

Brand level: `Winia` itself is a Korean home-appliance brand (winia.com), so public product searches will
collide somewhat on SEO. If this becomes an independent commercial product, use **Winia Forge** as the
toolchain brand externally while keeping `winia` as the command name; whether to rename the framework itself
is a larger branding decision outside the scope of this tool.

## 5. Command naming principles

### 5.1 target and format are separate

- **Platform** is `--target`: `windows`, `macos`, `linux`, `android`, `ios`.
- **Artifact** is `--format`: `msi`, `nsis`, `dmg`, `deb`, `appimage`, `apk`, `aab`.

Do not write `winia package --android`, and do not write `winia android-package`. The correct form is:

```bash
winia build --target android --release
winia package --target android --format apk,aab
```

Adding iOS later then means `--target ios`, with no new tool entry point.

### 5.2 Platform-specific operations

Only project operations that are genuinely platform-specific get a subcommand group:

```bash
winia android init
winia android sync
winia android studio
```

Device operations stay generic:

```bash
winia devices
winia emulators start Pixel_8_API_36
winia logcat --filter com.example.myapp
winia screenshot --target android
```

### 5.3 Artifact naming

Use one consistent artifact file name:

```text
{app}-{version}-{os}-{arch}.{ext}
```

For example:

```text
my-app-0.1.0-windows-x86_64-setup.exe
my-app-0.1.0-macos-universal.dmg
my-app-0.1.0-linux-x86_64.AppImage
my-app-0.1.0-android-arm64-v8a.apk
```

## 6. Conclusion

- Tool name: **Winia Forge**
- Command: **`winia`**
- Package name: **`winia-cli`**
- Configuration: **`winia.toml`**
- Android is only a target; it does not change the naming.
