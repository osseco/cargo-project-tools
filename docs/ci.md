# CI

Install the project's tools in GitHub Actions, and audit their locks.

## Install the tools

Binaries and build directories are cached separately. `install` runs on every run: it audits the locks, then skips each tool whose stamp and checksums in `.project-tools/bin` still match, so a restored bin cache is enough to build nothing. The bin cache falls back to an older entry with `restore-keys`, and then only the tools that changed are rebuilt. With `--message-format json`, each tool gets a `built` or an `up-to-date` record, so the job can tell what a cache hit left alone:

```bash
cargo project-tools install --message-format json > records.jsonl
jq -r 'select(.kind == "built" or .kind == "up-to-date") | "\(.tool) \(.kind)"' records.jsonl
```

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

A restored bin cache vouches for itself: the stamp and checksums that let `install` skip a tool come from the same cache. GitHub only lets a branch restore caches written by itself or by the default branch, so keep writing the caches only from runs on the default branch that you trust.

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

## Check without building

`cargo project-tools check` runs everything `install` checks before it builds: the runner against each tool's [`targets`](configuration.md#targets), the audit, and the [build review](build-review.md) of each tool that isn't up to date. It builds nothing and records nothing, reports every problem in one run, and fails if there is any. Run it instead of `audit` to also catch a lock whose build-time code nobody reviewed, or before `install` to fail before the first build starts.

Checking the build review vendors the dependencies of each tool that isn't up to date, so restore the bin cache from above first: a tool whose stamp matches is skipped, and the cached `bin/.stamps/<tool>.built.json` saves the downloads `audit` needs. The build cache isn't needed, and vendored sources are left out of it anyway.

Add `--message-format json` to get each problem as a JSON record on stdout, with its `file` (and `line`, where it points into the file) and what to put there, for example to turn them into annotations. Pin the records' `version`. See [JSON output](commands.md#json-output).

A failure names the files to change. Never fix it on the runner with `--auto-approve` or `--approve-target`: review where you work (`cargo project-tools review`), then commit `build-review.toml` or `project-tools.toml`. To make sure those commits get a person's review, see [Who approved it](build-review.md#who-approved-it).

## Build sandbox in CI

macOS runners have `sandbox-exec`. On Linux runners, install bubblewrap first. Ubuntu 24.04 also restricts the user namespaces it needs; allow them for the job:

```yaml
- run: |
    sudo apt-get install -y bubblewrap
    sudo sysctl -w kernel.apparmor_restrict_unprivileged_userns=0
```

Then require it, either in `[build]` with `sandbox = "require"`, or for the job only with `--sandbox require` on `check` and `install` (without editing `project-tools.toml`). A runner without a working sandbox then fails instead of building without one. Windows has no sandbox, so give it `windows = "off"` (or `"auto"`) under `[build.sandbox-os]`, or pass `--sandbox off` there. See [Configuration](configuration.md#sandbox).

`check` probes the sandbox for every tool, even one that's up to date, and `install` probes it before downloading anything. So a missing sandbox fails the job early. Under `auto` it's a `sandbox-missing` record (with the `tool`, and the `file` and `line` of the setting that applied). After each build, `install` writes a `built` record with the `sandbox` it used (`sandbox-exec`, `bwrap` or `none`), so the job can check that every tool really was sandboxed:

```bash
cargo project-tools install --message-format json > records.jsonl
jq -e 'select(.kind == "built" and .sandbox == "none") | halt_error' records.jsonl
```

## Offline builds

Downloading needs the network: the tool's source, the advisory database, and vendoring each tool's dependencies. `check` does all of it, and keeps each tool's source and vendored dependencies in `.project-tools/targets/<tool>` once its review passes. After that, `install --offline --no-audit` needs no network and nothing from `CARGO_HOME` but the toolchain. It builds from what `check` prepared, as long as the lock, settings, targets and review are the same, always with cargo's `--offline`. It writes only to `.project-tools/targets/<tool>`, `.project-tools/bin` and cargo's `.crates.toml` and `.crates2.json`. A tool that isn't up to date and wasn't prepared fails instead of fetching, and an offline `install` doesn't use sfw. What `check` prepared is used once, because review decisions are applied to it in place; run `check` again before another offline build. `check` skips tools that are up to date, so for a forced offline rebuild (to see that the build is reproducible, say), run `check --force` and then `install --offline --force`.

```bash
cargo project-tools check
# Then, with the network cut and CARGO_HOME read-only:
cargo project-tools install --offline --no-audit --sandbox off --message-format json > records.jsonl
```

The [sandbox](configuration.md#sandbox) enforces no network and no writes outside those paths for the build where there is one. Where there isn't, for example a container you provide, cut its network and mount `CARGO_HOME` read-only yourself, and pass `--sandbox off`. Each tool gets a `built` or `up-to-date` record.

## Socket Firewall in CI

With `settings.sfw = "auto"`, downloads go through sfw only when it's on `PATH`, and otherwise carry on with only a note. For CI, install it with [`osseco/setup-sfw-free`](https://github.com/osseco/setup-sfw-free) and set `sfw = "require"`, so a runner without it fails instead of downloading unfiltered.

If you keep `auto`, a missing sfw is also an `sfw-missing` record with `--message-format json`, which you can turn into your CI's own warning. It's written once, when a command that may download packages starts (`add`, `update`, `upgrade`, `install`, `check`, `review`, `audit`, `inspect` and `verify --repair`; `install` and `verify --repair` not with `--offline`), whether or not that run ends up downloading anything:

```bash
cargo project-tools install --message-format json > records.jsonl
jq -r 'select(.kind == "sfw-missing") | "::warning::\(.message)"' records.jsonl
```

On Windows runners, also set `CARGO_HTTP_CHECK_REVOKE: "false"` in the job's `env:`. See [Configuration](configuration.md#sfw) for why.
