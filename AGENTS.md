# AGENTS.md

How AI agents work in this repository. What the tool does, and how to use it: [README.md](README.md) and the user docs in [docs/](docs/), indexed by [llms.txt](llms.txt).

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

Agents never commit. The maintainer reviews and signs every commit; the signing key needs a password.

- Never run `git commit`, `git tag`, `git push`, or anything that rewrites history (`rebase`, `reset --hard`, `commit --amend`, force pushes).
- Work in reviewable chunks. At the end of each chunk, stage exactly that chunk's files with explicit paths (`git add <path>…`, never `git add -A` or `git add .`), show `git diff --cached --stat`, and propose a commit message. Staged means "commit this". Then wait until the maintainer has committed before starting the next chunk. On requested changes, edit and re-stage.
- Staging is the only commit-related git write an agent does. Creating and switching branches is fine. Anything that makes a commit (`cherry-pick`, `merge`, `rebase`), and `fetch`, `pull` and `push`, is the maintainer's: give them the exact command.
- Pull requests are squash-merged and the PR title becomes the commit on `main`, so PR titles follow the same format.

Messages use [Conventional Commits](https://www.conventionalcommits.org/):

```text
type(scope): imperative subject
```

- Types: `feat`, `fix`, `docs`, `refactor`, `test`, `build`, `ci`, `chore`.
- Scopes when useful: `cli`, `audit`, `install`, `lock`, `source`, `site`, `ci`.
- The subject is lowercase imperative, with no trailing period.
- Breaking changes use `!` or a `BREAKING CHANGE:` footer.
- The `ci` type and scope are only for this project's own CI, not for the CI guidance in `docs/ci.md`.
- Commit types decide the next release version (see [CONTRIBUTING.md](CONTRIBUTING.md#versions)). Pick them deliberately: `feat` bumps minor, `!` or `BREAKING CHANGE:` bumps major, everything else bumps patch.

## Checks

Run `pre-commit run --all-files` before staging a chunk. It runs fmt, clippy, test, cargo-deny, zizmor, actionlint, the version-script checks and basic file checks.

It needs these on PATH: `sfw`, `cargo-deny`, `zizmor`, `actionlint`, `uv`, `pre-commit`.

Unit tests go in a sibling `<module>_test.rs` file, pulled in at the end of the module with `#[cfg(test)] #[path = "<module>_test.rs"] mod tests;`. They must not touch the network. Tests that run cargo use the offline registry in `src/test_registry.rs`.

pre-commit doesn't cover `site/`. After changing `site/`, `docs/` or `llms.txt`, run in `site/`:

```bash
sfw pnpm install --frozen-lockfile
sfw pnpm run lint
sfw pnpm run typecheck
sfw pnpm test
sfw pnpm run build
```

Never stage `site/dist/` or `site/node_modules/`.

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
- `src/review.rs`: `build-review.toml`: collects build scripts, proc macros and shipped cargo config, checks them against the file, applies overrides and strips.
- `src/review_tui.rs` and `src/review_tui/`: the review screen (ratatui, edtui), a submodule of `review`. `text.rs` sanitizes what it shows and writes overrides (no links, atomic), `tree.rs` builds the tree, `app.rs` holds the state and actions, `input.rs` and `render.rs` handle keys, mouse and drawing.
- `src/highlight.rs`: a small Rust and TOML lexer that colours the review screen's editor.
- `src/vendor.rs`: `cargo vendor` into `targets/<tool>/vendor`; builds use it through source replacement.
- `src/graph.rs`: which locked packages a tool builds with its features and targets (`cargo tree` on a scratch copy).
- `src/targets.rs`: the host triple, the check that it and `--target` are approved targets, and the target suggestions for `init` (this machine and GitHub workflow runners).
- `src/audit.rs`: matches locks against the advisory database with rustsec (root and git packages included), our own yanked check, ignore list, fix suggestions.
- `src/advisory_db.rs`: downloads, revalidates (ETag, `advisory-db-max-age`) and swaps in the RustSec advisory database.
- `src/sfw.rs`: re-executes package-downloading commands under sfw.
- `src/net.rs`: ureq agent (honours `HTTPS_PROXY` and `SSL_CERT_FILE`).
- `site/`: the GitHub Pages site (Vite + React, built from the [osseco web design guide](https://github.com/osseco/web-design-guide) templates). It renders `docs/*.md` and publishes them raw with `llms.txt`. Follow the guide's `AGENTS.md` and `docs/checklist.md` when changing it.
- `docs/`: user docs. A change to a command or setting updates them in the same commit. Adding, renaming or removing a doc also updates `llms.txt` and the list in `README.md`.

## Security rules

- https only for downloads and git. `GIT_ALLOW_PROTOCOL=https` is set for clones.
- Verify every `.crate` against the index checksum before use.
- Reject archive paths that escape the crate directory.
- Tool names must be crate names (they become directory names), and files we rewrite in a tool's source must not be links.
- Start processes with `Command::arg`, never a shell.
- Cargo runs from the project root, never from inside a tool's source: config and toolchain files are found from the working directory up. A tool's shipped `rust-toolchain*` files are removed.
- Reject user values that start with `-` where they're passed as git refs.
- Never weaken TLS verification. Extra trust comes only from `SSL_CERT_FILE` (set by sfw).
- `install` always passes `--locked`, so cargo refuses a lock that doesn't fit the manifest.
- Build-time code (build scripts, proc macros, a tool's shipped `.cargo/config*`) is built only when recorded in the tool's `build-review.toml`. Collecting it uses `cargo tree`, `cargo metadata` and `cargo vendor`, which compile and run nothing.

## GitHub Actions

How CI and releases work: [CONTRIBUTING.md](CONTRIBUTING.md#ci).

- Pin every action to a full commit SHA with a version comment, in workflows and in README and `docs/` examples alike.
- Start each workflow with `permissions: {}`, and grant per job only what it needs.
- Set `persist-credentials: false` on checkout.
- Pass `${{ }}` values to `run:` through `env:`, never inline.
- No `actions/cache` in `product.yml` or `release.yml` for jobs whose output is signed and released. Two exceptions: the sfw binary cache of `osseco/setup-sfw-free`, which is checked against a pinned sha256 on restore, and the `test` job's cargo cache (registry and `target/`), which `main` builds from scratch and saves (`actions/cache/save`) and every other run only restores (`actions/cache/restore`). Test output is never released.
- Every job that installs packages runs `osseco/setup-sfw-free` with `cache: ${{ github.ref == 'refs/heads/main' && 'true' || 'restore' }}`: only `main` saves the cache, everything else restores it. Each action version pins one sfw-free version; Dependabot updates it.
- Pin tools that actions download by version (syft, zizmor, Socket CLI via `.github/socket/uv.lock`).
- zizmor and actionlint must report no findings.
