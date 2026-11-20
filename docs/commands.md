# Commands

Every command runs as `cargo project-tools <command>`. Apart from `init` and the [`self-*` commands](#managing-cargo-project-tools-itself), it works from any folder inside the project: it looks for `project-tools.toml` in the current folder and its parents. The `self-*` commands work from any folder.

| Command | What it does |
| --- | --- |
| `init` | Create `project-tools.toml` and `.project-tools/` |
| `add <crate>[@ver]` | Add a tool |
| `remove <crate>` | Remove a tool, its lock, build dir and binaries |
| `upgrade <crate>` | Move to a new tool version, keeping your dependency bumps where they still resolve |
| `update <crate> [deps…]` | Update dependencies in the tool's lock |
| `install [crates…]` | Audit, then build and install; skipped when nothing changed |
| `verify [crates…]` | Check installed binaries against their recorded checksums; `--repair` rebuilds |
| `audit [crates…]` | Check every lock against the RustSec advisory database, apply `audit-ignore.toml`, suggest fixes |
| `audit-db update` | Fetch or revalidate the advisory database |
| `list` | Show tools and their state |
| `self-update [<version>]` | Install a release of cargo-project-tools and make it active |
| `self-use <version>` | Make an installed version active |
| `self-list` | List installed versions |
| `self-available` | List the latest releases |
| `self-check` | Tell whether a newer version is installed or released |

`--no-sfw` works with every command and turns off [sfw](configuration.md#sfw) for that run.

## init

Creates `project-tools.toml` with default build settings, and `.project-tools/` with its `.gitignore`.

## add

```bash
cargo project-tools add cargo-nextest
cargo project-tools add cargo-nextest@0.9.80
cargo project-tools add my-tool --git https://github.com/org/my-tool --tag v1.2.0
```

| Option | Meaning |
| --- | --- |
| `--git <url>` | Install from a git repository (https only) |
| `--tag`, `--rev`, `--branch` | Which commit of `--git` to use |
| `-F`, `--features <list>` | Features to enable, comma-separated |
| `--no-default-features` | Turn off default features |
| `--bin <name>` | Binaries to install (repeatable; default: all) |

Copies the lock the tool ships with into `.project-tools/crates/<tool>/` and adds a `[tool.<name>]` entry. A tool that ships no lock gets one from `cargo generate-lockfile`, which only reads the index: nothing is compiled and no build script runs.

## remove

Removes the `[tool.<name>]` entry, the tool's lock folder, its build directory and its binaries.

## upgrade

```bash
cargo project-tools upgrade cargo-nextest
cargo project-tools upgrade cargo-nextest --version 0.9.90
```

Moves to the latest stable version, or the one matching `--version`. Starts from the new release's lock, then reapplies every dependency you had bumped past it, as long as cargo still resolves. If the new release ships no lock, it starts from yours instead: cargo keeps every pinned version that still fits and only resolves what the new version needs. A git tool follows its tag or branch again.

## update

```bash
cargo project-tools update cargo-nextest
cargo project-tools update cargo-nextest rustls
cargo project-tools update cargo-nextest rustls@0.23.10 --precise 0.23.18
```

| Option | Meaning |
| --- | --- |
| `[deps…]` | Dependencies to update (`name` or `name@version`); all when omitted |
| `--precise <version>` | Update a single dependency to exactly this version |
| `--recursive` | Also update the dependencies of the given dependencies |

Runs `cargo update` against the tool's lock. The lock is only saved if cargo resolves it.

## install

```bash
cargo project-tools install
cargo project-tools install cargo-nextest --force
```

First it [audits](#audit) the lock of every selected tool. If any of them has a vulnerability that isn't ignored, nothing is built. Then it builds each tool with `cargo install --path … --locked` and installs it into `.project-tools/bin`. A tool whose version, lock and build settings haven't changed since the last install, and whose binaries still match their checksums, is skipped. What it was built from is recorded in `.project-tools/bin/.stamps`, so restoring `.project-tools/bin` alone is enough to skip it.

A `rust-toolchain` or `rust-toolchain.toml` in the tool's source is removed before building, as `cargo install <crate>` ignores it too: the tool is built with your toolchain. Cargo always runs from the project root, so config and toolchain files are looked up from there, never from inside a tool's source.

After building, it writes the sha256 of each of the tool's binaries to `.project-tools/bin/<tool>.sha256`, in `sha256sum` format, for [`verify`](#verify).

| Option | Meaning |
| --- | --- |
| `--force` | Rebuild even when nothing changed |
| `--no-audit` | Skip the audit |
| `--deny-warnings` | Also refuse to build on warnings that aren't ignored |
| `--profile <name>`, `--debug` | Override `[build].profile` |
| `--target <triple>` | Override `[build].target` |
| `--config <value>` | Extra cargo `--config` values |
| `-j`, `--offline`, `--frozen`, `-q`, `-v`, `--color`, `--keep-going`, `--timings` | Passed through to cargo |

## verify

```bash
cargo project-tools verify
cargo project-tools verify cargo-nextest --repair
```

Checks each selected tool's binaries in `.project-tools/bin` against the `<tool>.sha256` that `install` wrote. It fails when a binary is missing or differs, or when a tool has no checksum file. It needs only `.project-tools/bin` and reads nothing from the network. The files are in `sha256sum` format, so `sha256sum -c *.sha256` inside `bin` checks the same thing without cargo-project-tools.

With `--repair`, every tool that fails is audited like in [`install`](#install), rebuilt, and checked against its recorded checksum. A repair never writes a new checksum. If the rebuild doesn't match, the tool's binaries are removed, so nothing unverified stays on `PATH`, and `verify` fails. A tool without a checksum file can't be repaired; run `install` for it.

The checksum sits next to the binaries. It catches corruption, partial cache restores and accidental changes, not someone who can write to `bin`.

| Option | Meaning |
| --- | --- |
| `--repair` | Rebuild tools that fail, and require the rebuild to match |
| `--no-audit`, `--deny-warnings` | As for `install`; only with `--repair` |
| `--profile <name>`, `--debug`, `--target <triple>`, `--config <value>`, `-j`, `--offline`, … | The build options of `install`. Use the ones you installed with, or the rebuild won't match |

### When a repair can match

| State | `install` | `verify --repair` |
| --- | --- | --- |
| `bin` gone, `targets/<tool>` present | Cargo copies the binary it already built, and the same checksum is written again | Fails: no checksum file. Run `install` |
| Binary missing or changed, `targets/<tool>` present | Rebuilds the same way and writes the checksum again | Cargo copies the binary it already built, which matches, even for builds that aren't reproducible |
| Binary changed, `targets/<tool>` gone | Full rebuild, new checksum | Full rebuild; matches only if the build is reproducible |

A full rebuild gives the same binary when the toolchain, target, flags and project path are the same, unless a build script embeds the time or similar (crates like `vergen` or `built`). For such a tool, in this order:

1. Keep `.project-tools/targets/<tool>` with `.project-tools/bin`, so a repair copies the binary cargo already built.
2. Make the build reproducible: set `SOURCE_DATE_EPOCH`, which many of these crates honour, and keep toolchain, target and flags the same.
3. Accept the new build on purpose with `install --force`, which records a new checksum. That trusts the new binary; it isn't a repair.

## audit

```bash
cargo project-tools audit
cargo project-tools audit --update-db --deny-warnings
```

Checks every tool's lock against the [RustSec advisory database](https://rustsec.org), the same database cargo-audit uses, and checks every crates.io package against the index for yanked versions. No cargo-audit is needed. It checks:

- crates.io packages, including the tool itself. A vulnerability is an error. An unmaintained, unsound or notice advisory is a warning.
- git packages, and tools installed from git, by name and version. Advisories are written for crates.io releases, so a match only means "possibly affected" and is a `git-source` warning.

Path packages and other registries aren't checked.

Only packages the tool actually builds are checked. A lock lists every optional dependency, whatever the features, so `audit` asks `cargo tree` what the tool's [`features` and `default-features`](configuration.md#toolname) build: normal and build dependencies, for every target. That resolves features without compiling anything or running build scripts, but cargo downloads the dependencies to do it. The rest is reported as `skipped N packages not built with these features`, and an ignore for one of them becomes unused.

Then it:

- drops advisories listed in the tool's [`audit-ignore.toml`](configuration.md#audit-ignore),
- warns about ignores that no longer match anything, and about expired ones,
- prints an `update … --precise …` command for each vulnerability that has a patched version, or `upgrade <tool>` when the tool itself is affected.

It also reports advisories against cargo-project-tools itself, and fails on them unless they're accepted in [`self-audit-ignore.toml`](configuration.md#advisories-against-cargo-project-tools-itself).

| Option | Meaning |
| --- | --- |
| `--update-db` | Revalidate the advisory database first, whatever its age |
| `--deny-warnings` | Fail on unmaintained, unsound, notice, yanked and git-source warnings that aren't ignored |

The database lives in `.project-tools/advisory-db`. It is fetched when missing, and revalidated when it was last checked longer ago than [`advisory-db-max-age`](configuration.md#advisory-db-max-age) (one day by default). Revalidating sends the stored ETag, so an unchanged database isn't downloaded again. If revalidating fails, `audit` warns and uses the copy it has.

## audit-db update

Fetches or revalidates the advisory database without auditing.

## list

Shows every configured tool with its version and source, and flags a missing `Cargo.lock` or an installed build.

## Managing cargo-project-tools itself

The `self-*` commands keep versioned copies of cargo-project-tools under `~/.cargo-project-tools`. Each version is kept as `bin/cargo-project-tools-X.Y.Z`, and `bin/cargo-project-tools` is a symlink to the active one, so put `~/.cargo-project-tools/bin` first on your `PATH`. `CARGO_PROJECT_TOOLS_HOME` replaces `~/.cargo-project-tools`.

Switching versions needs symlinks, so `self-update` and `self-use` work on macOS and Linux only. On Windows, download releases by hand.

### self-update

```bash
cargo project-tools self-update
cargo project-tools self-update 1.2.0
cargo project-tools self-update --no-switch
```

Installs the latest release, or the given version (`X.Y.Z`, `vX.Y.Z` or `latest`), and makes it active. A version that is already installed is only switched to.

Releases come from the `repo` in [`~/.cargo-project-tools/config.toml`](configuration.md#self-update-source). Each download must:

- match its hash in the release's `SHA256SUMS`,
- carry a cosign signature by the configured `signer`,
- report, with `--version`, the version it was downloaded as.

It needs [cosign](https://docs.sigstore.dev/cosign/system_config/installation/) on `PATH`. Downloads don't go through sfw: release assets aren't packages.

| Option | Meaning |
| --- | --- |
| `--no-switch` | Install without making it the active version |

### self-use

```bash
cargo project-tools self-use 1.1.0
```

Points `bin/cargo-project-tools` at an installed version. Nothing is downloaded.

### self-list

Lists the installed versions, newest first, with `*` on the active one. Also prints the repository and signer `self-update` uses.

### self-available

Lists the ten latest releases of the configured repository, newest first, with the publish date, and which are installed or active. Drafts and prereleases are left out. It asks the GitHub API without a token; nothing is downloaded.

### self-check

Compares the active version with the newest one installed and with the latest release, and names the command that switches to it or installs it. It asks the GitHub API without a token; nothing is downloaded. It exits 0 unless the check itself fails.
