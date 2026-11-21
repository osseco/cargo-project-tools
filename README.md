# cargo-project-tools

Install your project's cargo tools from **your own Cargo.lock files**.

`cargo install --locked` builds a tool with the Cargo.lock its maintainer published. When a dependency gets a security fix, you wait for a new release. `cargo project-tools` keeps a Cargo.lock per tool in your repository, so you can audit it, bump vulnerable dependencies yourself, and install the tools into the project.

Docs: <https://osseco.github.io/cargo-project-tools/>

## Install

On macOS or Linux, install into `~/.cargo-project-tools` with the signed `install.sh` from the latest release. It needs `curl` and [cosign](https://docs.sigstore.dev/cosign/system_config/installation/):

```bash
curl -fsSLO https://github.com/osseco/cargo-project-tools/releases/latest/download/install.sh
curl -fsSLO https://github.com/osseco/cargo-project-tools/releases/latest/download/install.sh.sigstore.json
cosign verify-blob install.sh --bundle install.sh.sigstore.json \
  --certificate-identity https://github.com/osseco/cargo-project-tools/.github/workflows/release.yml@refs/heads/main \
  --certificate-oidc-issuer https://token.actions.githubusercontent.com
sh install.sh
```

Or download a binary for your platform from the [latest release](https://github.com/osseco/cargo-project-tools/releases/latest), check it, and put it on your `PATH`:

```bash
gh attestation verify cargo-project-tools-x86_64-unknown-linux-musl --repo osseco/cargo-project-tools
```

Or build from source:

```bash
cargo install --git https://github.com/osseco/cargo-project-tools --locked
```

Then, on macOS and Linux, it keeps itself up to date under `~/.cargo-project-tools/bin` (signatures checked with cosign):

```bash
cargo project-tools self-check          # is a newer version installed or released?
cargo project-tools self-update         # install the latest and switch to it
cargo project-tools self-update 1.2.0   # install a given version and switch to it
cargo project-tools self-list           # installed versions, * marks the active one
```

## Quick start

```bash
cargo project-tools init                 # create project-tools.toml and .project-tools/
cargo project-tools add cargo-nextest    # starts from the lock the tool ships with, or generates one
cargo project-tools audit                # against the RustSec advisory database
cargo project-tools update cargo-nextest <dep>@<old> --precise <new>   # as suggested by audit
cargo project-tools install              # audits, then builds into .project-tools/bin
```

Add `.project-tools/bin` to your `PATH`. Commit `project-tools.toml` and `.project-tools/crates/`.

## Docs

- [Getting started](docs/getting-started.md)
- [How it works](docs/how-it-works.md)
- [Commands](docs/commands.md)
- [Configuration](docs/configuration.md)
- [Build review](docs/build-review.md)
- [CI](docs/ci.md)
- [AI agents](docs/agents.md)
- [Verifying a release](docs/verifying-releases.md)

Contributing: [CONTRIBUTING.md](CONTRIBUTING.md). Security: [SECURITY.md](SECURITY.md).

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT license ([LICENSE-MIT](LICENSE-MIT))

at your option.

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in the work by you, as defined in the Apache-2.0 license, shall be dual licensed as above, without any additional terms or conditions.
