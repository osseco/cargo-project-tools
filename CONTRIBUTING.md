# Contributing

Conventions for commits, branches and sfw are in [AGENTS.md](AGENTS.md). This page covers CI, versions and releases.

## Setup

- The Rust toolchain is pinned in `rust-toolchain.toml`. Install it with `rustup toolchain install` (not under sfw; rustup checks its own downloads).
- [Socket Firewall Free](https://docs.socket.dev/docs/socket-firewall-free) (`sfw`), `cargo-deny`, `cargo-audit`, `zizmor`, `actionlint`, `uv` and [pre-commit](https://pre-commit.com/).
- Run `pre-commit install` in your clone.

## CI

| Workflow | When | What |
| --- | --- | --- |
| `pr.yml` | every pull request | calculates the next version, runs `product.yml` with `<next>-pr.<number>.<run>.<attempt>`, then `Merge gate` |
| `product.yml` | called by `pr.yml` and `release.yml` | cargo-deny, cargo-audit, tests, pre-commit, SBOMs, builds; calls CodeQL, zizmor, actionlint and Socket |
| `release.yml` | push to `main` touching the product | the same product run for every target, then publishes the release |
| `codeql.yml`, `zizmor.yml`, `actionlint.yml`, `socket.yml` | also weekly | report findings to code scanning |

Every job that installs crates runs behind sfw. `cross` builds fetch through sfw first, then build offline inside the container. Jobs whose output is released use no `actions/cache`, so a poisoned cache entry can't end up in a signed binary.

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

```bash
gh attestation verify cargo-project-tools-x86_64-unknown-linux-musl --repo osseco/cargo-project-tools
```

```bash
cosign verify-blob cargo-project-tools-x86_64-unknown-linux-musl \
  --bundle cargo-project-tools-x86_64-unknown-linux-musl.sigstore.json \
  --certificate-identity https://github.com/osseco/cargo-project-tools/.github/workflows/release.yml@refs/heads/main \
  --certificate-oidc-issuer https://token.actions.githubusercontent.com
```

```bash
sha256sum --check --ignore-missing SHA256SUMS
```

Renaming `release.yml` or releasing from another branch changes the signing identity.
