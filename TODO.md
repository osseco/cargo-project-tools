# TODO

## Repository settings

Needed before the workflows work as intended:

- [ ] Create the GitHub repository `osseco/cargo-project-tools` and push `main`.
- [ ] **Releases → Immutable releases**: on.
- [ ] **Security → Private vulnerability reporting**: on (`SECURITY.md` points there).
- [ ] **Security → Code scanning**: on, so CodeQL, zizmor and actionlint SARIF uploads show up.
- [ ] **Secrets → Actions**: add `SOCKET_SECURITY_API_KEY`.
- [ ] **Environments**: create `release`, limit deployment branches to `main`, and add required reviewers if releases should be approved.
- [ ] **Branch protection / ruleset for `main`**: require pull requests and the `Merge gate` status check; block force pushes.
- [ ] **Actions → General → Workflow permissions**: read-only default token. Jobs request what they need, such as `contents: write` for publishing.

## Open decisions

- [ ] License (`Cargo.toml` has none; `deny.toml` skips the private crate).
