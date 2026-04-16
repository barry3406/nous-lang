//! Trust propagation.
//!
//! Trust flows downward through the call graph: if A calls B, A's effective
//! trust level is the MINIMUM of its own declared trust and B's effective trust.
//!
//! If you declare `trust proved` but call an `observed` function, your effective
//! trust is `observed` — because the claim now depends on something only observed.
//!
//! This is run as a fixed-point computation: keep propagating until nothing
//! changes. That gives us the strongest-possible-consistent trust assignment.
//!
//! Without propagation, trust is a lie. `trust proved` + calls to `observed`
//! functions + user sees "proved" = false confidence. That's worse than no
//! trust annotation at all.

use std::collections::{HashMap, HashSet};

use nous_ast::decl::{Decl, TrustLevel};
use nous_ast::expr::Expr;
use nous_ast::program::Program;
use nous_ast::span::Spanned;

/// Result of trust analysis.
#[derive(Debug, Clone)]
pub struct TrustAnalysis {
    /// Final effective trust per function (after propagation).
    pub effective: HashMap<String, TrustLevel>,
    /// Declared trust per function (what the programmer wrote).
    pub declared: HashMap<String, TrustLevel>,
    /// Functions whose effective trust is weaker than declared.
    /// Each: (fn_name, declared, effective, witness_callee)
    pub downgraded: Vec<(String, TrustLevel, TrustLevel, String)>,
}

impl TrustAnalysis {
    /// Return true if a function's effective trust differs from its declared trust.
    pub fn was_downgraded(&self, fn_name: &str) -> bool {
        self.downgraded.iter().any(|(n, _, _, _)| n == fn_name)
    }
}

/// The trust lattice: PROVED is strongest, ASSUMED is weakest.
/// When combining, take the MINIMUM (weakest link).
fn meet(a: &TrustLevel, b: &TrustLevel) -> TrustLevel {
    use TrustLevel::*;
    // Ord derives lexicographically on the enum variants in declaration order:
    // Proved < Checked < Observed < Assumed — so MAX gives us the weakest.
    // But conceptually, Proved is STRONGEST. So we want the later variant
    // when comparing (Assumed > Observed > Checked > Proved in "weakness").
    match (a, b) {
        (Assumed, _) | (_, Assumed) => Assumed,
        (Observed, _) | (_, Observed) => Observed,
        (Checked, _) | (_, Checked) => Checked,
        (Proved, Proved) => Proved,
    }
}

/// Run trust propagation on a program. Returns effective trust per function.
pub fn analyze(program: &Program) -> TrustAnalysis {
    // Step 1: collect declared trust and call graph
    let mut declared: HashMap<String, TrustLevel> = HashMap::new();
    let mut calls: HashMap<String, HashSet<String>> = HashMap::new();

    for decl in &program.declarations {
        match &decl.node {
            Decl::Fn(f) => {
                declared.insert(f.name.clone(), f.contract.trust.clone());
                let mut callees = HashSet::new();
                collect_calls(&f.body.node, &mut callees);
                calls.insert(f.name.clone(), callees);
            }
            Decl::Flow(f) => {
                declared.insert(f.name.clone(), f.contract.trust.clone());
                let mut callees = HashSet::new();
                for step in &f.steps {
                    collect_calls(&step.body.node, &mut callees);
                    collect_calls(&step.rollback.node, &mut callees);
                }
                calls.insert(f.name.clone(), callees);
            }
            Decl::Capability(c) => {
                declared.insert(c.name.clone(), c.trust.clone());
                calls.insert(c.name.clone(), HashSet::new());
            }
            _ => {}
        }
    }

    // Step 2: iteratively propagate. effective starts as declared, then we
    // walk the call graph taking the MEET of each caller with its callees.
    let mut effective = declared.clone();
    let mut witness: HashMap<String, String> = HashMap::new();

    let mut changed = true;
    let mut iterations = 0;
    // Bound iterations to prevent pathological cycles; in practice this
    // converges in O(depth of call graph) iterations.
    while changed && iterations < 100 {
        changed = false;
        iterations += 1;

        for (caller, callees) in &calls {
            let mut new_trust = effective.get(caller).cloned().unwrap_or(TrustLevel::Checked);
            let mut new_witness: Option<String> = None;

            for callee in callees {
                if let Some(callee_trust) = effective.get(callee) {
                    let combined = meet(&new_trust, callee_trust);
                    if combined != new_trust {
                        new_witness = Some(callee.clone());
                        new_trust = combined;
                    }
                }
                // Builtins (not in effective) are treated as Checked — they
                // don't downgrade but don't upgrade either. That's the most
                // honest default: we haven't proved them, we rely on them.
            }

            let current = effective.get(caller).cloned().unwrap_or(TrustLevel::Checked);
            if new_trust != current {
                effective.insert(caller.clone(), new_trust);
                if let Some(w) = new_witness {
                    witness.insert(caller.clone(), w);
                }
                changed = true;
            }
        }
    }

    // Step 3: compute the downgrade list
    let mut downgraded = Vec::new();
    for (name, decl_trust) in &declared {
        let eff_trust = effective.get(name).cloned().unwrap_or(TrustLevel::Checked);
        if &eff_trust != decl_trust {
            let w = witness.get(name).cloned().unwrap_or_else(|| "(transitive)".into());
            downgraded.push((name.clone(), decl_trust.clone(), eff_trust, w));
        }
    }

    TrustAnalysis {
        effective,
        declared,
        downgraded,
    }
}

/// Walk an expression tree and collect all called function names.
fn collect_calls(expr: &Expr, out: &mut HashSet<String>) {
    match expr {
        Expr::Call { func, args } => {
            if let Expr::Ident(name) = &func.node {
                out.insert(name.clone());
            }
            collect_calls(&func.node, out);
            for arg in args {
                collect_calls(&arg.node, out);
            }
        }
        Expr::Pipe { value, func, args } => {
            collect_calls(&value.node, out);
            if let Expr::Ident(name) = &func.node {
                out.insert(name.clone());
            }
            for arg in args {
                collect_calls(&arg.node, out);
            }
        }
        Expr::Let { value, .. } => collect_calls(&value.node, out),
        Expr::Block(stmts) => {
            for s in stmts {
                collect_calls(&s.node, out);
            }
        }
        Expr::If { condition, then_branch, else_branch } => {
            collect_calls(&condition.node, out);
            collect_calls(&then_branch.node, out);
            if let Some(eb) = else_branch {
                collect_calls(&eb.node, out);
            }
        }
        Expr::Match { scrutinee, arms } => {
            collect_calls(&scrutinee.node, out);
            for arm in arms {
                collect_calls(&arm.body.node, out);
            }
        }
        Expr::BinOp { left, right, .. } => {
            collect_calls(&left.node, out);
            collect_calls(&right.node, out);
        }
        Expr::UnaryOp { operand, .. } => collect_calls(&operand.node, out),
        Expr::Try(inner) | Expr::Return(inner) | Expr::Transaction(inner)
        | Expr::Ok(inner) | Expr::Err(inner) => collect_calls(&inner.node, out),
        Expr::FieldAccess { object, .. } => collect_calls(&object.node, out),
        Expr::Record { fields, .. } => {
            for (_, v) in fields {
                collect_calls(&v.node, out);
            }
        }
        Expr::RecordUpdate { base, updates } => {
            collect_calls(&base.node, out);
            for (_, v) in updates {
                collect_calls(&v.node, out);
            }
        }
        Expr::Tuple(items) | Expr::List(items) => {
            for i in items {
                collect_calls(&i.node, out);
            }
        }
        Expr::Require { condition, else_expr } => {
            collect_calls(&condition.node, out);
            if let Some(e) = else_expr {
                collect_calls(&e.node, out);
            }
        }
        Expr::Lambda { body, .. } => collect_calls(&body.node, out),
        _ => {}
    }
}

// Silence unused import warning if present
#[allow(dead_code)]
fn _unused(_: Spanned<Expr>) {}
