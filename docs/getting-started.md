# Getting started

Install cargo-project-tools, then add your first tool to a project.

## Install

Download a binary for your platform from the [latest release](https://github.com/osseco/cargo-project-tools/releases/latest), check it, and put it on your `PATH`:

```bash
gh attestation verify cargo-project-tools-x86_64-unknown-linux-musl --repo osseco/cargo-project-tools
```

Every asset is also signed with cosign. See [Verifying a release](verifying-releases.md) for all the checks.

Release binaries:

| Platform | Asset |
| --- | --- |
| macOS, Apple silicon | `cargo-project-tools-aarch64-apple-darwin` |
| Linux, x86_64 | `cargo-project-tools-x86_64-unknown-linux-musl` |
| Linux, arm64 | `cargo-project-tools-aarch64-unknown-linux-musl` |
| Windows, x86_64 | `cargo-project-tools-x86_64-pc-windows-msvc.exe` |

Or build from source:

```bash
cargo install --git https://github.com/osseco/cargo-project-tools --locked
```

Cargo runs it as a subcommand: `cargo project-tools …`.

### Keep it up to date

On macOS and Linux, cargo-project-tools can install its own releases into `~/.cargo-project-tools/bin`, checking the checksum and the cosign signature of each (it needs [cosign](https://docs.sigstore.dev/cosign/system_config/installation/)):

```bash
cargo project-tools self-update
```

Put `~/.cargo-project-tools/bin` first on your `PATH`, then check for new releases now and then:

```bash
cargo project-tools self-check
```

See [the self-* commands](commands.md#managing-cargo-project-tools-itself).

## First tool

In your project's root:

```bash
cargo project-tools init
```

```bash
cargo project-tools add cargo-nextest
```

`init` creates `project-tools.toml` and `.project-tools/`. `add` records the tool and copies the `Cargo.lock` it ships with (or generates one, if it ships none) into `.project-tools/crates/cargo-nextest/`. That lock is now yours.

## Audit and fix

```bash
cargo project-tools audit
```

`audit` checks every lock against the RustSec advisory database, which it downloads to `.project-tools/advisory-db` and keeps up to date. For each vulnerable dependency it suggests an `update` command, for example:

```bash
cargo project-tools update cargo-nextest <dep>@<old> --precise <new>
```

The lock is only saved if cargo resolves it.

## Build the tools

```bash
cargo project-tools install
```

This builds every tool with `--locked` against your lock and puts the binaries in `.project-tools/bin`. Add that folder to your `PATH`. Running it again skips tools whose version, lock and build settings haven't changed.

## Commit

Commit `project-tools.toml` and `.project-tools/crates/`. Everything else under `.project-tools/` is git-ignored. Next: [how it works](how-it-works.md), the [commands](commands.md), and [CI](ci.md).
