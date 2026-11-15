# How it works

Why each tool gets a Cargo.lock in your repository, and what lives where.

## The problem

`cargo install --locked` builds a tool with the Cargo.lock its maintainer published. When one of its dependencies gets a security fix, you wait for the maintainer to cut a new release. Until then, every build of that tool in your project and your CI pulls in the vulnerable version.

## The idea

`cargo project-tools` keeps a Cargo.lock per tool in your repository. You own it, so you can:

- audit it with cargo-audit, like your own project's lock,
- bump a vulnerable dependency yourself, without waiting for a release,
- review every change to it in a pull request,
- install the tools into the project, built from exactly that lock.

`install` always passes `--locked`, so cargo refuses a lock that doesn't fit the tool's manifest.

## Layout

```text
<project root>
├── project-tools.toml        # which tools, which versions, build settings
└── .project-tools
    ├── crates
    │   └── <tool>
    │       ├── Cargo.lock          # your lock for this tool
    │       └── audit-ignore.toml   # accepted advisories, with reasons
    ├── bin                   # installed binaries
    ├── targets               # build directories, one per tool
    └── advisory-db           # cargo-audit's database
```

Commit `project-tools.toml` and `.project-tools/crates/`. Everything else is git-ignored; the tool keeps `.project-tools/.gitignore` up to date.

## Where a lock comes from

- **crates.io**: `add` downloads the `.crate` from the sparse index, checks it against the index checksum, and takes the `Cargo.lock` inside it. A crate that ships no `Cargo.lock` can't be added.
- **git**: `add --git <https-url>` clones the repository (https only) at the given tag, rev or branch.

## Changing a lock

- `update` runs `cargo update` against the tool's manifest with your lock. The new lock is only saved if cargo resolves it.
- `upgrade` moves the tool to a new version. It starts from the new release's lock and carries your dependency bumps forward where they still resolve.

## Installing

`install` builds each tool with `cargo install --path … --locked`, using your lock and the build settings from `project-tools.toml`. It writes a stamp per tool. When the version, lock and settings match the stamp, the tool is skipped.

## Supply chain

- Downloads and git clones are https only, with normal TLS verification.
- Every `.crate` is checked against the index checksum before use, and archive paths that escape the crate directory are rejected.
- Package downloads run through [Socket Firewall Free](https://docs.socket.dev/docs/socket-firewall-free) when `sfw` is installed. See [Configuration](configuration.md#sfw).
