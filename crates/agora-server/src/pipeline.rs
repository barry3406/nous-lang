use serde::{Deserialize, Serialize};
use std::time::Instant;

use nous_parser::parse;
use nous_types::TypeChecker;
use nous_verify::Verifier;

// ---------------------------------------------------------------------------
// Result types
// ---------------------------------------------------------------------------

/// The outcome of a single verification phase.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PhaseResult {
    /// Whether this phase completed without errors.
    pub passed: bool,
    /// Wall-clock time the phase took, in milliseconds.
    pub duration_ms: u64,
    /// Human-readable (and machine-parseable) summary of the phase outcome.
    pub message: String,
}

/// The aggregated result of running a proposal through all three pipeline phases.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PipelineResult {
    /// `true` only if every non-optional phase passed.
    pub passed: bool,
    /// Phase 1 — parse + type-check.
    pub phase1: PhaseResult,
    /// Phase 2 — Z3 constraint verification.
    pub phase2: PhaseResult,
    /// Phase 3 — cross-reference check (stubbed, never blocks merge).
    pub phase3: PhaseResult,
    /// Structured diagnostics emitted by any phase, suitable for AI consumption.
    pub diagnostics: Vec<serde_json::Value>,
}

// ---------------------------------------------------------------------------
// VerifyPipeline
// ---------------------------------------------------------------------------

/// Runs incoming proposals through a three-phase verification pipeline.
///
/// Phases:
/// 1. **Parse + type-check** — must complete in < 1 s (budget enforced by the
///    platform, not here; timings are reported for monitoring).
/// 2. **Z3 constraint verification** — the Nous `Verifier` runs SMT checks on
///    every `require`/`ensure` clause.  Budget: < 10 s.
/// 3. **Cross-reference** — checks that downstream consumers are not broken.
///    Currently stubbed; always passes and never blocks a merge.
pub struct VerifyPipeline;

impl VerifyPipeline {
    /// Create a new pipeline instance.
    pub fn new() -> Self {
        Self
    }

    /// Verify `source` through all three phases and return the aggregated result.
    pub fn verify_proposal(&self, source: &str) -> PipelineResult {
        let mut diagnostics: Vec<serde_json::Value> = Vec::new();

        // ── Phase 1: Parse + type-check ────────────────────────────────────
        let phase1 = self.run_phase1(source, &mut diagnostics);

        // If parsing failed we cannot proceed to later phases.
        if !phase1.passed {
            return PipelineResult {
                passed: false,
                phase1,
                phase2: PhaseResult {
                    passed: false,
                    duration_ms: 0,
                    message: "skipped — phase 1 did not pass".to_string(),
                },
                phase3: PhaseResult {
                    passed: true,
                    duration_ms: 0,
                    message: "skipped — phase 1 did not pass".to_string(),
                },
                diagnostics,
            };
        }

        // Re-parse to get the AST we'll hand to subsequent phases.
        // (Phase 1 already validated it; this second parse is guaranteed to
        // succeed and is cheap relative to the verification work.)
        let program = parse(source).expect("parse succeeded in phase 1");

        // ── Phase 2: Z3 constraint verification ───────────────────────────
        let phase2 = self.run_phase2(&program, &mut diagnostics);

        // ── Phase 3: Cross-reference (stubbed) ────────────────────────────
        let phase3 = self.run_phase3(&mut diagnostics);

        let passed = phase1.passed && phase2.passed;
        // Phase 3 is advisory — it never blocks a merge (per AGORA.md §3).

        PipelineResult {
            passed,
            phase1,
            phase2,
            phase3,
            diagnostics,
        }
    }

    // -----------------------------------------------------------------------
    // Phase implementations
    // -----------------------------------------------------------------------

    /// Phase 1 — parse the source and run the type checker.
    fn run_phase1(
        &self,
        source: &str,
        diagnostics: &mut Vec<serde_json::Value>,
    ) -> PhaseResult {
        let start = Instant::now();

        // --- Parse ---
        let program = match parse(source) {
            Ok(p) => p,
            Err(e) => {
                let elapsed = start.elapsed().as_millis() as u64;
                let msg = format!("parse error: {e}");
                diagnostics.push(serde_json::json!({
                    "phase": 1,
                    "kind": "parse_error",
                    "message": msg,
                }));
                return PhaseResult {
                    passed: false,
                    duration_ms: elapsed,
                    message: msg,
                };
            }
        };

        // --- Type-check ---
        let mut checker = TypeChecker::new();
        match checker.check(&program) {
            Ok(()) => {
                let elapsed = start.elapsed().as_millis() as u64;
                PhaseResult {
                    passed: true,
                    duration_ms: elapsed,
                    message: "parse and type-check passed".to_string(),
                }
            }
            Err(errors) => {
                let elapsed = start.elapsed().as_millis() as u64;
                let messages: Vec<String> = errors.iter().map(|e| e.to_string()).collect();
                for msg in &messages {
                    diagnostics.push(serde_json::json!({
                        "phase": 1,
                        "kind": "type_error",
                        "message": msg,
                    }));
                }
                PhaseResult {
                    passed: false,
                    duration_ms: elapsed,
                    message: format!(
                        "{} type error(s): {}",
                        messages.len(),
                        messages.join("; ")
                    ),
                }
            }
        }
    }

    /// Phase 2 — run the Nous `Verifier` (SMT / Z3 constraint checks).
    fn run_phase2(
        &self,
        program: &nous_ast::Program,
        diagnostics: &mut Vec<serde_json::Value>,
    ) -> PhaseResult {
        let start = Instant::now();
        let verifier = Verifier::new();

        match verifier.verify(program) {
            Ok(result) => {
                let elapsed = start.elapsed().as_millis() as u64;
                // Forward structured diagnostics (warnings) as advisory entries.
                for diag in &result.diagnostics {
                    diagnostics.push(serde_json::json!({
                        "phase": 2,
                        "kind": "advisory",
                        "code": diag.code,
                        "message": diag.constraint,
                    }));
                }
                for warn in &result.warnings {
                    diagnostics.push(serde_json::json!({
                        "phase": 2,
                        "kind": "warning",
                        "message": warn,
                    }));
                }
                PhaseResult {
                    passed: true,
                    duration_ms: elapsed,
                    message: format!(
                        "verified {} constraint(s), {} unverified (SMT stub)",
                        result.verified_count, result.unverified_count
                    ),
                }
            }
            Err(errors) => {
                let elapsed = start.elapsed().as_millis() as u64;
                let messages: Vec<String> = errors.iter().map(|e| e.to_string()).collect();
                for msg in &messages {
                    diagnostics.push(serde_json::json!({
                        "phase": 2,
                        "kind": "constraint_error",
                        "message": msg,
                    }));
                }
                PhaseResult {
                    passed: false,
                    duration_ms: elapsed,
                    message: format!(
                        "{} constraint violation(s): {}",
                        messages.len(),
                        messages.join("; ")
                    ),
                }
            }
        }
    }

    /// Phase 3 — cross-reference check.
    ///
    /// Checks that downstream consumers of any changed namespace are not broken
    /// by this proposal.  Full implementation requires the global constraint
    /// graph; this stub always passes and reports itself as advisory.
    fn run_phase3(&self, diagnostics: &mut Vec<serde_json::Value>) -> PhaseResult {
        let start = Instant::now();
        diagnostics.push(serde_json::json!({
            "phase": 3,
            "kind": "stub",
            "message": "cross-reference check not yet implemented; pass is advisory only",
        }));
        let elapsed = start.elapsed().as_millis() as u64;
        PhaseResult {
            passed: true,
            duration_ms: elapsed,
            message: "cross-reference check stubbed — advisory pass".to_string(),
        }
    }
}

impl Default for VerifyPipeline {
    fn default() -> Self {
        Self::new()
    }
}
