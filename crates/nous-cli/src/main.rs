mod lsp;
mod repl;

use std::path::PathBuf;
use std::process;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "nous",
    version,
    about = "Nous — a programming language where AI writes constraints, the compiler verifies them, and the runtime executes them."
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Parse and type-check a Nous source file
    Check {
        /// Path to .ns source file
        file: PathBuf,
    },
    /// Run full constraint verification (SMT solver)
    Verify {
        /// Path to .ns source file
        file: PathBuf,
    },
    /// Execute a Nous program
    Run {
        /// Path to .ns source file
        file: PathBuf,
    },
    /// Emit structured diagnostics as JSON
    Emit {
        /// Path to .ns source file
        file: PathBuf,
    },
    /// Print the AST as JSON
    Ast {
        /// Path to .ns source file
        file: PathBuf,
    },
    /// Compile to JavaScript
    Js {
        /// Path to .ns source file
        file: PathBuf,
        /// Output file (default: stdout)
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
    /// Start Language Server Protocol server (for VS Code)
    Lsp,
    /// Start interactive REPL
    Repl {
        /// Output JSON for AI consumption (default: human-friendly)
        #[arg(long)]
        json: bool,
    },
}

fn main() {
    let cli = Cli::parse();

    let result = match cli.command {
        Commands::Check { file } => cmd_check(&file),
        Commands::Verify { file } => cmd_verify(&file),
        Commands::Run { file } => cmd_run(&file),
        Commands::Emit { file } => cmd_emit(&file),
        Commands::Ast { file } => cmd_ast(&file),
        Commands::Js { file, output } => cmd_js(&file, output.as_deref()),
        Commands::Lsp => { lsp::run_lsp(); Ok(()) },
        Commands::Repl { json } => { repl::run_repl(json); Ok(()) },
    };

    if let Err(e) = result {
        eprintln!("error: {e}");
        process::exit(1);
    }
}

fn read_source(path: &PathBuf) -> Result<String, String> {
    std::fs::read_to_string(path).map_err(|e| format!("cannot read {}: {e}", path.display()))
}

fn load_file(path: &PathBuf) -> Result<nous_ast::Program, String> {
    nous_parser::load_program(path)
        .map_err(|e| format!("load error: {e}"))
}

fn cmd_check(path: &PathBuf) -> Result<(), String> {
    // Phase 0: Parse (with multi-file resolution)
    let program = load_file(path)?;

    // Phase 1: Type check
    let mut checker = nous_types::TypeChecker::new();
    match checker.check(&program) {
        Ok(()) => {
            let decl_count = program.declarations.len();
            println!("✓ {} — {decl_count} declarations, all checks passed", path.display());
            Ok(())
        }
        Err(errors) => {
            for err in &errors {
                eprintln!("  ✗ {err}");
            }
            Err(format!("{} type error(s)", errors.len()))
        }
    }
}

fn cmd_verify(path: &PathBuf) -> Result<(), String> {
    let program = load_file(path)?;

    // Type check first
    let mut checker = nous_types::TypeChecker::new();
    checker
        .check(&program)
        .map_err(|errors| {
            for err in &errors {
                eprintln!("  ✗ {err}");
            }
            format!("{} type error(s) — fix before verifying", errors.len())
        })?;

    // Constraint verification
    let verifier = nous_verify::Verifier::new();
    match verifier.verify(&program) {
        Ok(result) => {
            println!(
                "✓ {} — {} constraints verified, {} unverified",
                path.display(),
                result.verified_count,
                result.unverified_count
            );
            for w in &result.warnings {
                println!("  ⚠ {w}");
            }
            Ok(())
        }
        Err(errors) => {
            for err in &errors {
                eprintln!("  ✗ {err}");
            }
            Err(format!("{} verification error(s)", errors.len()))
        }
    }
}

fn cmd_run(path: &PathBuf) -> Result<(), String> {
    let program = load_file(path)?;

    // Type check
    let mut checker = nous_types::TypeChecker::new();
    checker
        .check(&program)
        .map_err(|errors| format!("{} type error(s)", errors.len()))?;

    // Compile to bytecode
    let module = nous_runtime::compiler::CompilerCtx::new()
        .compile(&program)
        .map_err(|e| format!("compile error: {e}"))?;

    // Execute
    let mut vm = nous_runtime::Vm::new();
    match vm.execute(&module) {
        Ok(value) => {
            println!("{value}");
            Ok(())
        }
        Err(e) => Err(format!("runtime error: {e}")),
    }
}

fn cmd_emit(path: &PathBuf) -> Result<(), String> {
    let program = match load_file(path) {
        Ok(p) => p,
        Err(e) => {
            let diag = serde_json::json!({
                "level": "error",
                "phase": "parse",
                "message": e,
            });
            println!("{}", serde_json::to_string_pretty(&diag).unwrap());
            return Ok(());
        }
    };

    let mut checker = nous_types::TypeChecker::new();
    if let Err(errors) = checker.check(&program) {
        let diags: Vec<_> = errors
            .iter()
            .map(|e| {
                serde_json::json!({
                    "level": "error",
                    "phase": "type_check",
                    "message": e.to_string(),
                })
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&diags).unwrap());
        return Ok(());
    }

    let verifier = nous_verify::Verifier::new();
    match verifier.verify(&program) {
        Ok(result) => {
            if result.diagnostics.is_empty() {
                println!("[]");
            } else {
                println!("{}", serde_json::to_string_pretty(&result.diagnostics).unwrap());
            }
        }
        Err(errors) => {
            let diags: Vec<_> = errors
                .iter()
                .map(|e| {
                    serde_json::json!({
                        "level": "error",
                        "phase": "verify",
                        "message": e.to_string(),
                    })
                })
                .collect();
            println!("{}", serde_json::to_string_pretty(&diags).unwrap());
        }
    }
    Ok(())
}

fn cmd_ast(path: &PathBuf) -> Result<(), String> {
    let program = load_file(path)?;
    let json = serde_json::to_string_pretty(&program).map_err(|e| format!("json error: {e}"))?;
    println!("{json}");
    Ok(())
}

fn cmd_js(path: &PathBuf, output: Option<&std::path::Path>) -> Result<(), String> {
    let program = load_file(path)?;

    // Type check first
    let mut checker = nous_types::TypeChecker::new();
    checker
        .check(&program)
        .map_err(|errors| {
            for err in &errors {
                eprintln!("  ✗ {err}");
            }
            format!("{} type error(s)", errors.len())
        })?;

    let js = nous_runtime::codegen_js::compile_to_js(&program);

    if let Some(out_path) = output {
        std::fs::write(out_path, &js).map_err(|e| format!("write error: {e}"))?;
        eprintln!("✓ {} → {}", path.display(), out_path.display());
    } else {
        println!("{js}");
    }
    Ok(())
}
