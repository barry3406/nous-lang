use nous_ast::{Program, Span};
use nous_ast::decl::{Contract, Decl, FnDecl, FlowDecl};
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
}

impl Verifier {
    /// Create a new, empty verifier.
    pub fn new() -> Self {
        Self {
            errors: Vec::new(),
            warnings: Vec::new(),
            verified_count: 0,
            unverified_count: 0,
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
        for spanned in &program.declarations {
            self.visit_decl(&spanned.node, spanned.span);
        }

        if self.errors.is_empty() {
            Ok(VerifyResult {
                verified_count: self.verified_count,
                unverified_count: self.unverified_count,
                warnings: self.warnings,
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
            // Other declarations do not carry contracts yet.
            _ => {}
        }
    }

    fn visit_fn_decl(&mut self, decl: &FnDecl, _decl_span: Span) {
        self.check_contract(&decl.name, &decl.contract);
        self.check_declared_effects(&decl.name, &decl.contract);
    }

    fn visit_flow_decl(&mut self, decl: &FlowDecl, _decl_span: Span) {
        self.check_contract(&decl.name, &decl.contract);
        self.check_declared_effects(&decl.name, &decl.contract);

        // Also inspect each step body for nested requires.
        for step in &decl.steps {
            self.collect_inline_requires(&decl.name, &step.body.node, step.body.span);
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
                    // The condition CAN be false — this is expected for requires
                    // (they guard against bad inputs). Record as verified that
                    // the constraint is well-formed (not trivially unsatisfiable).
                    if ce.is_empty() {
                        self.verified_count += 1;
                    } else {
                        // The require is satisfiable (good — it can be met)
                        self.verified_count += 1;
                        self.warnings.push(format!(
                            "`{fn_name}` require `{condition_text}` is not always true; \
                             callers must ensure: {ce:?}"
                        ));
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
            let span = ensure.span;
            let constraint_text = expr_to_string(&ensure.node);

            match smt::check_contract(&require_exprs, &ensure.node) {
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

            if uses_primed_var(&ensure.node) {
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
