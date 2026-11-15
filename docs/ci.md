# CI

Install the project's tools in GitHub Actions, and audit their locks.

## Install the tools

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

## Audit the locks

Use a separate job that installs cargo-project-tools and `cargo-audit`, and caches `.project-tools/advisory-db` with a weekly key. `audit` only fetches the database when it's missing or you pass `--update-db`:

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

Add `--deny-warnings` to also fail on unmaintained, unsound and yanked dependencies.

## Socket Firewall in CI

With `settings.sfw = "auto"`, downloads go through sfw only when it's on `PATH`. To filter them in CI too, install it first with [`osseco/setup-sfw-free`](https://github.com/osseco/setup-sfw-free), and set `sfw = "require"` if a missing sfw should fail the job.
