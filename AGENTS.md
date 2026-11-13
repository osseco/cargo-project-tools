# AGENTS.md

## What this is
`cargo-project-tools` (run as `cargo project-tools`) manages a Rust project's cargo-installed tools.
Tool versions live in `project-tools.toml`. Each tool gets its own committed `Cargo.lock`, so
vulnerable dependencies can be bumped without waiting for the tool's maintainer. Binaries and
build dirs stay inside the project, so CI can cache them separately.

```
<project root>
├── project-tools.toml
└── .project-tools
    ├── .gitignore
    ├── advisory-db
    ├── bin
    ├── crates
    │   └── <crate>
    │       ├── Cargo.lock
    │       └── audit-ignore.toml
    └── targets
        └── <crate>
```
Committed: `project-tools.toml`, `.project-tools/.gitignore`, `.project-tools/crates/**`. Everything else is ignored.

Commands: `init`, `add`, `remove`, `upgrade`, `update`, `install`, `audit`, `audit-db update`, `list`.

## Code map
- `src/main.rs`: command implementations. `src/cli.rs`: clap definitions (`SUBCOMMAND` constant).
- `src/project.rs`: root discovery, the layout above, `.gitignore` upkeep.
- `src/config.rs`: `project-tools.toml` and `audit-ignore.toml` (read with serde, edited with `toml_edit` so comments survive).
- `src/source/registry.rs`: sparse index, `.crate` download with sha256 check, safe extraction, `IndexCache`.
- `src/source/git.rs`: https-only clones. `src/source/mod.rs`: full checkouts and minimal scratch projects (manifest + target stubs).
- `src/lock.rs`: lock parsing, root check, diff, `cargo update`.
- `src/install.rs`: `cargo install --path` invocation, build settings → `--config`, install stamps.
- `src/audit.rs`: runs cargo-audit, our own yanked check, ignore list, fix suggestions.
- `src/sfw.rs`: re-executes package-downloading commands under sfw.
- `src/net.rs`: ureq agent (honours `HTTPS_PROXY` and `SSL_CERT_FILE`).

## Development workflow
- Anything that downloads crates goes through Socket Firewall Free: `sfw cargo add …`, `sfw cargo build`, `sfw cargo update`. Run the tool itself as `sfw cargo project-tools …` (or let it wrap itself with `settings.sfw = "auto"`).
- Don't run `cargo audit` under sfw. cargo-audit uses the platform TLS verifier, which doesn't trust sfw's CA, and retries forever.
- Before committing: `pre-commit run --all-files` (fmt, clippy, test, cargo-deny, cargo-audit, zizmor, actionlint, basic file checks). These hooks need `cargo-deny`, `cargo-audit`, `zizmor`, `actionlint` and `pre-commit` on PATH.
- Unit tests sit next to the code (`cargo test`). They don't touch the network.
- GitHub Actions: pin every action to a full commit SHA with a version comment, keep `permissions: contents: read`, and set `persist-credentials: false`. zizmor must report no findings.

## Security rules
- https only for downloads and git; `GIT_ALLOW_PROTOCOL=https` is set for clones.
- Verify every `.crate` against the index checksum before use. Reject archive paths that escape the crate directory.
- Start processes with `Command::arg`, never a shell. Reject user values that start with `-` where they're passed as git refs.
- Never weaken TLS verification. Extra trust comes only from `SSL_CERT_FILE` (set by sfw).
- `install` always passes `--locked`, so cargo refuses a lock that doesn't fit the manifest.
