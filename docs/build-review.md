# Build review

Building a tool runs code before the tool itself ever runs: build scripts and proc macros of the tool and its dependencies, and any cargo config the tool's source ships, which can swap `rustc`, add a wrapper or a linker, or set environment variables. `install` builds only when each piece of it is recorded in the tool's `build-review.toml`, as approved, overridden or stripped.

## What is reviewed

| Kind | What | Pinned by |
| --- | --- | --- |
| `build-script` | The `build.rs` (or `package.build`) of the tool and of every dependency it builds | sha256 of the script file; when the script isn't at the package root, of every file in its directory (a vendored package's `.git` included); plus [the files it pulls in](#files-a-build-script-pulls-in). And the crate it comes with: its checksum from `Cargo.lock`, or for a git package its commit |
| `proc-macro` | Every proc-macro crate it builds | The crate checksum from `Cargo.lock`; for a git package its commit |
| `cargo-config` | `.cargo/config.toml` and `.cargo/config` in the tool's source, from the package directory up to the root of the checkout | sha256 of the file |

Only what the tool builds with its [`features` and `default-features`](configuration.md#toolname) counts, for the [`targets`](configuration.md#targets) you chose, so one review holds on each of them. Without `targets` that's every target, including Windows-only crates when you only build on Linux. Installing on a machine that isn't listed needs `--approve-target`, and then usually `--approve-build` for what that target adds. What's built is worked out with the build's own `rustflags` and `--config`, since a `--cfg` can turn on platform-specific dependencies.

Cargo never reads config from inside dependencies, and a config with `include`, or one that is a link, is refused. So is a config that sets `[source]`, `[patch]`, `paths` or `build.target`, which would build something other than what was reviewed. A `.cargo` between the checkout and the project root (in `.project-tools/targets/<tool>`, `.project-tools/targets` or `.project-tools`) is refused too: cargo would read it, and nobody reviewed it. A shipped `rust-toolchain` file is always removed instead.

A package whose directory leads outside the tool's checkout and vendored sources, through a link or `..`, is refused, so an override or strip never writes anywhere else.

### Files a build script pulls in

A build script can run code from other files in its package. Those count as part of it, and are followed through each file in turn:

- its modules: `mod x;` (`x.rs` or `x/mod.rs`), `#[path = "…"]`, and `#[cfg_attr(…, path = "…")]`, with every possible file taken
- what it includes: `include!`, `include_str!` and `include_bytes!`, with a plain path or `concat!(env!("CARGO_MANIFEST_DIR"), "…")`. A file under `OUT_DIR`, as `concat!(env!("OUT_DIR"), "…")` without `..`, is written by the script itself and is skipped.

When that can't be worked out for sure, its sha256 covers the whole package instead, and the review screen says why. That happens when:

- a path is computed some other way, or mentions `OUT_DIR` in any other way;
- a module or included file it names isn't there;
- a file can't be parsed;
- a module is declared inside a macro or a function, or a module file (`mod x;`) in an included file;
- an include macro is renamed (`use std::include as inc;`), used inside `macro_rules!`, or could be passed in to one (`$m!(…)`);
- a `path` is nested in more than one `cfg_attr`.

A file outside the package, inside `.git`, or a link, is refused.

A script that pulls in files has a different sha256 than its file alone, so its entry comes up once as changed.

## The file

`.project-tools/crates/<tool>/build-review.toml`, next to the lock. Commit it: it's the record of what you looked at.

```toml
[[build-script]]
package = "libc"
path = "build.rs"
sha256 = "…"
checksum = "…"
action = "approve"

[[build-script]]
package = "ring"
path = "build.rs"
sha256 = "…"
checksum = "…"
action = "override"
file = "overrides/ring-build.rs"

[[proc-macro]]
package = "serde_derive"
checksum = "…"
action = "approve"

[[cargo-config]]
path = ".cargo/config.toml"
sha256 = "…"
action = "strip"
```

| `action` | Effect |
| --- | --- |
| `approve` | Build it as shipped |
| `override` | Replace it with `file`, relative to `.project-tools/crates/<tool>/` |
| `strip` | Remove it: a build script is turned off with `build = false`, a config file is deleted. Cargo refuses to strip the build script of a package with `links` |

A proc macro can only be approved. To avoid one, turn off the feature that pulls it in.

`sha256` and `checksum` always describe the shipped content, also for `override` and `strip`, so a change upstream stops the build again and you can check that your override still fits.

A build script's entry also names its crate: `checksum` for a registry package, `commit` for a git one, neither for a package of the tool's own checkout. A build script can run or read any file of its crate, so a new version of the crate comes up for review again even when the script itself didn't change. Entries recorded before this come up once as changed, and keep their action.

## Reviewing on the terminal

When something isn't recorded and both input and output are a terminal, `add`, `update` and `install` open a review screen. Elsewhere, or with `--approve-build`, they behave as described under [Approving](#approving).

| Part | What it does |
| --- | --- |
| Tree (left) | The pending pieces by kind, and their files. A build script outside its package root shows its directory, which is what its sha256 covers. A proc macro shows its whole crate. `?` undecided, `✓` approve, `⊘` strip, `✎` override. A changed piece shows its previous decision, and starts on it when that was strip or override; one that was approved starts undecided, since its new content needs a look. One [reviewed for another tool](#reviewed-for-another-tool) says so and starts with that tool's decision. |
| Decision (top right) | Approve, Strip or Override for the selected piece. A proc macro can only be approved. |
| Editor (right) | The selected file, with vim keys and Rust and TOML colours. Shipped files are view only, and long lines wrap, so nothing hides past the right edge. Choosing Override opens the piece's own file, where an existing override shows. |
| Title bar | How many pieces are approved, overridden, stripped and undecided. |
| Buttons (bottom) | Tree, Editor, Save override, Done, Cancel. |

The mouse works everywhere. On the keyboard, `Ctrl-w` and then `h`, `l`, `k` or `j` moves to the tree, the editor, the decision or the buttons. In the tree, `j`/`k` move and `l`/`h` unfold and fold. With the decision or the buttons focused, `h`/`l` choose and `Enter` picks. `Ctrl-c` cancels.

### Overriding

With Override chosen, the build script or config file itself can be edited. **Save override** writes it to `overrides/<package>/<path>`, or `overrides/cargo-config/<path>` for config, in the tool's crate directory. Leaving an edited file asks whether to save it.

Saving is careful with what's already there:

- An override file you didn't open (one that appeared while you were editing) is only replaced after you say yes.
- An override that changed on disk since you opened it isn't replaced. Discard your edits to load it again.
- No link is followed anywhere under the crate directory. The file is written to a temporary file and renamed into place.

### What you see is what's there

The screen escapes anything that could hide, reorder or disguise text. Control characters show as `\x1b`; a Windows line ending (CRLF) is an ordinary line end, but a carriage return on its own, which can draw over its line, is escaped too. These show as `<U+202E>`:

- bidi controls, zero-width characters and other format characters, including tag characters
- variation selectors and characters that draw nothing
- spaces other than the plain one, and line and paragraph separators
- private-use characters and noncharacters

This also covers file names and link targets in the tree.

A file that needed escaping is marked `⚠` in the tree and gets a warning, and it can't be edited on the screen, so an override saved there only ever holds text you saw. Choosing Override for such a file writes a copy of the original with every line commented out under an `UNREVIEWED COPY` header, for you to edit in your own editor. An override already in place is kept. `install` refuses an override while that header is still there.

Look-alike letters from other scripts (a Cyrillic `а` for a Latin `a`) aren't escaped, since that would hide ordinary non-English text.

### Reviewed for another tool

The same piece (kind, package, path, sha256 and crate) is often used by several tools, for example a common dependency's build script. When another tool in `project-tools.toml` already records it, the screen starts with that decision and the tree says `approved in <tool>`, `stripped in <tool>` or `overridden in <tool>`. It's still only recorded when you press Done.

Only tools in `project-tools.toml` count, not every directory under `.project-tools/crates`. When those tools decided differently on a piece, it starts undecided. And another tool's decision never replaces this tool's own earlier strip or override of a changed piece.

For an override, the editor shows that tool's override file. Done copies it to this tool's `overrides/`, only if it still has the content it had when the screen opened. If this tool already has a different file there, yours is kept and the piece starts undecided. An override reached through a link under that tool's directory isn't used.

### Done and Cancel

**Done** records the decisions in `build-review.toml` and removes entries that no longer match anything. It first checks that nothing changed on disk while you were reviewing. For a proc macro, that covers every file of its crate. The command then still stops if any piece is undecided, or set to Override without a saved file. Run it again to decide on the rest.

**Cancel** records nothing and lists what's pending, as below.

## Approving

Outside a terminal (in CI, or with input or output redirected), `add`, `update` and `install` stop before writing or building, and list each piece with the file to look at, on stderr like the error:

```text
  build-time code to review:
    build-script zstd-sys@2.1.2+zstd.1.5.7 build.rs (changed; was approve)  .project-tools/targets/cargo-nextest/vendor/zstd-sys-2.1.2+zstd.1.5.7/build.rs
  to decide piece by piece, put these in .project-tools/crates/cargo-nextest/build-review.toml with the action you choose:
    # replaces the entry with sha256 = "5b0c…"
    [[build-script]]
    package = "zstd-sys"
    path = "build.rs"
    sha256 = "9e41…"
    action = "approve"
error: 1 piece of build-time code isn't recorded in .project-tools/crates/cargo-nextest/build-review.toml; review it where you have a terminal (run this command there for the review screen, or with --approve-build after reading the files), then commit .project-tools/crates/cargo-nextest/build-review.toml
```

Don't approve on a CI runner: review where you work, and commit `build-review.toml`. After Cancel on the review screen, the error says to rerun with `--approve-build` instead.

A piece another tool already records is listed with that tool's decision (`reviewed for <tool>: strip`), and its printed entry uses it. For an override, the entry also says which file to copy.

After reviewing, run the same command with `--approve-build`. New pieces are recorded as `approve`. A changed piece keeps its action and file, and its entry is updated in place, so comments next to it stay. Entries that no longer match anything are removed, for example after you narrow `targets`. Without the flag they're only reported. `install --approve-build` reviews even when the tool is up to date.

To decide piece by piece, copy the printed entries into the file, set each `action` (and `file` for `override`), and rerun. For a changed piece, replace the old entry named in the comment. To change a recorded decision, edit its `action` (and `file`) and run `install`. A changed decision makes `install` rebuild the tool from scratch, since cargo doesn't notice edits to vendored sources.

## Limits

A build script's files are found by reading its source, not by compiling it. A file it reads at run time (`std::fs::read`, say) isn't part of its sha256, but its entry names the crate's checksum or commit, so any change to the crate comes up again.

The libraries that build-time code runs aren't reviewed yet: a build script's build-dependencies (`cc`, `autocfg`, …) and a proc macro's own dependencies (`syn`, `quote`, …). They're checked against the lock and audited like any dependency, but a new version of one doesn't come up for review.

On a nightly toolchain, a manifest can use `cargo-features` to run code without a build script (a custom codegen backend, say). That isn't detected.
