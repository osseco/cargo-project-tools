# TODO

Manual steps for a maintainer. Check them off as they're done.

## Repository settings

Needed before the workflows work as intended:

- [ ] Create the GitHub repository `osseco/cargo-project-tools` and push `main`.
- [ ] **Releases → Immutable releases**: on.
- [ ] **General → Pull Requests**:
  - [ ] Allow squash merging only; no merge commits, no rebase merging.
  - [ ] Default squash commit message: pull request title. The title must be a Conventional Commit; it decides the next version.
  - [ ] Automatically delete head branches.
- [ ] **Security → Private vulnerability reporting**: on (`SECURITY.md` points there).
- [ ] **Security → Dependabot alerts** and **Dependabot security updates**: on.
- [ ] **Security → Secret scanning** and **Push protection**: on.
- [ ] **Security → Code scanning**: on, so CodeQL, zizmor and actionlint SARIF uploads show up.
- [ ] **Secrets → Actions**: add `SOCKET_SECURITY_API_KEY`.
- [ ] **Environments**: create `release`, limit deployment branches to `main`, add required reviewers.
- [ ] **Ruleset for `main`**: require pull requests and the `Merge gate` status check; block force pushes and deletion; require signed commits; require linear history; no bypass actors.
- [ ] **Ruleset for tags `v*`**: block updates and deletion.
- [ ] **Actions → General**:
  - [ ] Workflow permissions: read-only default token. Jobs request what they need, such as `contents: write` for publishing.
  - [ ] Don't allow GitHub Actions to create or approve pull requests.
  - [ ] Require actions to be pinned to a full-length commit SHA.
  - [ ] Require approval for workflows from fork pull requests by outside contributors.
- [ ] Set the owner in `.github/CODEOWNERS`, then require Code Owner review in the `main` ruleset.

## Open decisions

- [ ] License (`Cargo.toml` has none; `deny.toml` skips the private crate).

## First release

- [ ] Merge to `main`, approve the `release` environment, and check that the release has the four binaries, `SHA256SUMS`, the CycloneDX and SPDX SBOMs, and a `.sigstore.json` bundle per asset.
- [ ] Verify build provenance: `gh attestation verify cargo-project-tools-x86_64-unknown-linux-musl --repo osseco/cargo-project-tools`.
- [ ] Verify the SBOM attestation: the same command with `--predicate-type https://spdx.dev/Document/v2.3`.
- [ ] `sha256sum --check --ignore-missing SHA256SUMS`.
