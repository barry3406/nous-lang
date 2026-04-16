//! Built-in functions for the Nous runtime.
//!
//! These provide I/O and system capabilities that can't be expressed
//! in pure Nous. They are the "effect handlers" — Rust implementations
//! bound to Nous function signatures.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::io::Write;
use std::sync::Mutex;

use crate::value::Value;
use crate::error::RuntimeError;
use crate::bytecode::Module;

/// A native function takes a list of arguments and returns a Value.
pub type NativeFn = fn(&[Value]) -> Result<Value, RuntimeError>;

// ── Module stash (for builtins that re-enter the VM) ───
// The current executing module, set at the start of Vm::execute.
// Used by http_serve_nous to dispatch requests to Nous handlers.
static CURRENT_MODULE: Mutex<Option<Module>> = Mutex::new(None);

pub fn set_current_module(m: Module) {
    if let Ok(mut guard) = CURRENT_MODULE.lock() {
        *guard = Some(m);
    }
}

pub fn with_current_module<T>(f: impl FnOnce(&Module) -> T) -> Option<T> {
    let guard = CURRENT_MODULE.lock().ok()?;
    guard.as_ref().map(f)
}

/// Registry of built-in functions.
pub struct Builtins {
    fns: BTreeMap<String, NativeFn>,
}

impl Builtins {
    /// Create a new registry with all standard builtins.
    pub fn new() -> Self {
        let mut b = Self { fns: BTreeMap::new() };

        // ── I/O ──────────────────────────────────────
        b.register("print", builtin_print);
        b.register("println", builtin_println);
        b.register("to_text", builtin_to_text);

        // ── Text operations ──────────────────────────
        b.register("text_len", builtin_text_len);
        b.register("text_concat", builtin_text_concat);

        // ── List operations ──────────────────────────
        b.register("list_len", builtin_list_len);
        b.register("list_get", builtin_list_get);
        b.register("list_push", builtin_list_push);

        // ── Type conversion ──────────────────────────
        b.register("int_to_text", builtin_int_to_text);
        b.register("text_to_int", builtin_text_to_int);

        // ── Hash (for content-addressing) ────────────
        b.register("sha256", builtin_sha256);

        // ── Time ─────────────────────────────────────
        b.register("now_unix", builtin_now_unix);

        // Register I/O builtins
        crate::io::register_io_builtins(&mut b.fns);

        // Nous self-verification: run the Nous compiler pipeline on source code
        b.register("nous_verify", builtin_nous_verify);

        b
    }

    fn register(&mut self, name: &str, f: NativeFn) {
        self.fns.insert(name.to_string(), f);
    }

    /// Look up a built-in function by name.
    pub fn lookup(&self, name: &str) -> Option<&NativeFn> {
        self.fns.get(name)
    }

    /// Check if a name is a builtin.
    pub fn contains(&self, name: &str) -> bool {
        self.fns.contains_key(name)
    }

    /// Get all builtin names.
    pub fn names(&self) -> Vec<&String> {
        self.fns.keys().collect()
    }
}

impl Default for Builtins {
    fn default() -> Self {
        Self::new()
    }
}

// ── Builtin implementations ──────────────────────────

fn builtin_print(args: &[Value]) -> Result<Value, RuntimeError> {
    for arg in args {
        print!("{arg}");
    }
    std::io::stdout().flush().ok();
    Ok(Value::Void)
}

fn builtin_println(args: &[Value]) -> Result<Value, RuntimeError> {
    for arg in args {
        print!("{arg}");
    }
    println!();
    Ok(Value::Void)
}

fn builtin_to_text(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.is_empty() {
        return Ok(Value::Text(String::new()));
    }
    Ok(Value::Text(format!("{}", args[0])))
}

fn builtin_text_len(args: &[Value]) -> Result<Value, RuntimeError> {
    match args.first() {
        Some(Value::Text(s)) => Ok(Value::Int(s.len() as i64)),
        _ => Err(RuntimeError::TypeMismatch {
            expected: "Text".into(),
            got: args.first().map_or("nothing", |v| v.type_name()).into(),
        }),
    }
}

fn builtin_text_concat(args: &[Value]) -> Result<Value, RuntimeError> {
    let mut result = String::new();
    for arg in args {
        match arg {
            Value::Text(s) => result.push_str(s),
            other => result.push_str(&format!("{other}")),
        }
    }
    Ok(Value::Text(result))
}

fn builtin_list_len(args: &[Value]) -> Result<Value, RuntimeError> {
    match args.first() {
        Some(Value::List(items)) => Ok(Value::Int(items.len() as i64)),
        _ => Err(RuntimeError::TypeMismatch {
            expected: "List".into(),
            got: args.first().map_or("nothing", |v| v.type_name()).into(),
        }),
    }
}

fn builtin_list_get(args: &[Value]) -> Result<Value, RuntimeError> {
    match (args.first(), args.get(1)) {
        (Some(Value::List(items)), Some(Value::Int(idx))) => {
            let i = *idx as usize;
            items.get(i).cloned().ok_or(RuntimeError::TypeMismatch {
                expected: format!("index < {}", items.len()),
                got: format!("index {i}"),
            })
        }
        _ => Err(RuntimeError::TypeMismatch {
            expected: "List, Int".into(),
            got: "wrong types".into(),
        }),
    }
}

fn builtin_list_push(args: &[Value]) -> Result<Value, RuntimeError> {
    match args.first() {
        Some(Value::List(items)) => {
            let mut new_items = items.clone();
            if let Some(val) = args.get(1) {
                new_items.push(val.clone());
            }
            Ok(Value::List(new_items))
        }
        _ => Err(RuntimeError::TypeMismatch {
            expected: "List".into(),
            got: args.first().map_or("nothing", |v| v.type_name()).into(),
        }),
    }
}

fn builtin_int_to_text(args: &[Value]) -> Result<Value, RuntimeError> {
    match args.first() {
        Some(Value::Int(n)) => Ok(Value::Text(n.to_string())),
        _ => Err(RuntimeError::TypeMismatch {
            expected: "Int".into(),
            got: args.first().map_or("nothing", |v| v.type_name()).into(),
        }),
    }
}

fn builtin_text_to_int(args: &[Value]) -> Result<Value, RuntimeError> {
    match args.first() {
        Some(Value::Text(s)) => {
            let n: i64 = s.parse().map_err(|_| RuntimeError::TypeMismatch {
                expected: "numeric text".into(),
                got: format!("\"{s}\""),
            })?;
            Ok(Value::Int(n))
        }
        _ => Err(RuntimeError::TypeMismatch {
            expected: "Text".into(),
            got: args.first().map_or("nothing", |v| v.type_name()).into(),
        }),
    }
}

fn builtin_sha256(args: &[Value]) -> Result<Value, RuntimeError> {
    use sha2::{Sha256, Digest};
    match args.first() {
        Some(Value::Text(s)) => {
            let mut hasher = Sha256::new();
            hasher.update(s.as_bytes());
            let result = hasher.finalize();
            Ok(Value::Text(hex::encode(result)))
        }
        _ => Err(RuntimeError::TypeMismatch {
            expected: "Text".into(),
            got: args.first().map_or("nothing", |v| v.type_name()).into(),
        }),
    }
}

fn builtin_now_unix(args: &[Value]) -> Result<Value, RuntimeError> {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    Ok(Value::Int(secs as i64))
}

/// Run the full Nous verification pipeline on source code.
/// Args: (source: Text)
/// Returns: Record { passed: Bool, verified: Int, unverified: Int, errors: Text, warnings: Text }
fn builtin_nous_verify(args: &[Value]) -> Result<Value, RuntimeError> {
    use std::collections::BTreeMap;

    let source = match args.first() {
        Some(Value::Text(s)) => s.as_str(),
        _ => return Err(RuntimeError::TypeMismatch {
            expected: "Text (nous source code)".into(),
            got: args.first().map_or("nothing", |v| v.type_name()).into(),
        }),
    };

    // Phase 1: Parse
    let program = match nous_parser::parse(source) {
        Ok(p) => p,
        Err(e) => {
            let mut fields = BTreeMap::new();
            fields.insert("passed".into(), Value::Bool(false));
            fields.insert("phase".into(), Value::Text("parse".into()));
            fields.insert("verified".into(), Value::Int(0));
            fields.insert("unverified".into(), Value::Int(0));
            fields.insert("errors".into(), Value::Text(e.to_string()));
            fields.insert("warnings".into(), Value::Text(String::new()));
            return Ok(Value::Record { name: "VerifyResult".into(), fields });
        }
    };

    // Phase 2: Type check
    let mut checker = nous_types::TypeChecker::new();
    if let Err(errors) = checker.check(&program) {
        let err_str = errors.iter().map(|e| e.to_string()).collect::<Vec<_>>().join("; ");
        let mut fields = BTreeMap::new();
        fields.insert("passed".into(), Value::Bool(false));
        fields.insert("phase".into(), Value::Text("typecheck".into()));
        fields.insert("verified".into(), Value::Int(0));
        fields.insert("unverified".into(), Value::Int(0));
        fields.insert("errors".into(), Value::Text(err_str));
        fields.insert("warnings".into(), Value::Text(String::new()));
        return Ok(Value::Record { name: "VerifyResult".into(), fields });
    }

    // Phase 3: Z3 constraint verification
    let verifier = nous_verify::Verifier::new();
    match verifier.verify(&program) {
        Ok(result) => {
            let warn_str = result.warnings.join("; ");
            let mut fields = BTreeMap::new();
            fields.insert("passed".into(), Value::Bool(true));
            fields.insert("phase".into(), Value::Text("verified".into()));
            fields.insert("verified".into(), Value::Int(result.verified_count as i64));
            fields.insert("unverified".into(), Value::Int(result.unverified_count as i64));
            fields.insert("errors".into(), Value::Text(String::new()));
            fields.insert("warnings".into(), Value::Text(warn_str));
            Ok(Value::Record { name: "VerifyResult".into(), fields })
        }
        Err(errors) => {
            let err_str = errors.iter().map(|e| e.to_string()).collect::<Vec<_>>().join("; ");
            let mut fields = BTreeMap::new();
            fields.insert("passed".into(), Value::Bool(false));
            fields.insert("phase".into(), Value::Text("verify".into()));
            fields.insert("verified".into(), Value::Int(0));
            fields.insert("unverified".into(), Value::Int(0));
            fields.insert("errors".into(), Value::Text(err_str));
            fields.insert("warnings".into(), Value::Text(String::new()));
            Ok(Value::Record { name: "VerifyResult".into(), fields })
        }
    }
}
