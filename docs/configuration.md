# Configuration

`project-tools.toml` at the project root, an `audit-ignore.toml` per tool, and optionally a [`self-audit-ignore.toml`](#advisories-against-cargo-project-tools-itself). When the tool edits `project-tools.toml`, your comments stay. Outside the project, `~/.cargo-project-tools/config.toml` sets where [`self-update`](#self-update-source) gets releases.

## project-tools.toml

```toml
[settings]
sfw = "auto"                 # "auto", "require" or "off"
advisory-db-max-age = "1d"   # optional; hours ("12h") or days ("1d")
targets = ["aarch64-apple-darwin", "x86_64-unknown-linux-gnu"]   # optional; all when unset

[build]
profile = "release"
sandbox = "auto"             # "auto", "require" or "off"

[build.sandbox-os]           # optional; per operating system
macos = "require"
windows = "off"

[build.profile-settings]     # applied to every tool
strip = "symbols"
lto = "thin"
codegen-units = 1

[tool.cargo-nextest]
version = "0.9.80"

[tool.cargo-nextest.build.profile-settings]
panic = "unwind"             # per-tool override
```

`add`, `update` and `remove` maintain the `[tool.*]` entries. You edit `[settings]` and `[build]`.

### targets

The target triples tools are [reviewed](build-review.md), audited and installed for. Only build scripts, proc macros and dependencies used on these targets need approval, and `audit` checks only those dependencies. `install` refuses to build on a machine, or with a `--target`, that isn't listed; `--approve-target` adds it here. Outside a terminal, for example in CI, add a runner's target where you review instead, and commit this file. Without `targets`, every target counts.

`[tool.<name>].targets` replaces the list for one tool, for example a tool you only run in Linux CI.

### [build]

| Key | Meaning |
| --- | --- |
| `profile` | Cargo profile to build with |
| `target` | Target triple |
| `rustflags` | List of extra `rustflags` |
| `profile-settings` | Profile keys applied to the build profile |
| `sandbox` | Build in an OS sandbox: `"auto"` (default), `"require"` or `"off"`. See [sandbox](#sandbox) |
| `sandbox-os` | `sandbox` for one operating system: `linux`, `macos`, `windows`, `freebsd`, … |

Allowed `profile-settings` keys: `inherits`, `opt-level`, `debug`, `strip`, `lto`, `codegen-units`, `panic`, `incremental`, `debug-assertions`, `overflow-checks`, `split-debuginfo`, `rpath`.

### [tool.\<name\>]

| Key | Meaning |
| --- | --- |
| `version` | The tool's version |
| `git`, `tag`, `rev`, `branch` | Git source (https only) |
| `features` | Features to enable; `audit` checks only what they build |
| `default-features` | `false` turns off default features; `audit` follows it too |
| `bins` | Binaries to install (default: all) |
| `targets` | Replaces [`[settings].targets`](#targets) for this tool |
| `build` | A `[build]` table for this tool only; its keys override the global ones |

Changing any of these makes `install` rebuild the tool.

## sfw

Package downloads (`add`, `update`, `install`, `audit`) can run through [Socket Firewall Free](https://docs.socket.dev/docs/socket-firewall-free), which blocks known malicious packages.

| `settings.sfw` | Behavior |
| --- | --- |
| `"auto"` (default) | Use `sfw` when it's on `PATH`, otherwise carry on with a note |
| `"require"` | Fail when `sfw` isn't on `PATH` |
| `"off"` | Never use `sfw` |

`--no-sfw` turns it off for one run. When you already run `sfw cargo project-tools …`, the tool detects the outer sfw and doesn't start a second one. `audit` runs under sfw because cargo downloads the tool's dependencies to work out which of them are built. `audit-db` doesn't start sfw: the advisory database isn't a package. `audit` and `install` fetch the database under sfw, and trust sfw's certificate through `SSL_CERT_FILE`.

On Windows, cargo checks certificate revocation through schannel, and sfw's local certificate has no revocation endpoint, so downloads fail with `the revocation status is unknown`. Turn the check off for cargo with `CARGO_HTTP_CHECK_REVOKE=false`, or `http.check-revoke = false` in your [cargo config](https://doc.rust-lang.org/cargo/reference/config.html#httpcheck-revoke). Certificate and hostname checks stay on.

## sandbox

`install` (and `verify --repair`) run the build in an OS sandbox, so build scripts and proc macros, which run during the build, can't reach the network or write outside the build. That's on top of the [build review](build-review.md), not instead of it: everything stays readable, as cargo and rustc need your toolchain and `CARGO_HOME`.

| OS | Sandbox |
| --- | --- |
| macOS | `sandbox-exec` (Seatbelt), which ships with macOS |
| Linux | [bubblewrap](https://github.com/containers/bubblewrap) (`bwrap`), when it's on `PATH` and user namespaces are allowed |
| Others, Windows included | None |

In the sandbox, the build can write only to `.project-tools/targets/<tool>` (the build directory, with its own temp directory), `.project-tools/bin`, and cargo's install records `.project-tools/.crates.toml` and `.crates2.json`. Your project, `build-review.toml`, `CARGO_HOME` and your home directory are read-only, and there's no network. The build needs none: it runs from the vendored sources. Only the build is sandboxed. Downloading, vendoring and resolving run before it, outside, and through [sfw](#sfw).

| `sandbox` | Behavior |
| --- | --- |
| `"auto"` (default) | Use the sandbox when this OS has one that works, otherwise build without it, with a note (and a `sandbox-missing` [JSON record](commands.md#json-output)) |
| `"require"` | Fail when there's no working sandbox. On Windows that's always |
| `"off"` | Never use it |

`[build.sandbox-os]` sets it per operating system, by Rust's name for it (`std::env::consts::OS`), and wins over `sandbox`. For example, require it on macOS, use it when there on Linux, and don't look on Windows:

```toml
[build.sandbox-os]
macos = "require"
linux = "auto"
windows = "off"
```

A tool's own `[tool.<name>.build]` can set both too. `--no-sandbox` turns it off for one run. Changing it doesn't rebuild a tool that's up to date.

A build script that needs the network, for example one that downloads a library, fails in the sandbox. Look at why in its review before you turn the sandbox off for that tool.

## advisory-db-max-age

How long `audit` and `install` use the advisory database before they check it again. It takes hours (`"12h"`) or days (`"1d"`), and defaults to `"1d"`. A check sends the stored ETag, so it costs one small request when nothing changed. `--update-db` and `audit-db update` check whatever the age. `install --offline` never checks.

## audit-ignore

`.project-tools/crates/<tool>/audit-ignore.toml` lists advisories you accept for that tool:

```toml
[[ignore]]
id = "RUSTSEC-2024-0001"     # or a CVE/GHSA alias
reason = "only affects a feature we don't enable"
expires = "2027-01-01"       # optional
```

`id` matches any advisory, including unmaintained, unsound and `git-source` warnings.

- `reason` is required.
- After `expires`, the ignore stops applying and `audit` reports the advisory again.
- `audit` warns about ignores that no longer match anything, so the list doesn't grow stale.

### Advisories against cargo-project-tools itself

`audit`, and the audit `install` runs first, also check cargo-project-tools and the `rustsec` library it's built with, and fail on a vulnerability in either. To keep CI running until you can move to a fixed release, accept it in `.project-tools/self-audit-ignore.toml`, in the same format:

```toml
[[ignore]]
id = "RUSTSEC-2027-0001"
reason = "we don't fetch untrusted advisory databases; upgrading after the next release"
expires = "2027-02-01"       # required here
```

`expires` is required in this file, so an ignore for the tool itself always ends. Commit the file with the rest of `.project-tools`.

## self-update source

`~/.cargo-project-tools/config.toml` (under `CARGO_PROJECT_TOOLS_HOME` when set) tells the [`self-*` commands](commands.md#managing-cargo-project-tools-itself) where releases come from and who must have signed them. It's optional; without it, releases come from this repository.

```toml
repo = "acme/cargo-project-tools"
# signer = "https://github.com/acme/cargo-project-tools/.github/workflows/release.yml@refs/heads/main"
```

| Key | Meaning |
| --- | --- |
| `repo` | GitHub repository as `owner/name`. Default: `osseco/cargo-project-tools` |
| `signer` | The cosign certificate identity every download must be signed by (https only). Default: `release.yml` on `main` of `repo` |

A fork sets `repo`, and `signer` only when its release workflow isn't `release.yml` on `main`. `self-list` prints the values in use.
