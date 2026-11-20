# Build review

Building a tool runs code before the tool itself ever runs: build scripts and proc macros of the tool and its dependencies, and any cargo config the tool's source ships, which can swap `rustc`, add a wrapper or a linker, or set environment variables. `install` builds only when each piece of it is recorded in the tool's `build-review.toml`, as approved, overridden or stripped.

## What is reviewed

| Kind | What | Pinned by |
| --- | --- | --- |
| `build-script` | The `build.rs` (or `package.build`) of the tool and of every dependency it builds | sha256 of the script file; when the script isn't at the package root, of every file in its directory |
| `proc-macro` | Every proc-macro crate it builds | The crate checksum from `Cargo.lock`; for a git package its commit |
| `cargo-config` | `.cargo/config.toml` and `.cargo/config` in the tool's source, from the package directory up to the root of the checkout | sha256 of the file |

Only what the tool builds with its [`features` and `default-features`](configuration.md#toolname) counts, for every target, so one review holds on every platform. That includes Windows-only crates when you build on Linux.

Cargo never reads config from inside dependencies, and a config with `include`, or one that is a link, is refused. A shipped `rust-toolchain` file is always removed instead.

## The file

`.project-tools/crates/<tool>/build-review.toml`, next to the lock. Commit it: it's the record of what you looked at.

```toml
[[build-script]]
package = "libc"
path = "build.rs"
sha256 = "…"
action = "approve"

[[build-script]]
package = "ring"
path = "build.rs"
sha256 = "…"
action = "override"
file = "overrides/ring-build.rs"

[[proc-macro]]
package = "serde_derive"
checksum = "…"
action = "approve"

[[cargo-config]]
path = ".cargo/config.toml"
sha256 = "…"
action = "strip"
```

| `action` | Effect |
| --- | --- |
| `approve` | Build it as shipped |
| `override` | Replace it with `file`, relative to `.project-tools/crates/<tool>/` |
| `strip` | Remove it: a build script is turned off with `build = false`, a config file is deleted. Cargo refuses to strip the build script of a package with `links` |

A proc macro can only be approved. To avoid one, turn off the feature that pulls it in.

`sha256` and `checksum` always describe the shipped content, also for `override` and `strip`, so a change upstream stops the build again and you can check that your override still fits.

## Approving

When something isn't recorded, `add`, `upgrade`, `update` and `install` stop before writing or building, and list each piece with the file to look at:

```text
  build-time code to review:
    build-script zstd-sys@2.1.2+zstd.1.5.7 build.rs (changed; was approve)  .project-tools/targets/cargo-nextest/vendor/zstd-sys-2.1.2+zstd.1.5.7/build.rs
error: 1 piece of build-time code isn't recorded in .project-tools/crates/cargo-nextest/build-review.toml; review it, then rerun with --approve-build
```

After reviewing, run the same command with `--approve-build`. New pieces are recorded as `approve`. A changed piece keeps its action and file, and its entry is updated in place, so comments next to it stay. Entries that no longer match anything are removed. Without the flag they're only reported.

To override or strip something, edit its `action` (and `file`) and run `install`. A changed decision makes `install` rebuild the tool from scratch, since cargo doesn't notice edits to vendored sources.

## Limits

A build script's sha256 covers its own file, or its directory when it isn't at the package root. Code it pulls in from elsewhere in the crate with `include!` or `#[path]` isn't part of it, but the crate as a whole is still checked against the lock.
