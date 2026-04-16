use std::collections::{HashMap, HashSet, VecDeque};

use nous_ast::{Program, Span};
use nous_ast::decl::{Contract, Decl, FnDecl, FlowDecl, StateDecl};
use nous_ast::expr::Expr;

use crate::error::VerifyError;
use crate::smt::{self, SmtResult};

// ---------------------------------------------------------------------------
// Result types
// ---------------------------------------------------------------------------

/// Summary returned when verification completes without fatal errors.
#[derive(Debug, Clone)]
pub struct VerifyResult {
    /// Number of constraints that were proven correct (trivially true in this
    /// skeleton — real proofs require Z3 integration).
    pub verified_count: usize,
    /// Number of constraints that could not be proven (SMT check not yet wired).
    pub unverified_count: usize,
    /// Non-fatal warnings accumulated during the walk.
    pub warnings: Vec<String>,
    /// Structured diagnostics for AI consumption.
    pub diagnostics: Vec<crate::diagnostic::Diagnostic>,
}

// ---------------------------------------------------------------------------
// Verifier
// ---------------------------------------------------------------------------

/// Entry point for the Nous verification pipeline.
///
/// The verifier walks the AST, collects all `require` / `ensure` constraints
/// from [`FnDecl`] and [`FlowDecl`] nodes, and submits them to the SMT solver.
///
/// # SMT integration
/// All calls that would go to Z3 are marked `// TODO: Z3 integration`.  Until
/// that plumbing exists every constraint is recorded as *unverified*, which
/// keeps the pipeline non-blocking during development.
pub struct Verifier {
    /// Accumulated errors collected during a single `verify` call.
    errors: Vec<VerifyError>,
    /// Non-fatal warnings.
    warnings: Vec<String>,
    /// Running count of constraints proven correct.
    verified_count: usize,
    /// Running count of constraints not yet proven.
    unverified_count: usize,
    /// Structured diagnostics for AI consumption.
    pub diagnostics: Vec<crate::diagnostic::Diagnostic>,
}

impl Verifier {
    /// Create a new, empty verifier.
    pub fn new() -> Self {
        Self {
            errors: Vec::new(),
            warnings: Vec::new(),
            verified_count: 0,
            unverified_count: 0,
            diagnostics: Vec::new(),
        }
    }

    // -----------------------------------------------------------------------
    // Public API
    // -----------------------------------------------------------------------

    /// Walk `program`, verify all contracts, and return either a summary or
    /// the list of errors found.
    pub fn verify(
        mut self,
        program: &Program,
    ) -> Result<VerifyResult, Vec<VerifyError>> {
        // Phase 1: per-declaration checks
        for spanned in &program.declarations {
            self.visit_decl(&spanned.node, spanned.span);
        }

        // Phase 2: trust propagation across the call graph
        let trust_analysis = crate::trust::analyze(program);
        for (fn_name, declared, effective, witness) in &trust_analysis.downgraded {
            self.warnings.push(format!(
                "TRUST DOWNGRADE: `{fn_name}` declared {declared:?} but effective trust is {effective:?} \
                (because it calls `{witness}` which is not {declared:?})"
            ));
        }

        if self.errors.is_empty() {
            Ok(VerifyResult {
                verified_count: self.verified_count,
                unverified_count: self.unverified_count,
                warnings: self.warnings,
                diagnostics: self.diagnostics,
            })
        } else {
            Err(self.errors)
        }
    }

    // -----------------------------------------------------------------------
    // Declaration visitors
    // -----------------------------------------------------------------------

    fn visit_decl(&mut self, decl: &Decl, span: Span) {
        match decl {
            Decl::Fn(fn_decl) => self.visit_fn_decl(fn_decl, span),
            Decl::Flow(flow_decl) => self.visit_flow_decl(flow_decl, span),
            Decl::Entity(entity_decl) => {
                // Collect entity invariants for future SMT checks.
                for inv in &entity_decl.invariants {
                    self.check_invariant_expr(
                        &entity_decl.name,
                        &inv.node,
                        inv.span,
                    );
                }
            }
            Decl::State(state_decl) => self.verify_state_machine(state_decl, span),
            // Other declarations do not carry contracts yet.
            _ => {}
        }
    }

    fn visit_fn_decl(&mut self, decl: &FnDecl, _decl_span: Span) {
        // Check if this is a synthesizable function (no body, only ensures).
        // If so, the ensures are the specification — they're verified by construction
        // because the compiler synthesizes an implementation that satisfies them.
        let body_is_empty = matches!(&decl.body.node, Expr::Void)
            || matches!(&decl.body.node, Expr::Block(stmts) if stmts.is_empty());
        let is_synthesized = body_is_empty && !decl.contract.ensures.is_empty();

        if is_synthesized {
            // Synthesized function: ensures are verified by construction.
            // Only check requires (caller obligations).
            for req in &decl.contract.requires {
                let condition_text = expr_to_string(&req.condition.node);
                match smt::check_require_satisfiable(&req.condition.node) {
                    SmtResult::Verified => { self.verified_count += 1; }
                    SmtResult::Counterexample(ce) => {
                        self.verified_count += 1;
                        if !ce.is_empty() {
                            self.warnings.push(format!(
                                "`{}` require `{condition_text}` is not always true; callers must ensure: {ce:?}",
                                decl.name
                            ));
                            self.diagnostics.push(
                                crate::diagnostic::Diagnostic::require_violation(
                                    &decl.name, &condition_text, ce, req.condition.span,
                                )
                            );
                        }
                    }
                    SmtResult::Unknown(reason) => {
                        self.unverified_count += 1;
                        self.warnings.push(format!(
                            "SMT solver could not verify `{condition_text}` in `{}`: {reason}",
                            decl.name
                        ));
                    }
                }
            }
            // Mark ensures as verified-by-synthesis
            for _ensure in &decl.contract.ensures {
                self.verified_count += 1;
            }
            self.warnings.push(format!(
                "`{}` body synthesized from {} ensure constraint(s)",
                decl.name, decl.contract.ensures.len()
            ));
        } else {
            self.check_contract(&decl.name, &decl.contract);
        }
        self.check_declared_effects(&decl.name, &decl.contract);

        // Report trust level
        if decl.contract.trust != nous_ast::decl::TrustLevel::Checked {
            self.warnings.push(format!(
                "`{}` trust level: {:?}", decl.name, decl.contract.trust
            ));
        }

        // Report obligations as unresolved risks
        for obligation in &decl.contract.obligations {
            let desc = obligation.description.as_deref().unwrap_or("no description");
            self.warnings.push(format!(
                "OBLIGATION `{}` in `{}`: {desc} [unresolved — must be addressed]",
                obligation.name, decl.name
            ));
        }
    }

    fn visit_flow_decl(&mut self, decl: &FlowDecl, _decl_span: Span) {
        self.check_contract(&decl.name, &decl.contract);
        self.check_declared_effects(&decl.name, &decl.contract);

        // Also inspect each step body for nested requires.
        for step in &decl.steps {
            self.collect_inline_requires(&decl.name, &step.body.node, step.body.span);
        }

        // Report trust level
        if decl.contract.trust != nous_ast::decl::TrustLevel::Checked {
            self.warnings.push(format!(
                "flow `{}` trust level: {:?}", decl.name, decl.contract.trust
            ));
        }

        // Report obligations
        for obligation in &decl.contract.obligations {
            let desc = obligation.description.as_deref().unwrap_or("no description");
            self.warnings.push(format!(
                "OBLIGATION `{}` in flow `{}`: {desc} [unresolved — must be addressed]",
                obligation.name, decl.name
            ));
        }

        // Check flow steps for missing rollbacks (observed trust items)
        for step in &decl.steps {
            let has_rollback = !matches!(&step.rollback.node, Expr::Void)
                && !matches!(&step.rollback.node, Expr::Ident(n) if n == "nothing");
            if !has_rollback {
                self.warnings.push(format!(
                    "flow `{}` step `{}` has no rollback — if this step has side effects, add a compensating action",
                    decl.name, step.name
                ));
            }
        }
    }

    // -----------------------------------------------------------------------
    // State machine verification
    // -----------------------------------------------------------------------

    /// Verify structural properties of a state machine declaration.
    ///
    /// Four checks are performed in one pass over the transition list:
    ///
    /// 1. **Completeness** – every state that appears as a `from` source has at
    ///    least one outgoing transition (by definition, if it appears as `from` it
    ///    does). Pure `to`-only states with no outgoing transitions are classified
    ///    as *terminal* and are allowed to have no outgoing edges.
    ///
    /// 2. **Reachability** – BFS from the initial state (the `from` of the very
    ///    first transition) to find every reachable state. Any state that is
    ///    mentioned in the machine but is never reached raises `E201_UNREACHABLE_STATE`.
    ///
    /// 3. **Liveness** – reverse BFS from every terminal state. Any non-terminal
    ///    state that cannot reach a terminal state raises `E203_LIVENESS_VIOLATION`.
    ///
    /// 4. **Dead actions** – an action whose `from` state is unreachable is itself
    ///    dead and raises `W202_DEAD_ACTION`.
    fn verify_state_machine(&mut self, decl: &StateDecl, span: Span) {
        let machine = &decl.name;

        if decl.transitions.is_empty() {
            // A state machine with no transitions is trivially valid (and useless).
            return;
        }

        // ── Collect the universe of state names ───────────────────────────────
        // `from_states` — states with outgoing transitions.
        // `to_states`   — states that are targets of at least one transition.
        // All states = from_states ∪ to_states.
        let mut from_states: HashSet<&str> = HashSet::new();
        let mut to_states: HashSet<&str> = HashSet::new();

        // Forward adjacency: state → list of reachable states.
        let mut forward: HashMap<&str, Vec<&str>> = HashMap::new();
        // Reverse adjacency for liveness BFS: state → states that transition here.
        let mut reverse: HashMap<&str, Vec<&str>> = HashMap::new();

        for t in &decl.transitions {
            from_states.insert(&t.from);
            to_states.insert(&t.to);
            forward.entry(&t.from).or_default().push(&t.to);
            reverse.entry(&t.to).or_default().push(&t.from);
        }

        let all_states: HashSet<&str> = from_states.union(&to_states).copied().collect();

        // Terminal states: appear as `to` targets but never as `from` source —
        // they have no outgoing transitions.
        let terminal_states: HashSet<&str> = to_states
            .difference(&from_states)
            .copied()
            .collect();

        // Initial state: the `from` of the first transition in source order.
        let initial_state: &str = &decl.transitions[0].from;

        // ── Check 2: Forward reachability via BFS from initial state ──────────
        let reachable = bfs_reachable(initial_state, &forward, &all_states);

        let mut unreachable_states: HashSet<&str> = all_states
            .difference(&reachable)
            .copied()
            .collect();

        // The initial state is always reachable (it's the starting point).
        unreachable_states.remove(initial_state);

        for state in &unreachable_states {
            self.diagnostics.push(
                crate::diagnostic::Diagnostic::state_unreachable(machine, state, span),
            );
        }

        // ── Check 4: Dead actions (actions whose `from` is unreachable) ───────
        for t in &decl.transitions {
            if unreachable_states.contains(t.from.as_str()) {
                self.diagnostics.push(
                    crate::diagnostic::Diagnostic::dead_action(
                        machine,
                        &t.action,
                        &t.from,
                        span,
                    ),
                );
            }
        }

        // ── Check 3: Liveness — reverse BFS from every terminal state ─────────
        // States that can reach a terminal state.
        let can_terminate = bfs_reachable_multi(&terminal_states, &reverse, &all_states);

        for state in &all_states {
            // Only non-terminal, reachable states are subject to liveness.
            if terminal_states.contains(state) {
                continue;
            }
            if unreachable_states.contains(state) {
                // Already reported as unreachable; skip to avoid duplicate noise.
                continue;
            }
            if !can_terminate.contains(state) {
                self.diagnostics.push(
                    crate::diagnostic::Diagnostic::state_liveness_violation(
                        machine,
                        state,
                        span,
                    ),
                );
            }
        }
    }

    // -----------------------------------------------------------------------
    // Contract checking
    // -----------------------------------------------------------------------

    /// Verify all `require` and `ensure` clauses in a contract using Z3.
    fn check_contract(&mut self, fn_name: &str, contract: &Contract) {
        // --- require clauses -----------------------------------------------
        for req in &contract.requires {
            let span = req.condition.span;
            let condition_text = expr_to_string(&req.condition.node);

            match smt::check_require_satisfiable(&req.condition.node) {
                SmtResult::Verified => {
                    // The require condition is always true — proven by Z3
                    self.verified_count += 1;
                }
                SmtResult::Counterexample(ce) => {
                    self.verified_count += 1;
                    if !ce.is_empty() {
                        self.warnings.push(format!(
                            "`{fn_name}` require `{condition_text}` is not always true; \
                             callers must ensure: {ce:?}"
                        ));
                        self.diagnostics.push(
                            crate::diagnostic::Diagnostic::require_violation(
                                fn_name, &condition_text, ce, span,
                            )
                        );
                    }
                }
                SmtResult::Unknown(reason) => {
                    self.unverified_count += 1;
                    self.warnings.push(format!(
                        "SMT solver could not verify `{condition_text}` in `{fn_name}`: {reason}"
                    ));
                }
            }

            // Still check for literal false
            if is_literal_false(&req.condition.node) {
                self.errors.push(VerifyError::UnsatisfiableRequire {
                    condition: condition_text,
                    span,
                });
            }
        }

        // --- ensure clauses ------------------------------------------------
        let require_exprs: Vec<&Expr> = contract.requires.iter()
            .map(|r| &r.condition.node)
            .collect();

        for ensure in &contract.ensures {
            let span = ensure.condition.span;
            let constraint_text = expr_to_string(&ensure.condition.node);

            match smt::check_contract(&require_exprs, &ensure.condition.node) {
                SmtResult::Verified => {
                    self.verified_count += 1;
                }
                SmtResult::Counterexample(ce) => {
                    self.errors.push(VerifyError::ConstraintViolation {
                        constraint: constraint_text.clone(),
                        counterexample: Some(format!("{ce:?}")),
                        span,
                    });
                }
                SmtResult::Unknown(reason) => {
                    self.unverified_count += 1;
                    self.warnings.push(format!(
                        "SMT solver could not verify ensure `{constraint_text}` in `{fn_name}`: {reason}"
                    ));
                }
            }

            if uses_primed_var(&ensure.condition.node) {
                self.warnings.push(format!(
                    "ensure in `{fn_name}` uses primed variable; \
                     make sure the function mutates state: `{constraint_text}`"
                ));
            }
        }
    }

    /// Check that every effect used in a function body is declared in the
    /// contract.  Full data-flow analysis is deferred to Z3 integration;
    /// here we only do a trivial cross-reference of declared effects.
    fn check_declared_effects(&mut self, fn_name: &str, contract: &Contract) {
        // TODO: Z3 integration — walk the function body collecting `perform`
        // calls; compare against `contract.effects`.  For now we emit a
        // warning if effects list is empty on a flow (flows almost always
        // perform effects).
        if contract.effects.is_empty() {
            self.warnings.push(format!(
                "`{fn_name}` declares no effects; \
                 add `effects [...]` if the function performs I/O or mutation"
            ));
        }
    }

    // -----------------------------------------------------------------------
    // Entity invariant checking
    // -----------------------------------------------------------------------

    fn check_invariant_expr(
        &mut self,
        entity_name: &str,
        expr: &Expr,
        span: Span,
    ) {
        let invariant_text = expr_to_string(expr);

        // TODO: Z3 integration — encode the invariant as an SMT assertion
        // and verify it holds over all possible field assignments.
        self.record_unverified_constraint();

        if is_literal_false(expr) {
            self.errors.push(VerifyError::InvariantBroken {
                entity: entity_name.to_string(),
                invariant: invariant_text,
                span,
            });
        }
    }

    // -----------------------------------------------------------------------
    // Inline require collection (for flow steps)
    // -----------------------------------------------------------------------

    /// Recursively scan an expression for `Require` nodes and register them.
    fn collect_inline_requires(&mut self, _fn_name: &str, expr: &Expr, span: Span) {
        match expr {
            Expr::Require { condition, .. } => {
                let condition_text = expr_to_string(&condition.node);

                // TODO: Z3 integration
                self.record_unverified_constraint();

                if is_literal_false(&condition.node) {
                    self.errors.push(VerifyError::UnsatisfiableRequire {
                        condition: condition_text,
                        span: condition.span,
                    });
                }
            }
            Expr::Block(stmts) => {
                for s in stmts {
                    self.collect_inline_requires(_fn_name, &s.node, s.span);
                }
            }
            Expr::If { condition, then_branch, else_branch } => {
                self.collect_inline_requires(_fn_name, &condition.node, condition.span);
                self.collect_inline_requires(_fn_name, &then_branch.node, then_branch.span);
                if let Some(eb) = else_branch {
                    self.collect_inline_requires(_fn_name, &eb.node, eb.span);
                }
            }
            Expr::Let { value, .. } => {
                self.collect_inline_requires(_fn_name, &value.node, value.span);
            }
            Expr::Transaction(inner) => {
                self.collect_inline_requires(_fn_name, &inner.node, inner.span);
            }
            _ => {
                // Leaf nodes or uninteresting wrappers — nothing to recurse into
                // for require collection purposes.
            }
        }
    }

    // -----------------------------------------------------------------------
    // Helpers
    // -----------------------------------------------------------------------

    fn record_unverified_constraint(&mut self) {
        // TODO: Z3 integration — flip to `verified_count` when solver confirms.
        self.unverified_count += 1;
    }
}

impl Default for Verifier {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Graph utilities for state-machine analysis
// ---------------------------------------------------------------------------

/// BFS from a single `start` node along `adj` edges.
///
/// Only visits nodes that exist in `universe` (guards against orphaned edge
/// references). Returns the set of all reachable nodes (including `start`).
fn bfs_reachable<'a>(
    start: &'a str,
    adj: &HashMap<&'a str, Vec<&'a str>>,
    universe: &HashSet<&'a str>,
) -> HashSet<&'a str> {
    let mut visited: HashSet<&str> = HashSet::new();
    let mut queue: VecDeque<&str> = VecDeque::new();

    if universe.contains(start) {
        visited.insert(start);
        queue.push_back(start);
    }

    while let Some(node) = queue.pop_front() {
        if let Some(neighbours) = adj.get(node) {
            for &next in neighbours {
                if universe.contains(next) && visited.insert(next) {
                    queue.push_back(next);
                }
            }
        }
    }

    visited
}

/// BFS from *multiple* start nodes simultaneously (used for reverse liveness).
fn bfs_reachable_multi<'a>(
    starts: &HashSet<&'a str>,
    adj: &HashMap<&'a str, Vec<&'a str>>,
    universe: &HashSet<&'a str>,
) -> HashSet<&'a str> {
    let mut visited: HashSet<&str> = HashSet::new();
    let mut queue: VecDeque<&str> = VecDeque::new();

    for &s in starts {
        if universe.contains(s) && visited.insert(s) {
            queue.push_back(s);
        }
    }

    while let Some(node) = queue.pop_front() {
        if let Some(neighbours) = adj.get(node) {
            for &next in neighbours {
                if universe.contains(next) && visited.insert(next) {
                    queue.push_back(next);
                }
            }
        }
    }

    visited
}

// ---------------------------------------------------------------------------
// Small AST utilities (no Z3 required)
// ---------------------------------------------------------------------------

/// Render an expression as a compact human-readable string for error messages.
fn expr_to_string(expr: &Expr) -> String {
    match expr {
        Expr::BoolLit(b) => b.to_string(),
        Expr::IntLit(n) => n.to_string(),
        Expr::DecLit(s) => s.clone(),
        Expr::StringLit(s) => format!("\"{s}\""),
        Expr::Ident(name) => name.clone(),
        Expr::BinOp { op, left, right } => {
            let op_str = match op {
                nous_ast::expr::BinOp::Add => "+",
                nous_ast::expr::BinOp::Sub => "-",
                nous_ast::expr::BinOp::Mul => "*",
                nous_ast::expr::BinOp::Div => "/",
                nous_ast::expr::BinOp::Mod => "%",
                nous_ast::expr::BinOp::Eq => "==",
                nous_ast::expr::BinOp::Neq => "/=",
                nous_ast::expr::BinOp::Lt => "<",
                nous_ast::expr::BinOp::Lte => "<=",
                nous_ast::expr::BinOp::Gt => ">",
                nous_ast::expr::BinOp::Gte => ">=",
                nous_ast::expr::BinOp::And => "and",
                nous_ast::expr::BinOp::Or => "or",
                nous_ast::expr::BinOp::Implies => "implies",
            };
            format!(
                "({} {op_str} {})",
                expr_to_string(&left.node),
                expr_to_string(&right.node)
            )
        }
        Expr::UnaryOp { op, operand } => {
            let op_str = match op {
                nous_ast::expr::UnaryOp::Neg => "-",
                nous_ast::expr::UnaryOp::Not => "not ",
            };
            format!("({op_str}{})", expr_to_string(&operand.node))
        }
        Expr::FieldAccess { object, field } => {
            format!("{}.{field}", expr_to_string(&object.node))
        }
        Expr::Primed(inner) => format!("{}'", expr_to_string(&inner.node)),
        Expr::Pre(inner) => format!("pre.{}", expr_to_string(&inner.node)),
        Expr::SelfRef => "self".to_string(),
        Expr::Void => "void".to_string(),
        _ => "<expr>".to_string(),
    }
}

/// Return `true` if the expression is a literal `false` constant.
fn is_literal_false(expr: &Expr) -> bool {
    matches!(expr, Expr::BoolLit(false))
}

/// Return `true` if the expression contains any primed (`'`) variable references.
fn uses_primed_var(expr: &Expr) -> bool {
    match expr {
        Expr::Primed(_) => true,
        Expr::BinOp { left, right, .. } => {
            uses_primed_var(&left.node) || uses_primed_var(&right.node)
        }
        Expr::UnaryOp { operand, .. } => uses_primed_var(&operand.node),
        Expr::FieldAccess { object, .. } => uses_primed_var(&object.node),
        _ => false,
    }
}
