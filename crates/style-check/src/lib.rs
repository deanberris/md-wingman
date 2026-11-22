//! House-style checks that clippy doesn't have.
//!
//! The one check so far is control-flow depth. md-wingman prefers early
//! returns to nested conditionals: bail out with `let ... else`, `?`,
//! `return` or `continue`, and keep the main path flat. Clippy's
//! `excessive_nesting` counts every block, including the closures of gpui's
//! element builders, so it can't tell a deep `if` pyramid from a deep UI
//! tree. This check counts only control flow: `if`, `match`, `for`, `while`
//! and `loop`. An `else if` continues its chain rather than nesting, and a
//! closure or nested function starts again from zero.
//!
//! `cargo test -p style-check` runs it over the workspace (CI does this as
//! part of `cargo test --workspace`). `cargo run -p style-check` prints every
//! function that goes over the limit.

use std::fmt;
use std::path::{Path, PathBuf};

use syn::visit::{self, Visit};

/// How many levels of control flow may nest inside one function. A `for`
/// with an `if` inside is 2; a third level is fine, a fourth is not.
pub const MAX_DEPTH: usize = 3;

/// One place where control flow nests deeper than [`MAX_DEPTH`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Violation {
    pub file: PathBuf,
    pub line: usize,
    pub column: usize,
    pub depth: usize,
}

impl fmt::Display for Violation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}:{}:{}: control flow nested {} deep (max {MAX_DEPTH}); \
             return early, use `let ... else` or move the inner part into a function",
            self.file.display(),
            self.line,
            self.column + 1,
            self.depth,
        )
    }
}

/// Checks one file's source. `file` is only used in the report.
pub fn check_source(file: &Path, source: &str) -> syn::Result<Vec<Violation>> {
    let ast = syn::parse_file(source)?;
    let mut depth = Depth {
        file,
        depth: 0,
        out: Vec::new(),
    };
    depth.visit_file(&ast);
    Ok(depth.out)
}

/// Checks every `.rs` file under `dir`, skipping `target` directories.
pub fn check_dir(dir: &Path) -> Result<Vec<Violation>, String> {
    let mut files = Vec::new();
    collect_rust_files(dir, &mut files).map_err(|e| format!("{}: {e}", dir.display()))?;
    files.sort();
    let mut out = Vec::new();
    for file in files {
        let source =
            std::fs::read_to_string(&file).map_err(|e| format!("{}: {e}", file.display()))?;
        let found = check_source(&file, &source).map_err(|e| format!("{}: {e}", file.display()))?;
        out.extend(found);
    }
    Ok(out)
}

fn collect_rust_files(dir: &Path, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        if path.is_dir() {
            if path.file_name().is_some_and(|n| n != "target") {
                collect_rust_files(&path, out)?;
            }
            continue;
        }
        if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
    Ok(())
}

struct Depth<'a> {
    file: &'a Path,
    depth: usize,
    out: Vec<Violation>,
}

impl Depth<'_> {
    /// Visits `inner` one level deeper, reporting `span` if that is too deep.
    fn nested(&mut self, span: proc_macro2::Span, inner: impl FnOnce(&mut Self)) {
        self.depth += 1;
        if self.depth == MAX_DEPTH + 1 {
            let start = span.start();
            self.out.push(Violation {
                file: self.file.to_path_buf(),
                line: start.line,
                column: start.column,
                depth: self.depth,
            });
        }
        inner(self);
        self.depth -= 1;
    }

    /// Visits a closure or nested function from depth zero.
    fn fresh(&mut self, inner: impl FnOnce(&mut Self)) {
        let saved = std::mem::replace(&mut self.depth, 0);
        inner(self);
        self.depth = saved;
    }

    /// An `if` and its `else if` chain, all at one level.
    fn visit_if_chain(&mut self, node: &syn::ExprIf) {
        self.visit_expr(&node.cond);
        self.visit_block(&node.then_branch);
        let Some((_, else_branch)) = &node.else_branch else {
            return;
        };
        match else_branch.as_ref() {
            syn::Expr::If(next) => self.visit_if_chain(next),
            other => self.visit_expr(other),
        }
    }
}

impl<'ast> Visit<'ast> for Depth<'_> {
    fn visit_expr_if(&mut self, node: &'ast syn::ExprIf) {
        self.nested(node.if_token.span, |d| d.visit_if_chain(node));
    }

    fn visit_expr_match(&mut self, node: &'ast syn::ExprMatch) {
        self.visit_expr(&node.expr);
        self.nested(node.match_token.span, |d| {
            for arm in &node.arms {
                d.visit_arm(arm);
            }
        });
    }

    fn visit_expr_for_loop(&mut self, node: &'ast syn::ExprForLoop) {
        self.visit_expr(&node.expr);
        self.nested(node.for_token.span, |d| d.visit_block(&node.body));
    }

    fn visit_expr_while(&mut self, node: &'ast syn::ExprWhile) {
        self.nested(node.while_token.span, |d| {
            d.visit_expr(&node.cond);
            d.visit_block(&node.body);
        });
    }

    fn visit_expr_loop(&mut self, node: &'ast syn::ExprLoop) {
        self.nested(node.loop_token.span, |d| d.visit_block(&node.body));
    }

    fn visit_expr_closure(&mut self, node: &'ast syn::ExprClosure) {
        self.fresh(|d| visit::visit_expr_closure(d, node));
    }

    fn visit_item_fn(&mut self, node: &'ast syn::ItemFn) {
        self.fresh(|d| visit::visit_item_fn(d, node));
    }

    fn visit_impl_item_fn(&mut self, node: &'ast syn::ImplItemFn) {
        self.fresh(|d| visit::visit_impl_item_fn(d, node));
    }

    fn visit_trait_item_fn(&mut self, node: &'ast syn::TraitItemFn) {
        self.fresh(|d| visit::visit_trait_item_fn(d, node));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn depths(source: &str) -> Vec<(usize, usize)> {
        check_source(Path::new("t.rs"), source)
            .unwrap()
            .into_iter()
            .map(|v| (v.line, v.depth))
            .collect()
    }

    #[test]
    fn three_levels_are_fine() {
        let src = "fn f(xs: &[u8]) {
            for x in xs {
                if *x > 1 {
                    match x { 2 => {}, _ => {} }
                }
            }
        }";
        assert_eq!(depths(src), vec![]);
    }

    #[test]
    fn a_fourth_level_is_reported_once() {
        let src = "fn f(a: bool, b: bool, c: bool, d: bool) {
            if a {
                if b {
                    if c {
                        if d { if a {} }
                    }
                }
            }
        }";
        assert_eq!(depths(src), vec![(5, 4)]);
    }

    #[test]
    fn else_if_chains_stay_at_one_level() {
        let src = "fn f(a: u8) {
            if a == 1 {} else if a == 2 {} else if a == 3 {} else if a == 4 {} else {}
        }";
        assert_eq!(depths(src), vec![]);
    }

    #[test]
    fn closures_start_again() {
        let src = "fn f(a: bool) {
            if a { if a { if a { let g = || { if a { if a {} } }; } } }
        }";
        assert_eq!(depths(src), vec![]);
    }

    #[test]
    fn match_scrutinee_and_if_condition_are_not_nested() {
        let src = "fn f(a: Option<u8>) {
            if a.is_some() { if a.is_some() {
                match if a.is_some() { a } else { None } { Some(_) => {}, None => {} }
            } }
        }";
        assert_eq!(depths(src), vec![]);
    }
}
