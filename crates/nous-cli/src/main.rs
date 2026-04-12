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
}

fn main() {
    let cli = Cli::parse();

    let result = match cli.command {
        Commands::Check { file } => cmd_check(&file),
        Commands::Verify { file } => cmd_verify(&file),
        Commands::Run { file } => cmd_run(&file),
        Commands::Emit { file } => cmd_emit(&file),
        Commands::Ast { file } => cmd_ast(&file),
    };

    if let Err(e) = result {
        eprintln!("error: {e}");
        process::exit(1);
    }
}

fn read_source(path: &PathBuf) -> Result<String, String> {
    std::fs::read_to_string(path).map_err(|e| format!("cannot read {}: {e}", path.display()))
}

fn cmd_check(path: &PathBuf) -> Result<(), String> {
    let source = read_source(path)?;

    // Phase 0: Parse
    let program = nous_parser::parse(&source).map_err(|e| format!("parse error: {e}"))?;

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
    let source = read_source(path)?;
    let program = nous_parser::parse(&source).map_err(|e| format!("parse error: {e}"))?;

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
    let source = read_source(path)?;
    let program = nous_parser::parse(&source).map_err(|e| format!("parse error: {e}"))?;

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
    let source = read_source(path)?;

    // Try to parse and collect all diagnostics
    let program = match nous_parser::parse(&source) {
        Ok(p) => p,
        Err(e) => {
            let diag = serde_json::json!({
                "level": "error",
                "phase": "parse",
                "message": e.to_string(),
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
    if let Err(errors) = verifier.verify(&program) {
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
        return Ok(());
    }

    println!("[]"); // no diagnostics
    Ok(())
}

fn cmd_ast(path: &PathBuf) -> Result<(), String> {
    let source = read_source(path)?;
    let program = nous_parser::parse(&source).map_err(|e| format!("parse error: {e}"))?;
    let json = serde_json::to_string_pretty(&program).map_err(|e| format!("json error: {e}"))?;
    println!("{json}");
    Ok(())
}
