//! Multi-file module loader for Nous.
//!
//! Resolves `use` declarations, finds source files, parses them,
//! and merges all declarations into a single Program.
//!
//! Resolution strategy:
//!   `ns erp.auth` in file `apps/erp/auth.ns`
//!   `use erp.auth` → look for `auth.ns` in the same directory,
//!     or `erp/auth.ns` relative to the entry file's directory.
//!
//! Files are loaded at most once (by absolute path).

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use nous_ast::decl::Decl;
use nous_ast::program::Program;
use nous_ast::span::Spanned;

use crate::error::ParseError;

/// Load a Nous program starting from an entry file,
/// resolving all `use` declarations recursively.
pub fn load_program(entry_path: &Path) -> Result<Program, ParseError> {
    let mut loader = Loader::new();
    loader.load(entry_path)?;
    Ok(loader.into_program())
}

struct Loader {
    /// All declarations collected from all files, in dependency order.
    declarations: Vec<Spanned<Decl>>,
    /// Files already loaded (absolute path → true).
    loaded: HashSet<PathBuf>,
    /// Namespace → file path mapping discovered during loading.
    ns_to_path: HashMap<String, PathBuf>,
}

impl Loader {
    fn new() -> Self {
        Self {
            declarations: Vec::new(),
            loaded: HashSet::new(),
            ns_to_path: HashMap::new(),
        }
    }

    fn load(&mut self, file_path: &Path) -> Result<(), ParseError> {
        let abs_path = std::fs::canonicalize(file_path).map_err(|e| {
            ParseError::Grammar(format!("cannot resolve {}: {e}", file_path.display()))
        })?;

        // Skip if already loaded
        if self.loaded.contains(&abs_path) {
            return Ok(());
        }
        self.loaded.insert(abs_path.clone());

        // Read and parse
        let source = std::fs::read_to_string(&abs_path).map_err(|e| {
            ParseError::Grammar(format!("cannot read {}: {e}", abs_path.display()))
        })?;
        let program = crate::parse(&source)?;

        let file_dir = abs_path.parent().unwrap_or(Path::new("."));

        // First pass: register namespace and collect use declarations
        let mut uses: Vec<Vec<String>> = Vec::new();
        let mut ns_path: Option<Vec<String>> = None;

        for decl in &program.declarations {
            match &decl.node {
                Decl::Namespace(ns) => {
                    ns_path = Some(ns.path.clone());
                    let ns_key = ns.path.join(".");
                    self.ns_to_path.insert(ns_key, abs_path.clone());
                }
                Decl::Use(use_decl) => {
                    uses.push(use_decl.path.clone());
                }
                _ => {}
            }
        }

        // Second pass: resolve and load dependencies BEFORE our declarations
        for use_path in &uses {
            if let Some(dep_file) = self.resolve_use(use_path, file_dir) {
                self.load(&dep_file)?;
            }
            // If we can't resolve, that's ok — it might be a builtin or forward ref
        }

        // Third pass: add our declarations (after dependencies)
        for decl in program.declarations {
            match &decl.node {
                // Skip Use and Namespace — they're metadata, not code
                Decl::Use(_) | Decl::Namespace(_) => {}
                _ => {
                    self.declarations.push(decl);
                }
            }
        }

        Ok(())
    }

    /// Resolve a `use` path to a file on disk.
    ///
    /// Tries multiple strategies:
    ///   `use erp.auth` →
    ///     1. `./auth.ns` (sibling file)
    ///     2. `./erp/auth.ns` (subdirectory)
    ///     3. `../auth.ns` (parent directory)
    ///     4. Check ns_to_path registry
    fn resolve_use(&self, use_path: &[String], base_dir: &Path) -> Option<PathBuf> {
        if use_path.is_empty() {
            return None;
        }

        // Last segment is the module name, preceding segments are the path
        let module_name = use_path.last().unwrap();

        // Strategy 1: sibling file
        let sibling = base_dir.join(format!("{module_name}.ns"));
        if sibling.exists() {
            return Some(sibling);
        }

        // Strategy 2: subdirectory matching namespace path
        if use_path.len() >= 2 {
            let mut subdir = base_dir.to_path_buf();
            for segment in &use_path[..use_path.len() - 1] {
                subdir = subdir.join(segment);
            }
            let sub_file = subdir.join(format!("{module_name}.ns"));
            if sub_file.exists() {
                return Some(sub_file);
            }
        }

        // Strategy 3: full path as directory structure
        let mut full_path = base_dir.to_path_buf();
        for segment in &use_path[..use_path.len() - 1] {
            full_path = full_path.join(segment);
        }
        let full_file = full_path.join(format!("{module_name}.ns"));
        if full_file.exists() {
            return Some(full_file);
        }

        // Strategy 4: check if we already know where this namespace lives
        let ns_key = use_path.join(".");
        if let Some(known_path) = self.ns_to_path.get(&ns_key) {
            return Some(known_path.clone());
        }

        // Also try without the wildcard segment
        if use_path.len() >= 2 {
            let parent_ns = use_path[..use_path.len() - 1].join(".");
            if let Some(known_path) = self.ns_to_path.get(&parent_ns) {
                return Some(known_path.clone());
            }
        }

        None
    }

    fn into_program(self) -> Program {
        Program {
            declarations: self.declarations,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn test_single_file_load() {
        let dir = tempfile::tempdir().unwrap();
        let main_path = dir.path().join("main.ns");
        std::fs::write(&main_path, "ns test\n\nfn add(a: Int, b: Int) -> Int\n  a + b\n").unwrap();

        let program = load_program(&main_path).unwrap();
        assert!(!program.declarations.is_empty());
    }

    #[test]
    fn test_multi_file_load() {
        let dir = tempfile::tempdir().unwrap();

        // Write math.ns
        let math_path = dir.path().join("math.ns");
        std::fs::write(&math_path, "ns myapp.math\n\nfn add(a: Int, b: Int) -> Int\n  a + b\n").unwrap();

        // Write main.ns that uses math
        let main_path = dir.path().join("main.ns");
        std::fs::write(&main_path, "ns myapp\n\nuse myapp.math\n\nmain with [Prod]\n  add(1, 2)\n").unwrap();

        let program = load_program(&main_path).unwrap();
        // Should have declarations from both files
        let fn_count = program.declarations.iter()
            .filter(|d| matches!(&d.node, Decl::Fn(_)))
            .count();
        assert!(fn_count >= 1, "should have at least the add function from math.ns");
    }
}
