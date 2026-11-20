# Contributing

Conventions for commits, branches and sfw are in [AGENTS.md](AGENTS.md). This page covers CI, versions and releases.

## Setup

- The Rust toolchain is pinned in `rust-toolchain.toml`. Install it with `rustup toolchain install` (not under sfw; rustup checks its own downloads).
- [Socket Firewall Free](https://docs.socket.dev/docs/socket-firewall-free) (`sfw`), `cargo-deny`, `zizmor`, `actionlint`, `uv` and [pre-commit](https://pre-commit.com/).
- Run `pre-commit install` in your clone.

## CI

| Workflow | When | What |
| --- | --- | --- |
| `pr.yml` | every pull request | calculates the next version, runs `product.yml` with `<next>-pr.<number>.<run>.<attempt>` and `site.yml`, then `Merge gate` |
| `site.yml` | called by `pr.yml` and `pages.yml` | audit, lint, typecheck, test and build the Pages site in `site/` |
| `pages.yml` | push to `main` touching `site/`, `docs/` or `llms.txt` | runs `site.yml`, builds with the Pages base path, deploys to GitHub Pages |
| `product.yml` | called by `pr.yml` and `release.yml` | cargo-deny, tests on Linux, macOS and Windows, pre-commit, SBOMs, builds; calls CodeQL, zizmor, actionlint and Socket |
| `release.yml` | push to `main` touching the product | the same product run for every target, then publishes the release |
| `codeql.yml`, `zizmor.yml`, `actionlint.yml`, `socket.yml` | also weekly | report findings to code scanning |

Every job that installs crates runs behind sfw. `cross` builds fetch through sfw first, then build offline inside the container. Release binaries are built with cargo-auditable, and the build fails if a binary has no embedded dependency list. Jobs whose output is released use no `actions/cache`, so a poisoned cache entry can't end up in a signed binary. The exception is the sfw binary, installed by `osseco/setup-sfw-free`: only `main` saves it to the cache, other runs restore it, and every restore is checked against the sha256 pinned in the action. The test jobs, whose output isn't released, cache their cargo registry and `target/` by the same rule: `main` always builds from scratch and saves, pull requests only restore. Tests run on Linux, macOS and Windows without debug info, so the pricier runners compile less.

## Versions

The version comes from the latest `vX.Y.Z` tag and the [Conventional Commits](https://www.conventionalcommits.org/) since then (`.github/calculate-version.sh`). Without a tag it starts at `0.0.0`.

- `feat!:` or a `BREAKING CHANGE:` footer bumps major.
- `feat:` bumps minor.
- Anything else bumps patch.
- Each release bumps once, by the highest-ranking commit.

`Cargo.toml` keeps `version = "0.0.0"`. CI writes the real version with `.github/update_version_in_cargo.py` before building.

## Releases

A push to `main` that changes the product runs `release.yml`. When the version changed, the `publish` job (environment `release`) does four things:

1. Stages the binaries for `aarch64-apple-darwin`, `x86_64-unknown-linux-musl`, `aarch64-unknown-linux-musl` and `x86_64-pc-windows-msvc`, plus `SHA256SUMS` and CycloneDX and SPDX SBOMs.
2. Signs every asset with cosign; each gets a `<asset>.sigstore.json` bundle.
3. Attests build provenance and the SBOM for the binaries in `SHA256SUMS`.
4. Creates a draft GitHub Release with every asset, then publishes it. Releases are immutable, so a broken release is fixed by the next version.

## Verifying a release

See [docs/verifying-releases.md](docs/verifying-releases.md). Renaming `release.yml` or releasing from another branch changes the signing identity.
