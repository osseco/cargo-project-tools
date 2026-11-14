# AGENTS.md

How AI agents work in this repository. What the tool does, and how to use it: [README.md](README.md).

## Socket Firewall

Prefix every `pnpm` and `cargo` command with `sfw` ([Socket Firewall Free](https://docs.socket.dev/docs/socket-firewall-free)). This includes install, build, test, fmt, and clippy.

```bash
sfw cargo test --locked
sfw cargo clippy --locked --all-targets -- -D warnings
sfw cargo fmt --all
sfw cargo add <crate>
sfw pnpm install --frozen-lockfile
```

Exceptions, run plainly because their TLS verifiers don't trust sfw's CA:
- `cargo audit` would retry forever.
- `rustup toolchain install` fails. rustup checks its own downloads; the toolchain is pinned in `rust-toolchain.toml`.

Python tooling goes through sfw too: `sfw uv lock`, `sfw uv sync`.

The tool itself also wraps its package downloads in sfw (`settings.sfw = "auto"`). If you run it as `sfw cargo project-tools …`, it detects the outer sfw and doesn't start a second one.

## Branches

Never change or commit directly on `main`. Before the first edit, check the current branch. If it is `main`, create a branch from `origin/main` (no `git fetch`), not from local `main`, and work there:

```bash
git switch --no-track -c fix/short_description origin/main
```

Name it `type/short_description`, with the same types as commits.

## Commits

Use [Conventional Commits](https://www.conventionalcommits.org/):

```text
type(scope): imperative subject
```

- Types: `feat`, `fix`, `docs`, `refactor`, `test`, `build`, `ci`, `chore`.
- Scopes when useful: `cli`, `audit`, `install`, `lock`, `source`, `site`, `ci`.
- The subject is lowercase imperative, with no trailing period.
- Breaking changes use `!` or a `BREAKING CHANGE:` footer.
- The `ci` type and scope are only for this project's own CI, not for the CI guidance in the README.
- Commit types decide the next release version (see [CONTRIBUTING.md](CONTRIBUTING.md#versions)). Pick them deliberately: `feat` bumps minor, `!` or `BREAKING CHANGE:` bumps major, everything else bumps patch.

## Checks

Run `pre-commit run --all-files` before committing. It runs fmt, clippy, test, cargo-deny, cargo-audit, zizmor, actionlint, the version-script checks and basic file checks.

It needs these on PATH: `sfw`, `cargo-deny`, `cargo-audit`, `zizmor`, `actionlint`, `uv`, `pre-commit`.

Unit tests sit next to the code and must not touch the network.

## Code map

- `src/main.rs`: command implementations.
- `src/cli.rs`: clap definitions (`SUBCOMMAND` constant).
- `src/project.rs`: root discovery, the `.project-tools` layout, `.gitignore` upkeep.
- `src/config.rs`: `project-tools.toml` and `audit-ignore.toml` (read with serde, edited with `toml_edit` so comments survive).
- `src/source/registry.rs`: sparse index, `.crate` download with sha256 check, safe extraction, `IndexCache`.
- `src/source/git.rs`: https-only clones.
- `src/source/mod.rs`: full checkouts and minimal scratch projects (manifest + target stubs).
- `src/lock.rs`: lock parsing, root check, diff, `cargo update`.
- `src/install.rs`: `cargo install --path` invocation, build settings → `--config`, install stamps.
- `src/audit.rs`: runs cargo-audit, our own yanked check, ignore list, fix suggestions.
- `src/sfw.rs`: re-executes package-downloading commands under sfw.
- `src/net.rs`: ureq agent (honours `HTTPS_PROXY` and `SSL_CERT_FILE`).

## Security rules

- https only for downloads and git. `GIT_ALLOW_PROTOCOL=https` is set for clones.
- Verify every `.crate` against the index checksum before use.
- Reject archive paths that escape the crate directory.
- Start processes with `Command::arg`, never a shell.
- Reject user values that start with `-` where they're passed as git refs.
- Never weaken TLS verification. Extra trust comes only from `SSL_CERT_FILE` (set by sfw).
- `install` always passes `--locked`, so cargo refuses a lock that doesn't fit the manifest.

## GitHub Actions

How CI and releases work: [CONTRIBUTING.md](CONTRIBUTING.md#ci).

- Pin every action to a full commit SHA with a version comment, in workflows and in README examples alike.
- Start each workflow with `permissions: {}`, and grant per job only what it needs.
- Set `persist-credentials: false` on checkout.
- Pass `${{ }}` values to `run:` through `env:`, never inline.
- No `actions/cache` in `product.yml` or `release.yml`: their output is signed and released. The one exception is the sfw binary cache of `osseco/setup-sfw-free`, which is checked against a pinned sha256 on restore.
- Every job that installs packages runs `osseco/setup-sfw-free` with `cache: ${{ github.ref == 'refs/heads/main' && 'true' || 'restore' }}`: only `main` saves the cache, everything else restores it. Each action version pins one sfw-free version; Dependabot updates it.
- Pin tools that actions download by version (syft, zizmor, Socket CLI via `.github/socket/uv.lock`).
- zizmor and actionlint must report no findings.
