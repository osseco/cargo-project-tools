//! The files a build script pulls in besides itself: its modules (`mod`, `#[path]`) and
//! what it includes (`include!`, `include_str!`, `include_bytes!`), followed through those
//! files too. When that can't be worked out for sure, the answer is the whole package.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use proc_macro2::{Delimiter, TokenStream, TokenTree};
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
    finder.source(&script, dir)?;
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
        Ok(self.pkg_dir.join(rel))
    }

    /// Take in a file that exists: checked, recorded, and for source files, read.
    fn add(&mut self, path: &Path, include: Option<Include>, mod_dir: &Path) -> Result<()> {
        let path = self.check(path)?;
        self.files.insert(path.clone());
        match include {
            Some(Include::Data) => Ok(()),
            Some(Include::Code) => self.source(&path, path.parent().unwrap_or(mod_dir)),
            None => self.source(&path, mod_dir),
        }
    }

    /// Read a Rust source file whose `mod x;` declarations are looked for in `mod_dir`.
    fn source(&mut self, path: &Path, mod_dir: &Path) -> Result<()> {
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
        let mut includes = Vec::new();
        let mut mods = 0;
        scan(tokens, &mut includes, &mut mods);
        for (kind, args) in includes {
            self.include(kind, args, &file_dir, &rel)?;
        }
        // Items, or an expression for an `include!`d file.
        let walked = match syn::parse_file(&text) {
            Ok(file) => {
                let ctx = Ctx {
                    file_dir: &file_dir,
                    mod_dir: mod_dir.to_path_buf(),
                    inline: false,
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
                self.give_up(format!("{rel} has a `#[path]` that isn't a plain string"));
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
                };
                count += self.items(inner, &inner_ctx, rel)?;
                continue;
            }
            let base = if ctx.inline {
                &ctx.mod_dir
            } else {
                ctx.file_dir
            };
            for p in &paths {
                let file = base.join(p);
                if file.exists() {
                    // A file loaded through `#[path]` acts as a mod-rs file.
                    let dir = file.parent().unwrap_or(base).to_path_buf();
                    self.add(&file, None, &dir)?;
                }
            }
            if !always {
                let flat = ctx.mod_dir.join(format!("{name}.rs"));
                let nested = ctx.mod_dir.join(&name).join("mod.rs");
                if flat.exists() {
                    self.add(&flat, None, &ctx.mod_dir.join(&name))?;
                } else if nested.exists() {
                    self.add(&nested, None, &ctx.mod_dir.join(&name))?;
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
        if path.exists() {
            self.add(&path, Some(kind), file_dir)?;
        }
        Ok(())
    }
}

/// The module's paths from `#[path = "…"]` and `#[cfg_attr(…, path = "…")]`, and whether
/// one is unconditional (else the usual file is a candidate too); `None` when one isn't a
/// string literal.
fn path_attrs(attrs: &[syn::Attribute]) -> Option<(Vec<String>, bool)> {
    let mut out = Vec::new();
    let mut always = false;
    for attr in attrs {
        if attr.path().is_ident("path") {
            out.push(meta_str(&attr.meta)?);
            always = true;
        } else if attr.path().is_ident("cfg_attr") {
            let metas = attr
                .parse_args_with(Punctuated::<Meta, Token![,]>::parse_terminated)
                .ok()?;
            for meta in metas.iter().skip(1) {
                if meta.path().is_ident("path") {
                    out.push(meta_str(meta)?);
                }
            }
        }
    }
    Some((out, always))
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

/// Collect `include!`-family calls and count `mod` keywords, anywhere in the tokens.
fn scan(tokens: TokenStream, includes: &mut Vec<(Include, TokenStream)>, mods: &mut usize) {
    let tokens: Vec<TokenTree> = tokens.into_iter().collect();
    for (i, t) in tokens.iter().enumerate() {
        match t {
            TokenTree::Ident(id) => {
                let name = id.to_string();
                if name == "mod" {
                    *mods += 1;
                }
                let kind = match name.as_str() {
                    "include" => Some(Include::Code),
                    "include_str" | "include_bytes" => Some(Include::Data),
                    _ => None,
                };
                if let (Some(kind), Some(TokenTree::Punct(p)), Some(TokenTree::Group(g))) =
                    (kind, tokens.get(i + 1), tokens.get(i + 2))
                    && p.as_char() == '!'
                    && g.delimiter() != Delimiter::None
                {
                    includes.push((kind, g.stream()));
                }
            }
            TokenTree::Group(g) => scan(g.stream(), includes, mods),
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
/// `env!("CARGO_MANIFEST_DIR")`.
fn include_path(args: TokenStream, pkg_dir: &Path) -> IncludePath {
    if mentions_out_dir(args.clone()) {
        return IncludePath::Generated;
    }
    let Ok(expr) = syn::parse2::<Expr>(args) else {
        return IncludePath::Unknown;
    };
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

fn concat_into(expr: &Expr, pkg_dir: &Path, out: &mut String) -> bool {
    match expr {
        Expr::Lit(lit) => match &lit.lit {
            Lit::Str(s) => {
                out.push_str(&s.value());
                true
            }
            _ => false,
        },
        Expr::Macro(m) if m.mac.path.is_ident("concat") => {
            let Ok(parts) = m
                .mac
                .parse_body_with(Punctuated::<Expr, Token![,]>::parse_terminated)
            else {
                return false;
            };
            parts.iter().all(|p| concat_into(p, pkg_dir, out))
        }
        Expr::Macro(m) if m.mac.path.is_ident("env") => match m.mac.parse_body::<syn::LitStr>() {
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
