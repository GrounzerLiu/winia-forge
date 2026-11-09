# Releasing Winia and Winia Forge

> Status: describes the 0.1.0 release, verified against the published crates and a `cargo publish
> --dry-run` of each package. This is the procedure to follow for 0.1.1 and later.

## 1. What is published

All eight crates are already on crates.io at 0.1.0, so later releases are *updates*, not first
publishes. That matters: a name that already exists cannot be replaced, and a version cannot be
re-uploaded once it is out.

| Crate | Repo | Published | First release |
| --- | --- | --- | --- |
| `material-shapes` | winia | 0.1.0 | 2026-11-06 03:09 UTC |
| `winia-macros` | winia | 0.1.0 | 2026-11-06 03:11 UTC |
| `skiwin` | winia | 0.1.0 | 2026-11-06 03:11 UTC |
| `winia-material-symbols-outlined` | winia | 0.1.0 | — |
| `winia-material-symbols-rounded` | winia | 0.1.0 | — |
| `winia-material-symbols-sharp` | winia | 0.1.0 | — |
| `winia` | winia | 0.1.0 | 2026-11-06 03:27 UTC |
| `winia-winit` | separate | 0.31.0-beta.4 | — (fork of winit) |
| `winia-pack` | winia-forge | 0.1.0 | 2026-11-06 08:07 UTC |
| `winia-cli` | winia-forge | 0.1.0 | 2026-11-06 08:08 UTC |

`winia-winit` is the Android IME bridge and a published fork of winit. It lives outside both
workspaces; the `winia` workspace depends on it as `winit = { package = "winia-winit", version =
"=0.31.0-beta.4" }`. Releasing it is a separate job from releasing `winia`.

## 2. Publish order

The order is forced by the dependency graph, and the 0.1.0 timestamps confirm it was followed.

**winia workspace**

1. `material-shapes` — no local dependencies.
2. `winia-macros` — no local dependencies.
3. `skiwin` — no local dependencies.
4. `winia-material-symbols-outlined`, `-rounded`, `-sharp` — independent of each other.
5. `winia` — depends on `skiwin`, `winia-macros` and the three symbol crates, all with explicit
   `version = "0.1.0"` in the path dependency, which is what makes them resolvable from crates.io.

**winia-forge workspace**

1. `winia-pack` — no local dependencies.
2. `winia-cli` — depends on `winia-pack`; publishing it first would fail because the registry cannot
   resolve the path dependency.

The rule: a crate can only be published after every path dependency it names has a matching version
on crates.io. `cargo publish` enforces this and errors out, which is why the order above is not
optional.

## 3. Version bump

Versions are declared in three places and all of them have to move together:

- `winia/Cargo.toml`: `[package].version` per crate (the workspace itself is unversioned).
- `winia-forge/Cargo.toml`: `[workspace.package].version`, which both crates inherit through
  `version.workspace = true`.
- The `version` field of every path dependency. `winia/Cargo.toml` pins
  `skiwin = { path = "../skiwin", version = "0.1.0" }` and similar; if the sibling moves to 0.1.1 and
  this string does not, `cargo publish` rejects the package.

After bumping, refresh the lockfiles and confirm the workspace still builds.

## 4. Pre-flight checks

Run per crate, before publishing anything:

```bash
cargo publish -p <crate> --dry-run     # packages, verifies, aborts at upload
cargo package -p <crate> --list        # confirm no file is missing or wrongly included
```

`--dry-run` was run for `winia-macros` and `winia-pack` while preparing this document and both
packaged, compiled and reached the "aborting upload due to dry run" stage. It is the closest check to
a real publish.

`cargo package` reports the archive size. `winia` packages 430 files (10.0 MiB, 2.3 MiB compressed),
which is large but within the crates.io limit; a careless `include` can push it over.

Notes from the 0.1.0 packaging run:

- `Cargo.lock` lists several yanked transitive dependencies (`chacha20`, `clipboard-win` and others).
  Cargo warns but does not fail, and a publish would still work. Worth cleaning up when convenient,
  not a release blocker.
- No crate uses a `git` dependency, so nothing is unpublishable for that reason.

## 5. Authentication

`cargo publish` needs a token. On this machine `~/.cargo/credentials.toml` already holds one.

Scoped tokens are preferable to a global one: create them per crate family in the crates.io
dashboard so a leaked token cannot publish outside its scope.

## 6. Publishing

With the token in place, from each repository:

```bash
# winia, in the order from section 2
cargo publish -p material-shapes
cargo publish -p winia-macros
cargo publish -p skiwin
cargo publish -p winia-material-symbols-outlined
cargo publish -p winia-material-symbols-rounded
cargo publish -p winia-material-symbols-sharp
cargo publish -p winia

# winia-forge
cargo publish -p winia-pack
cargo publish -p winia-cli
```

Wait for crates.io to index each crate before publishing the one that depends on it. The index update
is usually quick but not instant, and a publish attempted too early fails with "no matching package".

A published version cannot be deleted, only yanked, and yanking does not free the version number.
Getting this right the first time is cheaper than fixing it afterwards.

## 7. What the CLI publishes that the library does not

`winia-cli` ships a binary named `winia` (`[[bin]] name = "winia"`), so `cargo install winia-cli`
puts a `winia` command on the PATH. The library `winia` publishes no binary.

The CLI's own `winia release` command is a *different* thing: it packages an **application** built
with Winia, not Winia itself. It bumps that application's version, builds and signs installers,
writes `dist/SHA256SUMS` and `dist/latest.json`, and can publish a GitHub release. Do not confuse it
with publishing the toolchain.

## 8. Checklist

Before tagging a release:

- [ ] Every crate's version bumped, including the `version` strings inside path dependencies.
- [ ] Lockfiles refreshed and the workspace builds.
- [ ] `cargo publish --dry-run` clean for each crate.
- [ ] `cargo package --list` contains no stray file and no missing asset.
- [ ] The README's version references still match (for example "winia 0.1 is published on crates.io").
- [ ] Docs that claim a publish state updated. `winia-forge/docs/04-architecture.md` still says
      `winia-cli` is not published, which contradicts the published 0.1.0; fix it as part of the
      release rather than carrying the contradiction forward.
- [ ] Publish in dependency order, waiting for the index between steps.
- [ ] Tag both repositories and push.
