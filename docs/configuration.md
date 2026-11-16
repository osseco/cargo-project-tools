# Configuration

`project-tools.toml` at the project root, an `audit-ignore.toml` per tool, and optionally a [`self-audit-ignore.toml`](#advisories-against-cargo-project-tools-itself). When the tool edits `project-tools.toml`, your comments stay.

## project-tools.toml

```toml
[settings]
sfw = "auto"                 # "auto", "require" or "off"
advisory-db-max-age = "1d"   # optional; hours ("12h") or days ("1d")

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

`--no-sfw` turns it off for one run. When you already run `sfw cargo project-tools …`, the tool detects the outer sfw and doesn't start a second one. `audit` and `audit-db` don't start sfw: the advisory database isn't a package. `install` fetches the database under sfw, and trusts sfw's certificate through `SSL_CERT_FILE`.

On Windows, cargo checks certificate revocation through schannel, and sfw's local certificate has no revocation endpoint, so downloads fail with `the revocation status is unknown`. Turn the check off for cargo with `CARGO_HTTP_CHECK_REVOKE=false`, or `http.check-revoke = false` in your [cargo config](https://doc.rust-lang.org/cargo/reference/config.html#httpcheck-revoke). Certificate and hostname checks stay on.

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
