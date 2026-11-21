//! The files a build script pulls in besides itself: its modules (`mod`, `#[path]`) and
//! what it includes (`include!`, `include_str!`, `include_bytes!`), followed through those
//! files too. When that can't be worked out for sure, the answer is the whole package.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use proc_macro2::{Delimiter, TokenStream, TokenTree};
use syn::ext::IdentExt;
use syn::punctuated::Punctuated;
use syn::{Expr, Lit, Meta, Token};

/// What a build script pulls in.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Pulled {
    /// Files besides the script, inside its package, sorted.
    pub files: Vec<PathBuf>,
    /// Why the whole package has to be covered instead, when it has to.
    pub whole_package: Option<String>,
}

/// Find what the build script `script` in the package at `pkg_dir` pulls in. Fails for a
/// file outside the package and for links.
pub fn find(pkg_dir: &Path, script: &Path) -> Result<Pulled> {
    let mut finder = Finder {
        pkg_dir: pkg_dir.to_path_buf(),
        real_pkg: pkg_dir
            .canonicalize()
            .with_context(|| format!("reading {}", pkg_dir.display()))?,
        files: BTreeSet::new(),
        seen: BTreeSet::new(),
        whole: None,
    };
    let dir = script
        .parent()
        .context("build script without a directory")?;
    let script = finder.check(script)?;
    finder.source(&script, dir, false)?;
    finder.files.remove(&script);
    Ok(Pulled {
        files: finder.files.into_iter().collect(),
        whole_package: finder.whole,
    })
}

struct Finder {
    pkg_dir: PathBuf,
    real_pkg: PathBuf,
    files: BTreeSet<PathBuf>,
    /// Sources already read, so a loop of includes ends.
    seen: BTreeSet<PathBuf>,
    whole: Option<String>,
}

/// Where module files are looked for, while walking one source file.
struct Ctx<'a> {
    file_dir: &'a Path,
    /// Where `mod x;` looks for `x.rs` and `x/mod.rs`.
    mod_dir: PathBuf,
    /// Inside an inline `mod x { … }`, where `#[path]` is relative to `mod_dir`.
    inline: bool,
    /// In an `include!`d file, where a `mod x;` file depends on the including module.
    included: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Include {
    Code,
    Data,
}

impl Finder {
    fn rel(&self, path: &Path) -> String {
        path.strip_prefix(&self.pkg_dir)
            .unwrap_or(path)
            .display()
            .to_string()
    }

    fn give_up(&mut self, why: String) {
        self.whole.get_or_insert(why);
    }

    /// `path` as a path in the package without `..`, after checking it's in the package and
    /// isn't a link.
    fn check(&self, path: &Path) -> Result<PathBuf> {
        let meta = path
            .symlink_metadata()
            .with_context(|| format!("reading {}", path.display()))?;
        if meta.file_type().is_symlink() {
            bail!("{} is a link; refusing to build with it", path.display());
        }
        let real = path
            .canonicalize()
            .with_context(|| format!("reading {}", path.display()))?;
        let Ok(rel) = real.strip_prefix(&self.real_pkg) else {
            bail!(
                "the build script reads {}, outside its package",
                path.display()
            );
        };
        if rel
            .components()
            .any(|c| c.as_os_str().eq_ignore_ascii_case(".git"))
        {
            bail!("the build script reads {}, inside .git", path.display());
        }
        Ok(self.pkg_dir.join(rel))
    }

    /// Take in a file that exists: checked, recorded, and for source files, read.
    fn add(&mut self, path: &Path, include: Option<Include>, mod_dir: &Path) -> Result<()> {
        let path = self.check(path)?;
        self.files.insert(path.clone());
        match include {
            Some(Include::Data) => Ok(()),
            Some(Include::Code) => self.source(&path, path.parent().unwrap_or(mod_dir), true),
            None => self.source(&path, mod_dir, false),
        }
    }

    /// Read a Rust source file whose `mod x;` declarations are looked for in `mod_dir`.
    /// `included`: it's `include!`d. Its includes resolve from its own directory, but where
    /// its `mod` files are depends on the module it lands in, which isn't followed.
    fn source(&mut self, path: &Path, mod_dir: &Path, included: bool) -> Result<()> {
        if self.whole.is_some() || !self.seen.insert(path.to_path_buf()) {
            return Ok(());
        }
        self.files.insert(path.to_path_buf());
        let rel = self.rel(path);
        let Ok(text) = fs::read_to_string(path) else {
            self.give_up(format!("{rel} isn't UTF-8"));
            return Ok(());
        };
        let Ok(tokens) = text.parse::<TokenStream>() else {
            self.give_up(format!("{rel} can't be read as Rust"));
            return Ok(());
        };
        let file_dir = path.parent().unwrap_or(mod_dir).to_path_buf();
        let mut scanned = Scan::default();
        scan(tokens, false, &mut scanned);
        if let Some(why) = scanned.unfollowable {
            self.give_up(format!("{rel} {why}"));
            return Ok(());
        }
        let mods = scanned.mods;
        for (kind, args) in scanned.includes {
            self.include(kind, args, &file_dir, &rel)?;
        }
        // Items, or an expression for an `include!`d file.
        let walked = match syn::parse_file(&text) {
            Ok(file) => {
                let ctx = Ctx {
                    file_dir: &file_dir,
                    mod_dir: mod_dir.to_path_buf(),
                    inline: false,
                    included,
                };
                self.items(&file.items, &ctx, &rel)?
            }
            Err(_) if syn::parse_str::<Expr>(&text).is_ok() => 0,
            Err(_) => {
                self.give_up(format!("{rel} can't be parsed"));
                return Ok(());
            }
        };
        // Every `mod` should be one the walk resolved; others come from inside macros or
        // function bodies, where they can't be followed.
        if mods > walked {
            self.give_up(format!("{rel} declares modules that can't be followed"));
        }
        Ok(())
    }

    /// Walk items for modules. Returns how many `mod` items were seen.
    fn items(&mut self, items: &[syn::Item], ctx: &Ctx, rel: &str) -> Result<usize> {
        let mut count = 0;
        for item in items {
            let syn::Item::Mod(m) = item else {
                continue;
            };
            count += 1;
            let Some((paths, always)) = path_attrs(&m.attrs) else {
                self.give_up(format!("{rel} has a `#[path]` that can't be read for sure"));
                continue;
            };
            let name = m.ident.to_string();
            if let Some((_, inner)) = &m.content {
                let mod_dir = match paths.first() {
                    Some(p) => ctx.mod_dir.join(p),
                    None => ctx.mod_dir.join(&name),
                };
                let inner_ctx = Ctx {
                    file_dir: ctx.file_dir,
                    mod_dir,
                    inline: true,
                    included: ctx.included,
                };
                count += self.items(inner, &inner_ctx, rel)?;
                continue;
            }
            if ctx.included {
                self.give_up(format!("{rel} is included and declares a module file"));
                continue;
            }
            let base = if ctx.inline {
                &ctx.mod_dir
            } else {
                ctx.file_dir
            };
            // A file that isn't there can't be checked: rustc may find it some other way.
            for p in &paths {
                let file = base.join(p);
                if !file.exists() {
                    self.give_up(format!("{rel} names a module file {p} that isn't there"));
                    continue;
                }
                // A file loaded through `#[path]` acts as a mod-rs file.
                let dir = file.parent().unwrap_or(base).to_path_buf();
                self.add(&file, None, &dir)?;
            }
            if !always {
                let flat = ctx.mod_dir.join(format!("{name}.rs"));
                let nested = ctx.mod_dir.join(&name).join("mod.rs");
                if flat.exists() {
                    self.add(&flat, None, &ctx.mod_dir.join(&name))?;
                } else if nested.exists() {
                    self.add(&nested, None, &ctx.mod_dir.join(&name))?;
                } else if paths.is_empty() {
                    self.give_up(format!("{rel} has a module {name} whose file isn't there"));
                }
            }
        }
        Ok(count)
    }

    fn include(
        &mut self,
        kind: Include,
        args: TokenStream,
        file_dir: &Path,
        rel: &str,
    ) -> Result<()> {
        let path = match include_path(args, &self.pkg_dir) {
            IncludePath::File(p) => file_dir.join(p),
            IncludePath::Generated => return Ok(()),
            IncludePath::Unknown => {
                self.give_up(format!("{rel} includes a file by a computed path"));
                return Ok(());
            }
        };
        if !path.exists() {
            self.give_up(format!(
                "{rel} includes {}, which isn't there",
                path.display()
            ));
            return Ok(());
        }
        self.add(&path, Some(kind), file_dir)
    }
}

/// The module's paths from `#[path = "…"]` and `#[cfg_attr(…, path = "…")]`, and whether
/// one is unconditional (else the usual file is a candidate too); `None` when one isn't a
/// string literal.
fn path_attrs(attrs: &[syn::Attribute]) -> Option<(Vec<String>, bool)> {
    let mut out = Vec::new();
    let mut always = false;
    for attr in attrs {
        if named(attr.path(), "path") {
            out.push(meta_str(&attr.meta)?);
            always = true;
        } else if named(attr.path(), "cfg_attr") {
            let metas = attr
                .parse_args_with(Punctuated::<Meta, Token![,]>::parse_terminated)
                .ok()?;
            for meta in metas.iter().skip(1) {
                if named(meta.path(), "path") {
                    out.push(meta_str(meta)?);
                } else if named(meta.path(), "cfg_attr") {
                    // A `path` nested further in isn't followed.
                    return None;
                }
            }
        }
    }
    Some((out, always))
}

/// Whether `path` is the single name `name`, written plainly or as `r#name`.
fn named(path: &syn::Path, name: &str) -> bool {
    path.get_ident().is_some_and(|i| i.unraw() == name)
}

fn meta_str(meta: &Meta) -> Option<String> {
    match meta {
        Meta::NameValue(nv) => match &nv.value {
            Expr::Lit(lit) => match &lit.lit {
                Lit::Str(s) => Some(s.value()),
                _ => None,
            },
            _ => None,
        },
        _ => None,
    }
}

/// What `scan` finds in a file's tokens.
#[derive(Default)]
struct Scan {
    includes: Vec<(Include, TokenStream)>,
    /// `mod` keywords anywhere.
    mods: usize,
    /// Why the includes can't all be followed, when they can't.
    unfollowable: Option<String>,
}

/// The include macro an identifier names, `r#` or not.
fn include_kind(id: &proc_macro2::Ident) -> Option<Include> {
    match id.unraw().to_string().as_str() {
        "include" => Some(Include::Code),
        "include_str" | "include_bytes" => Some(Include::Data),
        _ => None,
    }
}

/// Collect `include!`-family calls and count `mod` keywords, anywhere in the tokens. Some
/// includes can't be followed: one renamed (`use std::include as inc;`), one in a
/// `macro_rules!` body, whose path rustc resolves from wherever the macro is called, and a
/// macro passed in to a `macro_rules!` body (`$m!(…)`), which could be `include`. Other uses
/// of the name, like `cc::Build::include`, are fine.
fn scan(tokens: TokenStream, in_macro: bool, out: &mut Scan) {
    let tokens: Vec<TokenTree> = tokens.into_iter().collect();
    let punct =
        |i: usize, c: char| matches!(tokens.get(i), Some(TokenTree::Punct(p)) if p.as_char() == c);
    // Index of the group that is a `macro_rules!` body.
    let mut macro_body = None;
    for (i, t) in tokens.iter().enumerate() {
        match t {
            TokenTree::Ident(id) => {
                if id == "mod" {
                    out.mods += 1;
                }
                let bang = punct(i + 1, '!');
                if id.unraw() == "macro_rules" && bang {
                    macro_body = Some(i + 3);
                }
                if in_macro && bang && i > 0 && punct(i - 1, '$') {
                    out.unfollowable
                        .get_or_insert("calls a macro passed in to `macro_rules!`".into());
                }
                let Some(kind) = include_kind(id) else {
                    continue;
                };
                if matches!(tokens.get(i + 1), Some(TokenTree::Ident(a)) if a == "as") {
                    out.unfollowable.get_or_insert(format!("renames `{id}`"));
                    continue;
                }
                let Some(TokenTree::Group(g)) = tokens.get(i + 2) else {
                    continue;
                };
                if !bang || g.delimiter() == Delimiter::None {
                    continue;
                }
                if in_macro {
                    out.unfollowable
                        .get_or_insert("includes a file inside `macro_rules!`".into());
                } else {
                    out.includes.push((kind, g.stream()));
                }
            }
            TokenTree::Group(g) => scan(g.stream(), in_macro || macro_body == Some(i), out),
            _ => {}
        }
    }
}

enum IncludePath {
    File(PathBuf),
    /// In `OUT_DIR`: written by the build script itself.
    Generated,
    Unknown,
}

/// The path an `include!`-family call reads: a string literal, or `concat!` of literals and
/// `env!("CARGO_MANIFEST_DIR")`. Only `concat!(env!("OUT_DIR"), "…")` with no `..` counts as
/// written by the script; any other mention of `OUT_DIR` can't be followed.
fn include_path(args: TokenStream, pkg_dir: &Path) -> IncludePath {
    let out_dir = mentions_out_dir(args.clone());
    let Ok(expr) = syn::parse2::<Expr>(args) else {
        return IncludePath::Unknown;
    };
    if out_dir {
        return match in_out_dir(&expr) {
            true => IncludePath::Generated,
            false => IncludePath::Unknown,
        };
    }
    let mut out = String::new();
    if !concat_into(&expr, pkg_dir, &mut out) {
        return IncludePath::Unknown;
    }
    IncludePath::File(PathBuf::from(out))
}

/// Whether the tokens hold the string `"OUT_DIR"`, as in `env!("OUT_DIR")`.
fn mentions_out_dir(tokens: TokenStream) -> bool {
    tokens.into_iter().any(|t| match t {
        TokenTree::Literal(l) => l.to_string() == "\"OUT_DIR\"",
        TokenTree::Group(g) => mentions_out_dir(g.stream()),
        _ => false,
    })
}

/// `concat!(env!("OUT_DIR"), "…", …)`: string literals after it that don't climb out with
/// `..`.
fn in_out_dir(expr: &Expr) -> bool {
    let Expr::Macro(m) = expr else {
        return false;
    };
    if !named(&m.mac.path, "concat") {
        return false;
    }
    let Ok(parts) = m
        .mac
        .parse_body_with(Punctuated::<Expr, Token![,]>::parse_terminated)
    else {
        return false;
    };
    let mut parts = parts.iter();
    let first = parts.next().is_some_and(|p| match p {
        Expr::Macro(e) if named(&e.mac.path, "env") => e
            .mac
            .parse_body::<syn::LitStr>()
            .is_ok_and(|n| n.value() == "OUT_DIR"),
        _ => false,
    });
    let mut rest = String::new();
    let literals = parts.all(|p| match p {
        Expr::Lit(lit) => match &lit.lit {
            Lit::Str(s) => {
                rest.push_str(&s.value());
                true
            }
            _ => false,
        },
        _ => false,
    });
    first && literals && !rest.split(['/', '\\']).any(|c| c == "..")
}

fn concat_into(expr: &Expr, pkg_dir: &Path, out: &mut String) -> bool {
    match expr {
        Expr::Lit(lit) => match &lit.lit {
            Lit::Str(s) => {
                out.push_str(&s.value());
                true
            }
            _ => false,
        },
        Expr::Macro(m) if named(&m.mac.path, "concat") => {
            let Ok(parts) = m
                .mac
                .parse_body_with(Punctuated::<Expr, Token![,]>::parse_terminated)
            else {
                return false;
            };
            parts.iter().all(|p| concat_into(p, pkg_dir, out))
        }
        Expr::Macro(m) if named(&m.mac.path, "env") => match m.mac.parse_body::<syn::LitStr>() {
            Ok(name) if name.value() == "CARGO_MANIFEST_DIR" => {
                out.push_str(&pkg_dir.to_string_lossy());
                true
            }
            _ => false,
        },
        _ => false,
    }
}

#[cfg(test)]
#[path = "build_files_test.rs"]
mod tests;
