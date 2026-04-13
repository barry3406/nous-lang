use std::path::PathBuf;
use std::process::Command;

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap() // crates/
        .parent()
        .unwrap() // workspace root
        .to_path_buf()
}

fn run_nous(subcmd: &str, file: &str) -> (bool, String) {
    let example_path = workspace_root().join(file);
    let output = Command::new(env!("CARGO_BIN_EXE_nous"))
        .arg(subcmd)
        .arg(&example_path)
        .output()
        .expect("failed to execute nous");
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    let combined = format!("{stdout}{stderr}");
    (output.status.success(), combined)
}

// ── Parser tests ─────────────────────────────────────

#[test]
fn parse_minimal() {
    let (ok, out) = run_nous("check", "examples/minimal.ns");
    assert!(ok, "minimal.ns should parse: {out}");
    assert!(out.contains("declarations"));
}

#[test]
fn parse_banking() {
    let (ok, out) = run_nous("check", "examples/banking.ns");
    assert!(ok, "banking.ns should parse: {out}");
    assert!(out.contains("declarations"));
}

#[test]
fn parse_agent_action() {
    let (ok, out) = run_nous("check", "examples/agent_action.ns");
    assert!(ok, "agent_action.ns should parse: {out}");
}

// ── Type checker tests ───────────────────────────────

#[test]
fn type_check_catches_unreachable_state() {
    let (ok, out) = run_nous("check", "examples/state_verify.ns");
    assert!(!ok, "should fail: {out}");
    assert!(out.contains("unreachable"));
}

#[test]
fn type_check_catches_bad_return_type() {
    let (ok, out) = run_nous("check", "examples/type_mismatch.ns");
    assert!(!ok, "should fail: {out}");
    assert!(out.contains("type mismatch"));
}

#[test]
fn type_check_catches_bad_flow_result_type() {
    let (ok, out) = run_nous("check", "examples/flow_type_mismatch.ns");
    assert!(!ok, "should fail: {out}");
    assert!(out.contains("type mismatch"));
}

#[test]
fn type_check_catches_non_bool_constraints() {
    let (ok, out) = run_nous("check", "examples/constraint_type_mismatch.ns");
    assert!(!ok, "should fail: {out}");
    assert!(out.contains("expected `Bool`, got `Int`"));
}

#[test]
fn type_check_catches_non_bool_flow_contracts() {
    let (ok, out) = run_nous("check", "examples/flow_constraint_type_mismatch.ns");
    assert!(!ok, "should fail: {out}");
    assert!(out.contains("expected `Bool`"));
}

// ── Verifier tests ───────────────────────────────────

#[test]
fn verify_conservation_law() {
    let (ok, out) = run_nous("verify", "examples/verify_conservation.ns");
    assert!(ok, "conservation should verify: {out}");
    assert!(out.contains("3 constraints verified"));
    assert!(out.contains("0 unverified"));
}

#[test]
fn verify_contracts_with_counterexamples() {
    let (ok, out) = run_nous("verify", "examples/contracts.ns");
    assert!(ok, "contracts should verify: {out}");
    assert!(out.contains("2 constraints verified"));
}

#[test]
fn verify_banking() {
    let (ok, out) = run_nous("verify", "examples/banking.ns");
    assert!(ok, "banking should verify: {out}");
}

// ── Runtime tests ────────────────────────────────────

#[test]
fn run_factorial() {
    let (ok, out) = run_nous("run", "examples/runnable.ns");
    assert!(ok, "runnable should execute: {out}");
    assert!(
        out.trim() == "120",
        "factorial(5) should be 120, got: {out}"
    );
}

#[test]
fn run_contracts() {
    let (ok, out) = run_nous("run", "examples/contracts.ns");
    assert!(ok, "contracts should execute: {out}");
    assert!(
        out.trim() == "120",
        "100/5 + clamp(150,0,100) = 120, got: {out}"
    );
}

#[test]
fn run_contract_violation() {
    let (ok, out) = run_nous("run", "examples/contract_fail.ns");
    assert!(!ok, "should fail with require violation: {out}");
    assert!(out.contains("require violated"));
}

#[test]
fn run_banking() {
    let (ok, out) = run_nous("run", "examples/banking.ns");
    assert!(ok, "banking should execute: {out}");
    assert!(
        out.trim() == "1500",
        "transfer 300 from 1000+500 = 1500, got: {out}"
    );
}

#[test]
fn run_banking_simple() {
    let (ok, out) = run_nous("run", "examples/banking_run.ns");
    assert!(ok, "banking_run should execute: {out}");
    assert!(out.trim() == "1500", "got: {out}");
}

#[test]
fn run_flow_saga() {
    let (ok, out) = run_nous("run", "examples/flow_saga.ns");
    assert!(ok, "flow should execute: {out}");
    assert!(out.trim() == "100", "checkout(100) = 100, got: {out}");
}

#[test]
fn run_pipe() {
    let (ok, out) = run_nous("run", "examples/pipe_test.ns");
    assert!(ok, "pipe should execute: {out}");
    assert!(
        out.trim() == "110",
        "5|>double|>square|>add(10) = 110, got: {out}"
    );
}

#[test]
fn run_ensure() {
    let (ok, out) = run_nous("run", "examples/ensure_test.ns");
    assert!(ok, "ensure should execute: {out}");
    assert!(
        out.trim() == "47",
        "double(21)+safe_abs(5) = 47, got: {out}"
    );
}

#[test]
fn run_agent_action() {
    let (ok, out) = run_nous("run", "examples/agent_action.ns");
    assert!(ok, "agent should execute: {out}");
    assert!(out.trim() == "90", "budget 100 - 10 = 90, got: {out}");
}

// ── Structured diagnostics tests ─────────────────────

#[test]
fn emit_structured_json() {
    let (ok, out) = run_nous("emit", "examples/contracts.ns");
    assert!(ok, "emit should succeed: {out}");
    assert!(out.contains("W301_REQUIRE_NOT_ALWAYS_TRUE"));
    assert!(out.contains("counterexample"));
    assert!(out.contains("fix_strategies"));
    assert!(out.contains("add_guard"));
}

#[test]
fn emit_clean() {
    let (ok, out) = run_nous("emit", "examples/runnable.ns");
    assert!(ok, "emit should succeed: {out}");
    assert!(out.trim() == "[]", "no diagnostics expected: {out}");
}

// ── AST output tests ─────────────────────────────────

#[test]
fn ast_json_output() {
    let (ok, out) = run_nous("ast", "examples/minimal.ns");
    assert!(ok, "ast should succeed: {out}");
    assert!(out.contains("\"Entity\""));
    assert!(out.contains("\"State\""));
}
