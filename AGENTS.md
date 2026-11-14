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

Exception: never run `cargo audit` under sfw. cargo-audit uses the platform TLS verifier, which doesn't trust sfw's CA, and retries forever. Run it plainly: `cargo audit`.

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

## Checks

Run `pre-commit run --all-files` before committing. It runs fmt, clippy, test, cargo-deny, cargo-audit, zizmor, actionlint and basic file checks.

It needs these on PATH: `sfw`, `cargo-deny`, `cargo-audit`, `zizmor`, `actionlint`, `pre-commit`.

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

- Pin every action to a full commit SHA with a version comment, in workflows and in README examples alike.
- Keep `permissions: contents: read`.
- Set `persist-credentials: false` on checkout.
- zizmor and actionlint must report no findings.
