# Verifying a release

Every release asset carries a build provenance attestation and a cosign signature, and is listed in `SHA256SUMS`.

## What a release contains

- A binary per platform (see [Getting started](getting-started.md#install)).
- `SHA256SUMS` for the binaries.
- CycloneDX and SPDX SBOMs.
- `LICENSE-MIT` and `LICENSE-APACHE`.
- A `<asset>.sigstore.json` cosign bundle for every asset.

Releases are immutable: an asset never changes after publishing.

## Build provenance

```bash
gh attestation verify cargo-project-tools-x86_64-unknown-linux-musl --repo osseco/cargo-project-tools
```

The SBOM attestation uses the same command with `--predicate-type https://spdx.dev/Document/v2.3`.

## cosign

```bash
cosign verify-blob cargo-project-tools-x86_64-unknown-linux-musl \
  --bundle cargo-project-tools-x86_64-unknown-linux-musl.sigstore.json \
  --certificate-identity https://github.com/osseco/cargo-project-tools/.github/workflows/release.yml@refs/heads/main \
  --certificate-oidc-issuer https://token.actions.githubusercontent.com
```

The identity says the asset was signed by `release.yml` on `main` of this repository, and nowhere else.

## Checksums

```bash
sha256sum --check --ignore-missing SHA256SUMS
```

On macOS, use `shasum -a 256 --check --ignore-missing SHA256SUMS`.

## Embedded dependency list

The binaries are built with [cargo-auditable](https://github.com/rust-secure-code/cargo-auditable), so each one carries the exact crate versions it was built from. Scan a downloaded binary for known vulnerabilities:

```bash
cargo audit bin cargo-project-tools-x86_64-unknown-linux-musl
```

syft and other scanners that support cargo-auditable read the same data.
