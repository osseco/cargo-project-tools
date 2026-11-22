# How it works

Why each tool gets a Cargo.lock in your repository, and what lives where.

## The problem

`cargo install --locked` builds a tool with the Cargo.lock its maintainer published. When one of its dependencies gets a security fix, you wait for the maintainer to cut a new release. Until then, every build of that tool in your project and your CI pulls in the vulnerable version.

## The idea

`cargo project-tools` keeps a Cargo.lock per tool in your repository. You own it, so you can:

- audit it against the RustSec advisory database, like your own project's lock,
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
    │       ├── audit-ignore.toml   # accepted advisories, with reasons
    │       ├── build-review.toml   # approved, overridden or stripped build-time code
    │       └── overrides/          # your replacement files (optional)
    ├── self-audit-ignore.toml  # accepted advisories against cargo-project-tools (optional)
    ├── bin                   # installed binaries, <tool>.sha256 checksums, .stamps/
    ├── targets               # build directories, one per tool
    └── advisory-db           # RustSec advisory database
```

Commit `project-tools.toml`, `.project-tools/crates/` and `.project-tools/self-audit-ignore.toml` if you have one. Everything else is git-ignored; the tool keeps `.project-tools/.gitignore` up to date.

## Where a lock comes from

- **crates.io**: `add` downloads the `.crate` from the sparse index, checks it against the index checksum, and takes the `Cargo.lock` inside it.
- **git**: `add --git <https-url>` clones the repository (https only) at the given tag, rev or branch.
- **No `Cargo.lock` shipped**: `add` runs `cargo generate-lockfile`. It only queries the index, so nothing is compiled and no build script runs. Versions are the newest that fit; with resolver 3 (or `incompatible-rust-versions = "fallback"`) cargo also honours `rust-version`, so the result depends on your cargo.

## Changing a lock

- `update` runs `cargo update` against the tool's manifest with your lock. The new lock is only saved if cargo resolves it.
- `update --latest` (or `--version`) moves the tool to a new version. It starts from the new release's lock and carries your dependency bumps forward where they still resolve. If the new release ships no lock, it starts from yours.
- `add -i` and `update -i` do this on a [setup screen](commands.md#interactive-setup): features, the tool's version and single dependencies change there, with `cargo tree` and the advisory database showing the result as you go.

## Installing

`install` first audits the locks of the tools it's about to build, and builds nothing if one of them has a vulnerability you haven't ignored. It vendors each tool's dependencies into `targets/<tool>/vendor` and checks the build scripts, proc macros and shipped cargo config against the tool's [`build-review.toml`](build-review.md). Then it builds each tool with `cargo install --path … --locked`, using your lock and the build settings from `project-tools.toml`. It writes a stamp per tool to `bin/.stamps/<tool>.json`, and the sha256 of each binary to `bin/<tool>.sha256`. When the version, lock, settings, OS and architecture match the stamp and the binaries match their checksums, the tool is skipped. Both files sit next to the binaries, so restoring `bin` from a cache is enough to skip a build.

The source of each tool is kept in `.project-tools/targets/<tool>/src`. A `.crate` is unpacked with its original file times, and a git tool already at its pinned commit is reset in place instead of cloned again (only a clone it made itself, recorded in `targets/<tool>/src.clone`). Vendored dependencies all get the same old file time, as they're copied afresh each time and build scripts watching them would otherwise run again. So with an unchanged build directory cargo finds nothing to rebuild and copies the binary it already has. [`verify --repair`](commands.md#verify) relies on that.

When build-time code changed since its entry was recorded, the version the entry names is fetched into `targets/<tool>/previous` to compare with: the `.crate` with that checksum (checked like any other), or a clone at that commit. It's only read, never built. This happens outside a terminal and for `review --diff`, not on the review screen or in `check`.

## Auditing

`audit` reads the [RustSec advisory database](https://github.com/rustsec/advisory-db) with the [rustsec](https://crates.io/crates/rustsec) library, which cargo-audit is built on. It downloads the database as a `git archive` tarball of the default branch over https, and keeps the ETag, the commit and the time of the last check in `.project-tools/advisory-db/.fetch.json`. A new download is unpacked next to the old copy, checked, and only then swapped in. As with cargo-audit's git fetch, the database is trusted because it comes from GitHub over https; there's no checksum to check it against.

Unlike cargo-audit, it also checks the root package of each lock, which is the tool itself, and git packages by name and version.

Which packages a tool builds comes from `cargo tree` and is kept in `bin/.stamps/<tool>.built.json`, keyed by the tool's version, lock, features and targets. A file that doesn't match is computed again. Like the install stamps, it's trusted as found in `bin`, so restore `bin` only from caches your own runs wrote.

## Supply chain

- Downloads and git clones are https only, with normal TLS verification. A lock with a source that isn't https (`git+ssh://`, `git://`, `file://`, plain http) is refused, and cargo fetches git dependencies with the git command, limited to https.
- Every `.crate` is checked against the index checksum before use. Archive paths that escape the crate directory are rejected, and so is an archive with a `.git` in it, whose config could make git run commands.
- A git tool's clone is only reused when this tool made it, at that commit and from that url. Git runs with no hooks and no fsmonitor, whatever a repository's config says.
- The build runs in an OS sandbox where there is one (macOS, and Linux with bubblewrap): no network, and writes only to the tool's build directory and `.project-tools/bin`. See [Configuration](configuration.md#sandbox).
- The build sets `GIT_CEILING_DIRECTORIES` to `.project-tools/targets/<tool>`. A build script that runs git, for example to embed a commit, sees the tool's own clone, but never your project's repository.
- Package downloads run through [Socket Firewall Free](https://docs.socket.dev/docs/socket-firewall-free) when `sfw` is installed. See [Configuration](configuration.md#sfw).
