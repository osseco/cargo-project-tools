# Commands

Every command runs as `cargo project-tools <command>`. Apart from `init`, it works from any folder inside the project: it looks for `project-tools.toml` in the current folder and its parents.

| Command | What it does |
| --- | --- |
| `init` | Create `project-tools.toml` and `.project-tools/` |
| `add <crate>[@ver]` | Add a tool |
| `remove <crate>` | Remove a tool, its lock, build dir and binaries |
| `upgrade <crate>` | Move to a new tool version, keeping your dependency bumps where they still resolve |
| `update <crate> [deps…]` | Update dependencies in the tool's lock |
| `install [crates…]` | Build and install; skipped when nothing changed |
| `audit [crates…]` | cargo-audit every lock, apply `audit-ignore.toml`, suggest fixes |
| `audit-db update` | Refresh the advisory database |
| `list` | Show tools and their state |

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

Builds each tool with `cargo install --path … --locked` and installs it into `.project-tools/bin`. A tool whose version, lock and build settings haven't changed since the last install is skipped.

| Option | Meaning |
| --- | --- |
| `--force` | Rebuild even when nothing changed |
| `--profile <name>`, `--debug` | Override `[build].profile` |
| `--target <triple>` | Override `[build].target` |
| `--config <value>` | Extra cargo `--config` values |
| `-j`, `--offline`, `--frozen`, `-q`, `-v`, `--color`, `--keep-going`, `--timings` | Passed through to cargo |

## audit

```bash
cargo project-tools audit
cargo project-tools audit --update-db --deny-warnings
```

Runs [cargo-audit](https://crates.io/crates/cargo-audit) (must be on `PATH`) on every tool's lock, and checks every dependency against the index for yanked versions. Then it:

- drops advisories listed in the tool's [`audit-ignore.toml`](configuration.md#audit-ignore),
- warns about ignores that no longer match anything, and about expired ones,
- prints an `update … --precise …` command for each vulnerability that has a patched version.

| Option | Meaning |
| --- | --- |
| `--update-db` | Fetch the advisory database first |
| `--deny-warnings` | Fail on unmaintained, unsound and yanked warnings that aren't ignored |

The database lives in `.project-tools/advisory-db`. It is fetched when missing or with `--update-db`.

## audit-db update

Fetches or refreshes the advisory database without auditing.

## list

Shows every configured tool with its version and source, and flags a missing `Cargo.lock` or an installed build.
