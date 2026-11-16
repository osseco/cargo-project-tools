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

      - name: Verify cached tools
        if: steps.tools.outputs.cache-hit == 'true'
        working-directory: .project-tools/bin
        run: sha256sum -c -- *.sha256

      - run: echo "$GITHUB_WORKSPACE/.project-tools/bin" >> "$GITHUB_PATH"
```

`install` writes a `<tool>.sha256` for every tool into `.project-tools/bin`, so the restored binaries can be checked with `sha256sum` without installing cargo-project-tools. [`cargo project-tools verify`](commands.md#verify) does the same check. With `--repair` it rebuilds a tool that fails and requires the rebuild to match; that only works reliably when `.project-tools/targets` is restored too (see [When a repair can match](commands.md#when-a-repair-can-match)).

## Audit the locks

Use a separate job that installs cargo-project-tools and runs `audit`. It needs no other tools and builds nothing. Caching `.project-tools/advisory-db` is optional: a cached copy is revalidated with its ETag, so an unchanged database isn't downloaded again.

```yaml
      - id: week
        run: echo "week=$(date +%G-%V)" >> "$GITHUB_OUTPUT"
      - uses: actions/cache@55cc8345863c7cc4c66a329aec7e433d2d1c52a9 # v6.1.0
        with:
          path: .project-tools/advisory-db
          key: advisory-db-${{ steps.week.outputs.week }}
      - run: cargo install --git https://github.com/osseco/cargo-project-tools --locked
      - run: cargo project-tools audit
```

Add `--deny-warnings` to also fail on unmaintained, unsound, yanked and git-source warnings. `install` runs the same audit before it builds, so a separate audit job is only needed when the job doesn't install.

## Socket Firewall in CI

With `settings.sfw = "auto"`, downloads go through sfw only when it's on `PATH`. To filter them in CI too, install it first with [`osseco/setup-sfw-free`](https://github.com/osseco/setup-sfw-free), and set `sfw = "require"` if a missing sfw should fail the job.

On Windows runners, also set `CARGO_HTTP_CHECK_REVOKE: "false"` in the job's `env:`. See [Configuration](configuration.md#sfw) for why.
