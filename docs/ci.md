# CI

Install the project's tools in GitHub Actions, and audit their locks.

## Install the tools

Binaries and build directories are cached separately. `install` runs on every run: it audits the locks, then skips each tool whose stamp and checksums in `.project-tools/bin` still match, so a restored bin cache is enough to build nothing. The bin cache falls back to an older entry with `restore-keys`, and then only the tools that changed are rebuilt.

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
          key: tools-bin-${{ runner.os }}-${{ hashFiles('project-tools.toml', '.project-tools/crates/**') }}
          restore-keys: tools-bin-${{ runner.os }}-

      - name: Cache tool builds
        if: steps.tools.outputs.cache-hit != 'true'
        uses: actions/cache@55cc8345863c7cc4c66a329aec7e433d2d1c52a9 # v6.1.0
        with:
          path: |
            .project-tools/targets
            !.project-tools/targets/*/vendor
          key: tools-targets-${{ runner.os }}-${{ hashFiles('project-tools.toml', '.project-tools/crates/**') }}
          restore-keys: tools-targets-${{ runner.os }}-

      - name: Install tools
        run: |
          cargo install --git https://github.com/osseco/cargo-project-tools --locked
          cargo project-tools install

      - run: echo "$GITHUB_WORKSPACE/.project-tools/bin" >> "$GITHUB_PATH"
```

The build directories are only restored when the bin cache isn't an exact hit, because only then can something need building. The keys hash everything in `.project-tools/crates`: the locks, and the [build review](build-review.md) decisions and overrides, which also decide whether a tool is rebuilt. Vendored sources are left out: `install` vendors them again on every build.

`install` keeps a stamp (`bin/.stamps/<tool>.json`) and a `<tool>.sha256` for every tool in `.project-tools/bin`. The stamp includes the OS and architecture, so a bin cache restored on the wrong platform is rebuilt rather than used. A job that doesn't want to install can check restored binaries with [`cargo project-tools verify`](commands.md#verify), or with `sha256sum -c *.sha256` inside `bin`, but then the locks aren't audited.

## Audit the locks

Use a separate job that installs cargo-project-tools and runs `audit`. It needs no other tools and builds nothing. To find out which dependencies a tool builds, cargo downloads them once; the answer is kept in `.project-tools/bin/.stamps`, so caching `.project-tools/bin` as above saves those downloads on later runs. Caching `.project-tools/advisory-db` is optional: a cached copy is revalidated with its ETag, so an unchanged database isn't downloaded again.

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
