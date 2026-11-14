# cargo-project-tools

Install your project's cargo tools from **your own Cargo.lock files**.

`cargo install --locked` builds a tool with the Cargo.lock its maintainer published. When a dependency gets a security fix, you wait for a new release. `cargo project-tools` keeps a Cargo.lock per tool in your repository, so you can audit it, bump vulnerable dependencies yourself, and install the tools into the project.

## How it works

```
<project root>
├── project-tools.toml        # which tools, which versions, build settings
└── .project-tools
    ├── crates
    │   └── <tool>
    │       ├── Cargo.lock          # your lock for this tool
    │       └── audit-ignore.toml   # accepted advisories, with reasons
    ├── bin                   # installed binaries
    └── targets               # build directories, one per tool
```

Commit `project-tools.toml` and `.project-tools/crates/`. Everything else is git-ignored (the tool keeps `.project-tools/.gitignore` up to date).

## Getting started

Download a binary for your platform from the [latest release](https://github.com/osseco/cargo-project-tools/releases/latest), check it, and put it on your `PATH`:

```bash
gh attestation verify cargo-project-tools-x86_64-unknown-linux-musl --repo osseco/cargo-project-tools
```

Every asset is also signed with cosign; see [CONTRIBUTING.md](CONTRIBUTING.md#verifying-a-release). Or build from source:

```bash
cargo install --git https://github.com/osseco/cargo-project-tools --locked
```

Then, in your project:

```bash
cargo project-tools init                 # create project-tools.toml and .project-tools/
cargo project-tools add cargo-nextest    # starts from the lock the tool ships with
cargo project-tools audit                # needs cargo-audit
cargo project-tools update cargo-nextest <dep>@<old> --precise <new>   # as suggested by audit
cargo project-tools install              # builds into .project-tools/bin
```

Add `.project-tools/bin` to your `PATH`.

## Commands

| Command | What it does |
| --- | --- |
| `init` | Create `project-tools.toml` and `.project-tools/` |
| `add <crate>[@ver]` | Add a tool (`--git <https-url> --tag/--rev/--branch` for git sources) |
| `remove <crate>` | Remove a tool, its lock, build dir and binaries |
| `upgrade <crate>` | Move to a new tool version, keeping your dependency bumps where they still resolve |
| `update <crate> [deps…]` | Update dependencies in the tool's lock; the lock is only saved if cargo resolves it |
| `install [crates…]` | Build and install; skipped when nothing changed |
| `audit [crates…]` | cargo-audit every lock, apply `audit-ignore.toml`, suggest fixes |
| `audit-db update` | Refresh the advisory database |
| `list` | Show tools and their state |

## Configuration

```toml
[settings]
sfw = "auto"                 # "auto", "require" or "off"

[build]
profile = "release"

[build.profile-settings]     # applied to every tool
strip = "symbols"
lto = "thin"
codegen-units = 1

[tool.cargo-nextest]
version = "0.9.80"

[tool.cargo-nextest.build.profile-settings]
panic = "unwind"             # per-tool override
```

`.project-tools/crates/<tool>/audit-ignore.toml`:

```toml
[[ignore]]
id = "RUSTSEC-2024-0001"     # or a CVE/GHSA alias
reason = "only affects a feature we don't enable"
expires = "2027-01-01"       # optional
```

`audit` warns about ignores that no longer match anything, and about expired ones.

Package downloads go through [Socket Firewall Free](https://docs.socket.dev/docs/socket-firewall-free) when `sfw` is installed (`settings.sfw`, or `--no-sfw`).

## In CI (GitHub Actions)

Binaries and build directories are cached separately. When the bin cache key matches, nothing is built. The key covers `project-tools.toml` and every lock, so any change to a tool's version, lock or build settings rebuilds it.

```yaml
jobs:
  tools:
    runs-on: ubuntu-latest
    permissions:
      contents: read
    steps:
      - uses: actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1 # v7.0.1
        with:
          persist-credentials: false

      - name: Cache tool binaries
        id: tools
        uses: actions/cache@55cc8345863c7cc4c66a329aec7e433d2d1c52a9 # v6.1.0
        with:
          path: |
            .project-tools/bin
            .project-tools/.crates.toml
            .project-tools/.crates2.json
          key: tools-bin-${{ runner.os }}-${{ hashFiles('project-tools.toml', '.project-tools/crates/**/Cargo.lock') }}

      - name: Cache tool builds
        if: steps.tools.outputs.cache-hit != 'true'
        uses: actions/cache@55cc8345863c7cc4c66a329aec7e433d2d1c52a9 # v6.1.0
        with:
          path: .project-tools/targets
          key: tools-targets-${{ runner.os }}-${{ hashFiles('project-tools.toml', '.project-tools/crates/**/Cargo.lock') }}
          restore-keys: tools-targets-${{ runner.os }}-

      - name: Install tools
        if: steps.tools.outputs.cache-hit != 'true'
        run: |
          cargo install --git https://github.com/osseco/cargo-project-tools --locked
          cargo project-tools install

      - run: echo "$GITHUB_WORKSPACE/.project-tools/bin" >> "$GITHUB_PATH"
```

To audit the locks too, use a separate job that installs cargo-project-tools and `cargo-audit`, and caches `.project-tools/advisory-db` with a weekly key. `audit` only fetches the database when it's missing or you pass `--update-db`:

```yaml
      - id: week
        run: echo "week=$(date +%G-%V)" >> "$GITHUB_OUTPUT"
      - uses: actions/cache@55cc8345863c7cc4c66a329aec7e433d2d1c52a9 # v6.1.0
        with:
          path: .project-tools/advisory-db
          key: advisory-db-${{ steps.week.outputs.week }}
      - uses: taiki-e/install-action@cfdb7ed4ccc7c7dce37759eae89dc5674b469b39 # v2.90.3
        with:
          tool: cargo-audit
      - run: cargo install --git https://github.com/osseco/cargo-project-tools --locked
      - run: cargo project-tools audit
```

## License

To be decided.
