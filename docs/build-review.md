# Build review

Building a tool runs code before the tool itself ever runs: build scripts and proc macros of the tool and its dependencies, and any cargo config the tool's source ships, which can swap `rustc`, add a wrapper or a linker, or set environment variables. `install` builds only when each piece of it is recorded in the tool's `build-review.toml`, as approved, overridden or stripped.

## What is reviewed

| Kind | What | Pinned by |
| --- | --- | --- |
| `build-script` | The `build.rs` (or `package.build`) of the tool and of every dependency it builds | sha256 of the script file; when the script isn't at the package root, of every file in its directory |
| `proc-macro` | Every proc-macro crate it builds | The crate checksum from `Cargo.lock`; for a git package its commit |
| `cargo-config` | `.cargo/config.toml` and `.cargo/config` in the tool's source, from the package directory up to the root of the checkout | sha256 of the file |

Only what the tool builds with its [`features` and `default-features`](configuration.md#toolname) counts, for the [`targets`](configuration.md#targets) you chose, so one review holds on each of them. Without `targets` that's every target, including Windows-only crates when you only build on Linux. Installing on a machine that isn't listed needs `--approve-target`, and then usually `--approve-build` for what that target adds.

Cargo never reads config from inside dependencies, and a config with `include`, or one that is a link, is refused. A shipped `rust-toolchain` file is always removed instead.

## The file

`.project-tools/crates/<tool>/build-review.toml`, next to the lock. Commit it: it's the record of what you looked at.

```toml
[[build-script]]
package = "libc"
path = "build.rs"
sha256 = "…"
action = "approve"

[[build-script]]
package = "ring"
path = "build.rs"
sha256 = "…"
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

## Reviewing on the terminal

When something isn't recorded and both input and output are a terminal, `add`, `upgrade`, `update` and `install` open a review screen. Elsewhere, or with `--approve-build`, they behave as described under [Approving](#approving).

| Part | What it does |
| --- | --- |
| Tree (left) | The pending pieces by kind, and their files. A build script outside its package root shows its directory, which is what its sha256 covers. A proc macro shows its whole crate. `?` undecided, `✓` approve, `⊘` strip, `✎` override. A changed piece shows its previous decision and starts with it. |
| Decision (top right) | Approve, Strip or Override for the selected piece. A proc macro can only be approved. |
| Editor (right) | The selected file, with vim keys and Rust and TOML colours. Shipped files are view only. |
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

The screen escapes anything that could hide, reorder or disguise text. Control characters show as `\x1b`. These show as `<U+202E>`:

- bidi controls, zero-width characters and other format characters, including tag characters
- variation selectors and characters that draw nothing
- spaces other than the plain one, and line and paragraph separators
- private-use characters and noncharacters

This also covers file names and link targets in the tree.

A file that needed escaping is marked `⚠` in the tree and gets a warning, and it can't be edited on the screen, so an override saved there only ever holds text you saw. Choosing Override for such a file writes a copy of the original with every line commented out under an `UNREVIEWED COPY` header, for you to edit in your own editor. An override already in place is kept. `install` refuses an override while that header is still there.

Look-alike letters from other scripts (a Cyrillic `а` for a Latin `a`) aren't escaped, since that would hide ordinary non-English text.

### Done and Cancel

**Done** records the decisions in `build-review.toml` and removes entries that no longer match anything. It first checks that nothing changed on disk while you were reviewing. For a proc macro, that covers every file of its crate. The command then still stops if any piece is undecided, or set to Override without a saved file. Run it again to decide on the rest.

**Cancel** records nothing and lists what's pending, as below.

## Approving

Outside a terminal (in CI, or with input or output redirected), `add`, `upgrade`, `update` and `install` stop before writing or building, and list each piece with the file to look at:

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
error: 1 piece of build-time code isn't recorded in .project-tools/crates/cargo-nextest/build-review.toml; review it, then rerun with --approve-build
```

After reviewing, run the same command with `--approve-build`. New pieces are recorded as `approve`. A changed piece keeps its action and file, and its entry is updated in place, so comments next to it stay. Entries that no longer match anything are removed, for example after you narrow `targets`. Without the flag they're only reported. `install --approve-build` reviews even when the tool is up to date.

To decide piece by piece, copy the printed entries into the file, set each `action` (and `file` for `override`), and rerun. For a changed piece, replace the old entry named in the comment. To change a recorded decision, edit its `action` (and `file`) and run `install`. A changed decision makes `install` rebuild the tool from scratch, since cargo doesn't notice edits to vendored sources.

## Limits

A build script's sha256 covers its own file, or its directory when it isn't at the package root. Code it pulls in from elsewhere in the crate with `include!` or `#[path]` isn't part of it, but the crate as a whole is still checked against the lock.
