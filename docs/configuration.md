# Configuration

`project-tools.toml` at the project root, and an `audit-ignore.toml` per tool. When the tool edits `project-tools.toml`, your comments stay.

## project-tools.toml

```toml
[settings]
sfw = "auto"                 # "auto", "require" or "off"

[build]
profile = "release"

[build.profile-settings]     # applied to every tool
strip = "symbols"
lto = "thin"
codegen-units = 1

[tool.cargo-nextest]
version = "0.9.80"

[tool.cargo-nextest.build.profile-settings]
panic = "unwind"             # per-tool override
```

`add`, `upgrade` and `remove` maintain the `[tool.*]` entries. You edit `[settings]` and `[build]`.

### [build]

| Key | Meaning |
| --- | --- |
| `profile` | Cargo profile to build with |
| `target` | Target triple |
| `rustflags` | List of extra `rustflags` |
| `profile-settings` | Profile keys applied to the build profile |

Allowed `profile-settings` keys: `inherits`, `opt-level`, `debug`, `strip`, `lto`, `codegen-units`, `panic`, `incremental`, `debug-assertions`, `overflow-checks`, `split-debuginfo`, `rpath`.

### [tool.\<name\>]

| Key | Meaning |
| --- | --- |
| `version` | The tool's version |
| `git`, `tag`, `rev`, `branch` | Git source (https only) |
| `features` | Features to enable |
| `default-features` | `false` turns off default features |
| `bins` | Binaries to install (default: all) |
| `build` | A `[build]` table for this tool only; its keys override the global ones |

Changing any of these makes `install` rebuild the tool.

## sfw

Package downloads (`add`, `upgrade`, `update`, `install`) can run through [Socket Firewall Free](https://docs.socket.dev/docs/socket-firewall-free), which blocks known malicious packages.

| `settings.sfw` | Behavior |
| --- | --- |
| `"auto"` (default) | Use `sfw` when it's on `PATH`, otherwise carry on with a note |
| `"require"` | Fail when `sfw` isn't on `PATH` |
| `"off"` | Never use `sfw` |

`--no-sfw` turns it off for one run. When you already run `sfw cargo project-tools …`, the tool detects the outer sfw and doesn't start a second one. `audit` never runs through sfw: cargo-audit's TLS verifier doesn't trust sfw's certificate, and the advisory database isn't a package.

## audit-ignore

`.project-tools/crates/<tool>/audit-ignore.toml` lists advisories you accept for that tool:

```toml
[[ignore]]
id = "RUSTSEC-2024-0001"     # or a CVE/GHSA alias
reason = "only affects a feature we don't enable"
expires = "2027-01-01"       # optional
```

- `reason` is required.
- After `expires`, the ignore stops applying and `audit` reports the advisory again.
- `audit` warns about ignores that no longer match anything, so the list doesn't grow stale.
