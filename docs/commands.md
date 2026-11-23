# Commands

Every command runs as `cargo project-tools <command>`. Apart from `init` and the [`self-*` commands](#managing-cargo-project-tools-itself), it works from any folder inside the project: it looks for `project-tools.toml` in the current folder and its parents. The `self-*` commands work from any folder.

| Command | What it does |
| --- | --- |
| `init` | Create `project-tools.toml` and `.project-tools/` |
| `add <crate>[@ver]` | Add a tool; `-i` to choose features and versions on a [setup screen](#interactive-setup) |
| `remove <crate>` | Remove a tool, its lock, build dir and binaries |
| `update <crate> [deps…]` | Update dependencies in the tool's lock, or move the tool to another version (`--latest`, `--version`); `-i` for the [setup screen](#interactive-setup) |
| `install [crates…]` | Audit, then build and install; skipped when nothing changed |
| `check [crates…]` | Check what `install` checks before it builds (targets, audit, build review); builds and records nothing |
| `review [crates…]` | Review the build-time code `check` reports, on the review screen or with `--auto-approve`; builds nothing |
| `verify [crates…]` | Check installed binaries against their recorded checksums; `--repair` rebuilds |
| `audit [crates…]` | Check every lock against the RustSec advisory database, apply `audit-ignore.toml`, suggest fixes |
| `audit-db update` | Fetch or revalidate the advisory database |
| `list` | Show tools and their state |
| `inspect <crate>[@ver]` | Show what a tool builds with its features, and what each other feature would add, without a terminal |
| `self-update [<version>]` | Install a release of cargo-project-tools and make it active |
| `self-use <version>` | Make an installed version active |
| `self-list` | List installed versions |
| `self-available` | List the latest releases |
| `self-check` | Tell whether a newer version is installed or released |

`--no-sfw` works with every command and turns off [sfw](configuration.md#sfw) for that run.

`--message-format json` also works with every command. See [JSON output](#json-output).

## init

Creates `project-tools.toml` with default build settings, and `.project-tools/` with its `.gitignore`.

It also sets the [`targets`](configuration.md#targets) tools are reviewed, audited and installed for. It offers this machine and the GitHub-hosted runners your `.github/workflows` use (`ubuntu-*`, `macos-*`, `windows-*`, found anywhere in the files, matrix lists included), and asks which to keep: numbers or triples, `all` for every target, or Enter for every suggestion. Without a terminal it takes every suggestion.

```bash
cargo project-tools init
cargo project-tools init --target aarch64-apple-darwin --target x86_64-unknown-linux-gnu
```

| Option | Meaning |
| --- | --- |
| `--target <triple>` | Set the targets without asking (repeatable) |

An existing `project-tools.toml` is left as it is.

## add

```bash
cargo project-tools add cargo-nextest
cargo project-tools add cargo-nextest@0.9.80
cargo project-tools add my-tool --git https://github.com/org/my-tool --tag v1.2.0
cargo project-tools add -i cargo-nextest
```

| Option | Meaning |
| --- | --- |
| `--git <url>` | Install from a git repository (https only) |
| `--tag`, `--rev`, `--branch` | Which commit of `--git` to use |
| `-F`, `--features <list>` | Features to enable, comma-separated |
| `--no-default-features` | Turn off default features |
| `--bin <name>` | Binaries to install (repeatable; default: all) |
| `-i`, `--interactive` | Choose the version, features and dependency versions on the [setup screen](#interactive-setup) first |
| `--fix-advisories` | Before the build review, move each dependency with advisories to the lowest semver-compatible version that fixes them all; not with `-i` |
| `--auto-approve` | Record the tool's build scripts, proc macros and cargo config in [`build-review.toml`](build-review.md) |

Copies the lock the tool ships with into `.project-tools/crates/<tool>/` and adds a `[tool.<name>]` entry. A tool that ships no lock gets one from `cargo generate-lockfile`, which only reads the index: nothing is compiled and no build script runs.

Before anything is written, it [audits](#audit) what the lock builds and reports the advisories, with a `fix` for each, without failing on them (`check` and `install` do). With `--fix-advisories` it first moves every dependency it can to a semver-compatible fix, and lists what it couldn't, so the review below covers the lock that's kept. Then it [reviews the build-time code](build-review.md) the lock brings in. In a terminal it opens the [review screen](build-review.md#reviewing-on-the-terminal). Code that's still undecided after that, or outside a terminal, gets an `action = "pending"` entry in `build-review.toml`: the tool and its lock are written anyway, and `add` then fails saying how many pieces are waiting. Decide them with [`review`](#review); `check` and `install` refuse pending entries until then.

## remove

Removes the `[tool.<name>]` entry, the tool's lock folder, its build directory and its binaries.

## update

```bash
cargo project-tools update cargo-nextest
cargo project-tools update cargo-nextest rustls
cargo project-tools update cargo-nextest rustls@0.23.10 --precise 0.23.18
cargo project-tools update cargo-nextest --latest
cargo project-tools update cargo-nextest --version 0.9.90
cargo project-tools update cargo-nextest --no-default-features -F default-no-update
cargo project-tools update -i cargo-nextest
```

| Option | Meaning |
| --- | --- |
| `[deps…]` | Dependencies to update (`name` or `name@version`); all when omitted, unless the tool moves or only features change. A `name` locked at more than one version fails before anything is updated, listing the `name@version` to use instead |
| `--precise <version>` | Update a single dependency to exactly this version |
| `--recursive` | Also update the dependencies of the given dependencies |
| `--latest` | Move the tool to its latest stable version; a git tool follows its tag or branch again |
| `--version <req>` | Move a registry tool to the version matching this requirement |
| `-i`, `--interactive` | Change the version, features and dependency versions on the [setup screen](#interactive-setup); not with the options above |
| `--fix-advisories` | After the update, before the build review, move each dependency with advisories to the lowest semver-compatible version that fixes them all; not with `-i` |
| `--auto-approve` | Record new or changed build-time code in [`build-review.toml`](build-review.md) |

Runs `cargo update` against the tool's lock. Then it audits the result, as `add` does, and with `--fix-advisories` moves what it can to semver-compatible fixes. The lock is only saved if cargo resolves it. Build-time code without a decision gets a pending entry, as for `add`, and `update` fails saying how many pieces are waiting; decide them with [`review`](#review).

With `--latest` or `--version`, the tool moves first. It starts from the new release's lock, then reapplies every dependency you had bumped past it, as long as cargo still resolves. If the new release ships no lock, it starts from yours instead: cargo keeps every pinned version that still fits and only resolves what the new version needs. Dependencies you name are updated after that. The new version (and commit, for git) is written to `project-tools.toml`.

With `-F`, `--no-default-features` or `--default-features`, the build review covers what the new features build, and `features` and `default-features` are written to `project-tools.toml` once it passes. Without dependencies named, the lock isn't updated: only the features change. Use [`inspect`](#inspect) first to see what each feature adds.

`upgrade <crate> [--version <req>]` still works and does the same as `update <crate> --latest` or `--version`, with a note that it's deprecated. It will be removed in a future major release.

## Interactive setup

`add -i` and `update -i` open a setup screen before the [build review](build-review.md#reviewing-on-the-terminal), so you can see what a tool pulls in and change it first. `add -i` starts from the newest release (or `@version`) and `-F`/`--no-default-features`. `update -i` starts from the tool's version, features and lock as they are.

| Part | What it does |
| --- | --- |
| Tool | The tool's version; Enter lists the others, and picking one moves the tool there, keeping your dependency bumps where they still resolve. A git tool follows its branch or tag again. |
| Features | `default` and each feature as a checkbox (`[·]` is on through `default`). Space or a click turns one on or off, and the tree and advisories follow. |
| Dependencies | What cargo builds for those features and your [`targets`](configuration.md#targets), as a foldable tree. With several targets, **All targets** holds what every one of them builds, and **Only `<target>`** holds just the way to what that target adds. Grey packages are in All targets, and shown only on that way. `build` marks a build dependency, `⚠` a vulnerability, `!` a warning (unmaintained, unsound, yanked), and `(*)` a package already shown higher up in its group. |
| Details | For a package: its advisories, and after Enter, why it's built (which features pull it in) and its versions. The lowest version that fixes its advisories is marked `★`, counting warnings such as unsound ones when the advisory names a fix; Enter on a version moves the package there, if cargo can resolve it. A moved package or tool shows `(was …)` in the tree, and its versions list marks the `original` one. Pre-releases are labelled `pre-release`. An advisory from the RustSec database has a `[view]` link (or press `v`) that opens it in a dialog: its fields and its full text, where `←`/`→` move between the package's advisories and Esc closes. |
| Title bar | How many packages are built, and how many have open vulnerabilities and warnings. |
| Buttons | Tree, Details, Next finding (also `n`), Done, Cancel. |

The keys are the review screen's: `Ctrl-w` and `h`, `l` or `j` move to the tree, the details or the buttons; `j`/`k`/`h`/`l` move and fold; `Ctrl-c` cancels. The details and the advisory dialog scroll with the mouse wheel and `PgUp`/`PgDn`. Dragging over their text selects it and copies it to the clipboard through the terminal (OSC 52, which works over ssh, and in tmux with `set-clipboard on`), and `y` copies it again. Shift- or Option-drag still gives the terminal's own selection, which spans the whole screen. Advisories already ignored in the tool's `audit-ignore.toml` are shown as ignored and not counted.

For example, cargo-nextest's default features include `self-update`. Turn off `default`, turn on `default-no-update`, and the tree no longer has what `self-update` pulls in.

**Done** goes on to the build review with exactly that setup, then saves the lock, and writes `version`, `features` and `default-features` (and `rev` for git) to `project-tools.toml`. **Cancel** saves nothing. The screen needs a terminal; without one, use [`inspect`](#inspect). Messages from moving the tool or a dependency, such as dependency bumps kept or dropped, are printed once the screen closes.

## install

```bash
cargo project-tools install
cargo project-tools install cargo-nextest --force
```

First it [audits](#audit) the lock of every selected tool. If any of them has a vulnerability that isn't ignored, nothing is built. Then it builds each tool with `cargo install --path … --locked` and installs it into `.project-tools/bin`. A tool whose version, lock, build settings and targets haven't changed since the last install, and whose binaries still match their checksums, is skipped. `CARGO_BUILD_TARGET` and `--config build.target=…` are refused, because only `--target` and `[build].target` are checked against the tool's targets. What it was built from is recorded in `.project-tools/bin/.stamps`, so restoring `.project-tools/bin` alone is enough to skip it.

The tool's dependencies are vendored first with `cargo vendor --locked` into `.project-tools/targets/<tool>/vendor`, and the build reads them from there instead of cargo's shared caches. Cargo checks each one against the lock, and nothing is compiled while vendoring. Then every build script, proc macro and cargo config the build would use must be in the tool's [`build-review.toml`](build-review.md), or nothing is built; overrides and strips from that file are applied to the vendored copies.

A `rust-toolchain` or `rust-toolchain.toml` in the tool's source is removed before building, as `cargo install <crate>` ignores it too: the tool is built with your toolchain. Cargo always runs from the project root, so config and toolchain files are looked up from there, never from inside a tool's source.

After building, it writes the sha256 of each of the tool's binaries to `.project-tools/bin/<tool>.sha256`, in `sha256sum` format, for [`verify`](#verify).

To check all this without building, use [`check`](#check).

| Option | Meaning |
| --- | --- |
| `--force` | Rebuild even when nothing changed |
| `--auto-approve` | Record new or changed build-time code in [`build-review.toml`](build-review.md); reviews even when the tool is up to date |
| `--approve-target` | Add this machine, and `--target`, to the tool's [`targets`](configuration.md#targets) when they aren't listed |
| `--no-audit` | Skip the audit |
| `--deny-warnings` | Also refuse to build on warnings that aren't ignored |
| `--profile <name>`, `--debug` | Override `[build].profile` |
| `--target <triple>` | Override `[build].target` |
| `--config <value>` | Extra cargo `--config` values |
| `--sandbox <mode>` | The [OS sandbox](configuration.md#sandbox) for this run, `auto`, `require` or `off`, whatever `[build].sandbox` and `[build.sandbox-os]` say |
| `--offline` | Build only from what [`check`](#check) prepared, with no network and nothing from `CARGO_HOME` but the toolchain; a tool that isn't up to date and wasn't prepared fails. Also uses the advisory database as it is (pair it with `--no-audit` after `check`), and doesn't use sfw |
| `-j`, `--frozen`, `-q`, `-v`, `--color`, `--keep-going`, `--timings` | Passed through to cargo |

## check

```bash
cargo project-tools check
cargo project-tools check cargo-nextest --target x86_64-pc-windows-msvc
```

Checks what [`install`](#install) checks before it builds, then stops: this machine (and `--target`) against [`targets`](configuration.md#targets), the audit, and, for each tool that isn't up to date, its build-time code against `build-review.toml`. That still vendors the dependencies, but it builds nothing, and writes nothing to `project-tools.toml` or `build-review.toml`, and no binaries or install stamps (only the audit's `bin/.stamps/<tool>.built.json`). A tool whose review passes keeps its source and vendored dependencies in `.project-tools/targets/<tool>` (with `prepared.json`), and `install` builds from them once without fetching again, as long as the lock, settings, targets and review are the same. See [offline builds](ci.md#offline-builds). It reports every problem rather than stopping at the first, and fails if there are any. Use it in CI to fail fast before a long build, or in a job that only audits.

It never approves anything, so it has no `--auto-approve`, `--approve-target` or `--force`.

| Option | Meaning |
| --- | --- |
| `--no-audit` | Skip the audit |
| `--deny-warnings` | Also fail on warnings that aren't ignored |
| `--target <triple>`, `--config <value>`, `--profile <name>`, `--debug`, `--offline` | As for `install`: what would be built, and so what is checked |
| `--sandbox <mode>` | As for `install`. `check` probes the [sandbox](configuration.md#sandbox) for every tool, up to date or not: `require` without one fails, `auto` writes `sandbox-missing` |

When build-time code isn't recorded, the error names [`review`](#review).

## review

```bash
cargo project-tools review
cargo project-tools review cargo-nextest --auto-approve
cargo project-tools review cargo-nextest --approve zstd-sys@2.1.2+zstd.1.5.7=9e41… --strip ring@0.17.8
cargo project-tools review cargo-nextest --from /tmp/other-branch-build-review.toml
```

Runs each selected tool's [build review](build-review.md) as `install` would, then stops before building. In a terminal it opens the [review screen](build-review.md#reviewing-on-the-terminal); with `--auto-approve` it records everything new, changed or pending at once; with `--approve`, `--strip`, `--override` or `--from` it records just those pieces; elsewhere it lists what's waiting. What's left gets a `pending` entry, and the command fails listing it. A tool that is up to date (its install stamp matches, binaries and all) has nothing to review. It doesn't check targets or audit; `check` and `install` do. Afterwards, commit `build-review.toml` (and `overrides/`).

| Option | Meaning |
| --- | --- |
| `--auto-approve` | Record new or changed build-time code in [`build-review.toml`](build-review.md) |
| `--approve <spec>` | Record the pending build-time code of `package@version`, or a cargo config by its path, as `approve`. Add `=<hash>` (the `sha256`, or `checksum` for a whole crate) to make sure it's the code you read: a different hash fails before anything is written. Each `review-pending` record has a ready `spec`. Needs one tool; repeatable |
| `--strip <spec>` | Record the pending build script (or cargo config) as `strip`, as for `--approve` |
| `--override <spec>` | Record the pending build script (or cargo config) as `override`, with the file already at `overrides/<package>/<path>` (`overrides/cargo-config/<path>`) in the tool's crate directory, as for `--approve` |
| `--from <file>` | Carry over the decisions in another `build-review.toml` (another branch's, say) for pieces with exactly the same kind, package, path, hash and crate. Overrides and pieces two files decide differently aren't carried over. Repeatable |
| `--diff` | Show how each changed piece differs from the version its entry was recorded for, as a unified diff of the files its hash covers, and record nothing |
| `--offline` | Vendor without the network |
| `--config <value>` | Extra cargo `--config` values, as for `install` |

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
| `--profile <name>`, `--debug`, `--target <triple>`, `--config <value>`, `-j`, `--offline`, `--sandbox <mode>`, … | The build options of `install`. Use the ones you installed with, or the rebuild won't match (the sandbox doesn't change the binary) |

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

Only packages the tool actually builds are checked. A lock lists every optional dependency, whatever the features, so `audit` asks `cargo tree` what the tool's [`features` and `default-features`](configuration.md#toolname) build: normal and build dependencies, for the tool's [`targets`](configuration.md#targets) (named as `targets: …` in the output) or every target when none are set. That resolves features without compiling anything or running build scripts, but cargo downloads the dependencies to do it. The answer is kept in `.project-tools/bin/.stamps/<tool>.built.json`, so this only happens again when the lock, the features or the targets change. The rest is reported as `skipped N packages not built with these features`, and an ignore for one of them becomes unused.

Then it:

- drops advisories listed in the tool's [`audit-ignore.toml`](configuration.md#audit-ignore),
- warns about ignores that no longer match anything, and about expired ones,
- prints an `update … --precise …` command for each vulnerability that has a patched version, or `update <tool> --latest` when the tool itself is affected.

Advisories aren't filtered by the operating system or architecture they name, since that metadata is often missing; where it's there, the record's `affected` shows it. When one can't affect the tool's [`targets`](configuration.md#targets), for example a Windows-only bug in a tool you build only for Linux and macOS, ignore it in `audit-ignore.toml` with that as the reason, and look at it again when you add a target.

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

## inspect

```bash
cargo project-tools inspect cargo-nextest
cargo project-tools inspect typos-cli@1.40.0 --no-default-features -F unstable-schema
cargo project-tools inspect cargo-nextest --message-format json
```

Shows what the setup screen shows, as text or as a [JSON](#json-output) `inspect` record, so a script or an AI agent can choose features without a terminal. It records nothing.

- For a configured tool, it uses the tool's lock, features and [`targets`](configuration.md#targets). `-F` and `--no-default-features` try other features instead.
- For any other crate (or a configured one with `@version` or `--git`), it starts from the lock `add` would start from, with `-F`, `--no-default-features` and `--git`/`--branch`/`--tag`/`--rev` as for `add`.

Either way it checks out into a temporary directory under `.project-tools/targets/.tmp`, never the one `install` builds from.

It reports:

- the tool's features and what each enables;
- the packages built with the chosen features, and their advisories (ones in the tool's `audit-ignore.toml` are marked ignored);
- the build scripts, proc macros and build-time libraries among them, and whether the tool's `build-review.toml` has an entry for each (build-time libraries are worked out with every feature on here, so this can list more than a review would);
- for each feature that isn't on, not even through the ones selected, what turning it on adds: packages, advisories, build scripts, proc macros and build-time libraries. With the default features on, it also shows what `default-features = false` removes; with them off, what turning them on adds.

Each feature costs one `cargo tree` run. Like `audit`, it lets cargo download the dependencies to read their manifests, and compiles nothing.

In JSON, the record has `tool`, `tool_version`, `features`, `default_features`, `selected`, `targets`, `source` (the tool's checked-out source in `.project-tools/targets/.inspect/<tool>/src`, kept until the next `inspect` of that tool, whose `Cargo.toml` and README say what the features are for), `bins` (`name`, `required_features`, and `built`: whether these features build it), `packages` (`name`, `version`), `findings` (`package`, `version`, `type`, `id`, `title`, `patched`, `ignored`, `fix`, the command `audit` would suggest, for a vulnerability or yanked version that isn't ignored, and `affected`, as in an `advisory` record), `build_time` (`type`, `package`, `version`, `recorded`) and `feature_changes`. Each entry of `feature_changes` has `feature`, `enable`, and the `packages`, `findings`, `build_time` and `bins` it adds. When `enable` is `false` (only for `default`), they are what it removes, so a binary listed there is one turning off the defaults would drop.

## JSON output

With `--message-format json`, stdout holds only JSON records, one per line. All human text, and cargo's own output, goes to stderr instead. `add`, `update`, `install`, `check`, `review`, `audit` and `inspect` write these records:

| `kind` | When | Fields |
| --- | --- | --- |
| `review-pending` | Build-time code has no decision in [`build-review.toml`](build-review.md): none recorded, or `pending` | `tool`, `file`, `line`, `item`, `spec`, `entry` |
| `review-unused` | An entry in `build-review.toml` matches nothing the tool builds | `tool`, `file`, `line`, `type`, `package`, `path` |
| `target-missing` | This machine or `--target` isn't in the tool's [`targets`](configuration.md#targets) | `tool`, `file`, `line`, `targets` |
| `sandbox-missing` | The sandbox mode is `auto` and there's no working [sandbox](configuration.md#sandbox), so the tool builds without one | `tool`, `file`, `line` (the `sandbox` or `sandbox-os` setting that applied; left out for the default and `--sandbox`), `message` |
| `built` | `install` built a tool | `tool`, `tool_version`, `sandbox` (`sandbox-exec`, `bwrap` or `none`), `bins` |
| `up-to-date` | `install` or `check` found a tool's binaries current (stamp and checksums match), so it builds nothing | `tool`, `tool_version`, `bins` |
| `advisory` | A finding of the [audit](#audit) that isn't ignored | `tool`, `file`, `line`, `id`, `type`, `package`, `package_version`, `title`, `patched`, `severity`, `fix`, `affected` |
| `lock-change` | `add` or `update` changed the tool's lock: `add` lists what `--fix-advisories` or the setup screen changed in the published lock | `tool`, `file`, `package`, `from`, `to` (`from` left out for an added package, `to` for a removed one) |
| `ignore-unused` | An ignore that matched nothing | `tool`, `file`, `line`, `id` |
| `ignore-expired` | An ignore past its `expires` date | `tool`, `file`, `line`, `id`, `expires` |
| `inspect` | What [`inspect`](#inspect) found | See [inspect](#inspect) |
| `ambiguous-dependency` | `update` was given a bare name that is locked at more than one version | `tool`, `package`, `candidates` (each `name@version`) |
| `sfw-missing` | `settings.sfw = "auto"` found no `sfw`, so downloads aren't filtered; written when a command that may download starts, whether or not it downloads | `message` |
| `error` | The command failed | `message` |
| `summary` | The last record of `add`, `update`, `install`, `check`, `review` and `audit` | `ok`, `vulnerabilities`, `warnings`, `pending_reviews`, `missing_targets` |

Every record also has `"version": 1`. The version goes up when a record changes in a way that breaks readers.

Other fields:

- `file` is the file to change, relative to the project root: `build-review.toml`, `project-tools.toml`, the tool's `Cargo.lock`, or the ignore file.
- `line` is the 1-based line in `file` where it is: the package's `[[package]]` in `Cargo.lock` for an `advisory`, the entry in `build-review.toml` for `review-unused`, and for `review-pending` its `pending` entry or the entry it replaces, the `[[ignore]]` entry for `ignore-*`, and the `targets` line for `target-missing`. It's left out when there is no such spot, for example a new piece that `install` or `check` reports without writing a pending entry, an advisory against cargo-project-tools itself, or a file that doesn't parse.
- `item` describes the piece of build-time code:
  - `type` is `build-script`, `proc-macro`, `build-dep` or `cargo-config`;
  - `package`, `version` and `path` are left out when they don't apply;
  - `sha256` is given for build scripts and cargo config, `checksum` for proc macros and build-time libraries;
  - a build script also has its crate's `checksum`, or `commit` for a git package;
  - `location` is the file or directory to read;
  - `previous` is the entry it replaces: `action`, `sha256` or `checksum`, the old crate's `checksum` or `commit`, and `changed`, the keys whose values changed. A build script with only `checksum` or `commit` in `changed` is the same script in a different crate. When the version the entry was recorded for could be fetched, `version` is that version and `location` its copy of the build script or crate, to compare with;
  - `known` (`tool`, `action`) is a decision another tool records for it.
- `spec` is what `review --approve`, `--strip` and `--override` take for the piece: `package@version=hash`, or `path=hash` for cargo config.
- `entry` is the TOML for `file`, with the action it would get, for deciding by hand.
- In an `advisory`:
  - `package_version` is the package's version (not `version`, which is the schema's);
  - `type` is `vulnerability`, `unmaintained`, `unsound`, `notice`, `yanked` or `git-source`;
  - `severity` is `error` for a vulnerability and `warning` otherwise. With `--deny-warnings`, warnings fail the run too, and `summary.ok` says so;
  - `fix` is the command to run, when one exists;
  - `affected` is what the advisory limits itself to, when the database says: `os` and `arch` (Rust's names, like `windows`) and `functions` (paths like `crate::module::fn`). It's left out when the database names none, which is common, so a missing `affected` doesn't mean every platform is affected.

```json
{"version":1,"kind":"target-missing","tool":"cargo-deny","file":"project-tools.toml","line":4,"targets":["x86_64-unknown-linux-gnu"]}
{"version":1,"kind":"summary","ok":false,"vulnerabilities":0,"warnings":0,"pending_reviews":0,"missing_targets":1}
```

Control characters, DEL, C1 controls, and characters that hide or reorder text (bidi controls, zero-width and other invisible characters, line and paragraph separators) are written as `\uXXXX` escapes, so `cat`ing the records can't drive a terminal. The values themselves aren't changed. Check a value before you use it in a command or a CI annotation, above all one that comes from someone else:

- From cargo-project-tools: `kind`, `version`, `tool`, `file` and `line`. A tool name is a crate name, checked when the tool is added, and `file` is always one of the paths above, built from it.
- From the project's own files: `targets` in `target-missing`, and `id` and `expires` in `ignore-unused` and `ignore-expired`.
- From the locks, the tools' sources and the advisory database, which you don't control: everything else. That covers `package`, `package_version`, `item` (its `path` and `location` among them), `entry`, an advisory's `id`, `title` and `patched`, `fix`, `message`, and everything in an `inspect` record except `tool`.

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
