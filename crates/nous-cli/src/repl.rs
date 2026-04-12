//! Nous REPL — interactive constraint verification and execution.
//!
//! Designed for AI consumption: every response is structured JSON.
//! A human can read it, but the format optimizes for machine parsing.
//!
//! Commands:
//!   :def <nous declaration>   — define entity/fn/type/state
//!   :verify                   — verify all current constraints with Z3
//!   :run <expression>         — evaluate an expression
//!   :env                      — show all definitions
//!   :reset                    — clear all definitions
//!   :quit                     — exit
//!   <expression>              — shorthand for :run <expression>

use std::io::{self, BufRead, Write};

use nous_ast::decl::Decl;
use nous_ast::expr::Expr;
use nous_ast::program::Program;
use nous_ast::span::Spanned;

/// Run the REPL. Reads from stdin, writes to stdout.
/// If `json_mode` is true, all output is JSON (for AI consumption).
/// If false, output is human-friendly text.
pub fn run_repl(json_mode: bool) {
    let stdin = io::stdin();
    let mut stdout = io::stdout();
    let mut session = ReplSession::new();

    if !json_mode {
        println!("Nous v0.1 REPL — constraint synthesis & verification");
        println!("Commands: :def, :verify, :run, :env, :reset, :quit");
        println!();
    }

    for line in stdin.lock().lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => break,
        };
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        let response = session.handle_input(trimmed);

        if json_mode {
            println!("{}", serde_json::to_string(&response).unwrap());
        } else {
            print_human_response(&response);
        }

        if let ReplResponseKind::Quit = response.kind {
            break;
        }

        if !json_mode {
            print!("nous> ");
            stdout.flush().ok();
        }
    }
}

// ── Session state ────────────────────────────────────

struct ReplSession {
    /// Accumulated source code from :def commands.
    source: String,
    /// Number of definitions.
    def_count: usize,
}

impl ReplSession {
    fn new() -> Self {
        Self {
            source: "ns repl\n\n".to_string(),
            def_count: 0,
        }
    }

    fn handle_input(&mut self, input: &str) -> ReplResponse {
        if input == ":quit" || input == ":q" {
            return ReplResponse {
                kind: ReplResponseKind::Quit,
                success: true,
                message: "goodbye".to_string(),
                data: None,
            };
        }

        if input == ":reset" {
            self.source = "ns repl\n\n".to_string();
            self.def_count = 0;
            return ReplResponse {
                kind: ReplResponseKind::Reset,
                success: true,
                message: "session cleared".to_string(),
                data: None,
            };
        }

        if input == ":env" {
            return ReplResponse {
                kind: ReplResponseKind::Env,
                success: true,
                message: format!("{} definitions", self.def_count),
                data: Some(serde_json::json!({
                    "definitions": self.def_count,
                    "source": self.source,
                })),
            };
        }

        if input == ":verify" {
            return self.verify_all();
        }

        if let Some(def) = input.strip_prefix(":def ") {
            // Support escaped newlines: \n in the input becomes actual newlines
            let expanded = def.replace("\\n", "\n");
            return self.add_definition(&expanded);
        }

        if let Some(expr) = input.strip_prefix(":run ") {
            return self.run_expression(expr);
        }

        // Default: try to evaluate as expression
        self.run_expression(input)
    }

    fn add_definition(&mut self, def: &str) -> ReplResponse {
        // Append definition to source
        self.source.push_str(def);
        self.source.push('\n');
        self.def_count += 1;

        // Try to parse the full source to check validity
        match nous_parser::parse(&self.source) {
            Ok(program) => {
                // Type check
                let mut checker = nous_types::TypeChecker::new();
                match checker.check(&program) {
                    Ok(()) => {
                        // Check if the latest definition was synthesized
                        let last_decl = program.declarations.last();
                        let synthesized = last_decl.map_or(false, |d| {
                            if let Decl::Fn(f) = &d.node {
                                let body_empty = matches!(&f.body.node, Expr::Void)
                                    || matches!(&f.body.node, Expr::Block(s) if s.is_empty());
                                body_empty && !f.contract.ensures.is_empty()
                            } else {
                                false
                            }
                        });

                        ReplResponse {
                            kind: ReplResponseKind::Defined,
                            success: true,
                            message: if synthesized {
                                "defined (body will be synthesized from ensures)".to_string()
                            } else {
                                "defined".to_string()
                            },
                            data: Some(serde_json::json!({
                                "definitions": self.def_count,
                                "synthesized": synthesized,
                            })),
                        }
                    }
                    Err(errors) => {
                        // Undo the definition
                        let lines: Vec<&str> = self.source.lines().collect();
                        self.source = lines[..lines.len()-1].join("\n") + "\n";
                        self.def_count -= 1;

                        ReplResponse {
                            kind: ReplResponseKind::Error,
                            success: false,
                            message: errors.iter().map(|e| e.to_string()).collect::<Vec<_>>().join("; "),
                            data: None,
                        }
                    }
                }
            }
            Err(e) => {
                // Undo
                let lines: Vec<&str> = self.source.lines().collect();
                self.source = lines[..lines.len()-1].join("\n") + "\n";
                self.def_count -= 1;

                ReplResponse {
                    kind: ReplResponseKind::Error,
                    success: false,
                    message: format!("parse error: {e}"),
                    data: None,
                }
            }
        }
    }

    fn verify_all(&self) -> ReplResponse {
        let program = match nous_parser::parse(&self.source) {
            Ok(p) => p,
            Err(e) => {
                return ReplResponse {
                    kind: ReplResponseKind::Error,
                    success: false,
                    message: format!("parse error: {e}"),
                    data: None,
                };
            }
        };

        let verifier = nous_verify::Verifier::new();
        match verifier.verify(&program) {
            Ok(result) => {
                ReplResponse {
                    kind: ReplResponseKind::Verified,
                    success: true,
                    message: format!(
                        "{} verified, {} unverified",
                        result.verified_count, result.unverified_count
                    ),
                    data: Some(serde_json::json!({
                        "verified": result.verified_count,
                        "unverified": result.unverified_count,
                        "warnings": result.warnings,
                        "diagnostics": result.diagnostics,
                    })),
                }
            }
            Err(errors) => {
                ReplResponse {
                    kind: ReplResponseKind::Error,
                    success: false,
                    message: errors.iter().map(|e| e.to_string()).collect::<Vec<_>>().join("; "),
                    data: None,
                }
            }
        }
    }

    fn run_expression(&self, expr_str: &str) -> ReplResponse {
        // Wrap expression in a main block
        let full_source = format!(
            "{}main with [Repl]\n  {}\n",
            self.source, expr_str
        );

        let program = match nous_parser::parse(&full_source) {
            Ok(p) => p,
            Err(e) => {
                return ReplResponse {
                    kind: ReplResponseKind::Error,
                    success: false,
                    message: format!("parse error: {e}"),
                    data: None,
                };
            }
        };

        // Type check
        let mut checker = nous_types::TypeChecker::new();
        if let Err(errors) = checker.check(&program) {
            return ReplResponse {
                kind: ReplResponseKind::Error,
                success: false,
                message: errors.iter().map(|e| e.to_string()).collect::<Vec<_>>().join("; "),
                data: None,
            };
        }

        // Compile
        let module = match nous_runtime::compiler::CompilerCtx::new().compile(&program) {
            Ok(m) => m,
            Err(e) => {
                return ReplResponse {
                    kind: ReplResponseKind::Error,
                    success: false,
                    message: format!("compile error: {e}"),
                    data: None,
                };
            }
        };

        // Execute
        let mut vm = nous_runtime::Vm::new();
        match vm.execute(&module) {
            Ok(value) => {
                ReplResponse {
                    kind: ReplResponseKind::Result,
                    success: true,
                    message: format!("{value}"),
                    data: Some(serde_json::json!({
                        "value": format!("{value}"),
                        "type": value.type_name(),
                    })),
                }
            }
            Err(e) => {
                ReplResponse {
                    kind: ReplResponseKind::Error,
                    success: false,
                    message: format!("runtime error: {e}"),
                    data: None,
                }
            }
        }
    }
}

// ── Response types ───────────────────────────────────

#[derive(Debug, serde::Serialize)]
struct ReplResponse {
    kind: ReplResponseKind,
    success: bool,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    data: Option<serde_json::Value>,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "snake_case")]
enum ReplResponseKind {
    Defined,
    Verified,
    Result,
    Error,
    Env,
    Reset,
    Quit,
}

fn print_human_response(r: &ReplResponse) {
    match r.kind {
        ReplResponseKind::Defined => println!("  ✓ {}", r.message),
        ReplResponseKind::Verified => {
            println!("  ✓ {}", r.message);
            if let Some(data) = &r.data {
                if let Some(warnings) = data.get("warnings").and_then(|w| w.as_array()) {
                    for w in warnings {
                        if let Some(s) = w.as_str() {
                            println!("  ⚠ {s}");
                        }
                    }
                }
            }
        }
        ReplResponseKind::Result => println!("  → {}", r.message),
        ReplResponseKind::Error => println!("  ✗ {}", r.message),
        ReplResponseKind::Env => {
            println!("  {}", r.message);
            if let Some(data) = &r.data {
                if let Some(src) = data.get("source").and_then(|s| s.as_str()) {
                    for line in src.lines() {
                        if !line.trim().is_empty() {
                            println!("    {line}");
                        }
                    }
                }
            }
        }
        ReplResponseKind::Reset => println!("  ✓ {}", r.message),
        ReplResponseKind::Quit => {}
    }
}
