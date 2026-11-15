# cargo-project-tools

Install your project's cargo tools from **your own Cargo.lock files**.

`cargo install --locked` builds a tool with the Cargo.lock its maintainer published. When a dependency gets a security fix, you wait for a new release. `cargo project-tools` keeps a Cargo.lock per tool in your repository, so you can audit it, bump vulnerable dependencies yourself, and install the tools into the project.

Docs: <https://osseco.github.io/cargo-project-tools/>

## Install

Download a binary for your platform from the [latest release](https://github.com/osseco/cargo-project-tools/releases/latest), check it, and put it on your `PATH`:

```bash
gh attestation verify cargo-project-tools-x86_64-unknown-linux-musl --repo osseco/cargo-project-tools
```

Or build from source:

```bash
cargo install --git https://github.com/osseco/cargo-project-tools --locked
```

## Quick start

```bash
cargo project-tools init                 # create project-tools.toml and .project-tools/
cargo project-tools add cargo-nextest    # starts from the lock the tool ships with
cargo project-tools audit                # needs cargo-audit
cargo project-tools update cargo-nextest <dep>@<old> --precise <new>   # as suggested by audit
cargo project-tools install              # builds into .project-tools/bin
```

Add `.project-tools/bin` to your `PATH`. Commit `project-tools.toml` and `.project-tools/crates/`.

## Docs

- [Getting started](docs/getting-started.md)
- [How it works](docs/how-it-works.md)
- [Commands](docs/commands.md)
- [Configuration](docs/configuration.md)
- [CI](docs/ci.md)
- [Verifying a release](docs/verifying-releases.md)

Contributing: [CONTRIBUTING.md](CONTRIBUTING.md). Security: [SECURITY.md](SECURITY.md).

## License

To be decided.
