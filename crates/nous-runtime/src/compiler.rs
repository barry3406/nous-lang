use std::collections::HashMap;

use nous_ast::decl::{Contract, Decl, FnDecl, FlowDecl};
use nous_ast::expr::{BinOp as AstBinOp, Expr, MatchArm, Pattern, UnaryOp as AstUnaryOp};
use nous_ast::program::Program;
use nous_ast::span::Spanned;

use crate::bytecode::{Chunk, Module, Op};
use crate::error::CompileError;
use crate::value::Value;

// ---------------------------------------------------------------------------
// Scope / symbol table
// ---------------------------------------------------------------------------

/// Tracks local variable names → slot indices within a single function frame.
#[derive(Debug, Default)]
struct Scope {
    locals: HashMap<String, usize>,
    next_slot: usize,
}

impl Scope {
    fn define(&mut self, name: impl Into<String>) -> usize {
        let slot = self.next_slot;
        self.locals.insert(name.into(), slot);
        self.next_slot += 1;
        slot
    }

    fn resolve(&self, name: &str) -> Option<usize> {
        self.locals.get(name).copied()
    }
}

// ---------------------------------------------------------------------------
// Function registry (name → chunk index)
// ---------------------------------------------------------------------------

#[derive(Debug, Default)]
struct FnRegistry {
    map: HashMap<String, usize>,
}

impl FnRegistry {
    fn register(&mut self, name: impl Into<String>, chunk_idx: usize) {
        self.map.insert(name.into(), chunk_idx);
    }

    fn lookup(&self, name: &str) -> Option<usize> {
        self.map.get(name).copied()
    }
}

// ---------------------------------------------------------------------------
// Compiler context
// ---------------------------------------------------------------------------

/// Compiles a Nous [`Program`] into a [`Module`] of bytecode.
///
/// # Supported constructs
/// - Integer, decimal, string, bool, and void literals
/// - `let` bindings (simple identifier patterns)
/// - Block expressions
/// - Binary and unary arithmetic / comparison / logic operators
/// - `if / then / else` expressions
/// - Function calls (resolved by name)
/// - `Ok(e)` / `Err(e)` / `?` (try) wrappers
/// - `require` expressions compiled to `CheckRequire`
/// - Top-level `fn` declarations compiled to their own chunks
/// - Inline `ensure` clauses (compiled as `CheckEnsure` after the body)
///
/// # Not yet implemented
/// The items below are stubbed with TODO comments.  They will raise
/// [`CompileError::Unsupported`] if encountered.
///
/// - Flow declarations and step semantics
/// - Lambda / closure expressions
/// - Record construction and update
/// - Match expressions
/// - List and tuple constructors
/// - Pipe operator
/// - Primed variables / postcondition verification expressions
/// - Effect system integration
pub struct CompilerCtx {
    module: Module,
    fns: FnRegistry,
    lambda_count: usize,
}

impl CompilerCtx {
    /// Create a fresh compiler.
    pub fn new() -> Self {
        Self {
            module: Module::new(),
            fns: FnRegistry::default(),
            lambda_count: 0,
        }
    }

    // -----------------------------------------------------------------------
    // Entry point
    // -----------------------------------------------------------------------

    /// Compile `program` and return the resulting [`Module`].
    pub fn compile(mut self, program: &Program) -> Result<Module, CompileError> {
        // First pass: register all top-level function names so that forward
        // references in call expressions can be resolved.
        for spanned in &program.declarations {
            match &spanned.node {
                Decl::Fn(fn_decl) => {
                    // Reserve a chunk index (we'll fill it in during the
                    // second pass).
                    let placeholder = Chunk::new(&fn_decl.name);
                    let idx = self.module.add_chunk(placeholder);
                    self.fns.register(&fn_decl.name, idx);
                }
                Decl::Flow(flow_decl) => {
                    // TODO: flow compilation
                    let placeholder = Chunk::new(&flow_decl.name);
                    let idx = self.module.add_chunk(placeholder);
                    self.fns.register(&flow_decl.name, idx);
                }
                _ => {}
            }
        }

        // Second pass: compile bodies.
        for spanned in &program.declarations {
            match &spanned.node {
                Decl::Fn(fn_decl) => self.compile_fn(fn_decl)?,
                Decl::Flow(flow_decl) => self.compile_flow(flow_decl)?,
                Decl::Main(main_decl) => {
                    self.compile_main_body(&main_decl.body)?;
                }
                // Types, entities, use declarations, etc. have no bytecode.
                _ => {}
            }
        }

        Ok(self.module)
    }

    // -----------------------------------------------------------------------
    // Function compilation
    // -----------------------------------------------------------------------

    fn compile_fn(&mut self, decl: &FnDecl) -> Result<(), CompileError> {
        let chunk_idx = self
            .fns
            .lookup(&decl.name)
            .ok_or_else(|| CompileError::Internal {
                message: format!("fn `{}` not registered in first pass", decl.name),
            })?;

        let mut scope = Scope::default();
        // Parameters occupy the first local slots.
        for param in &decl.params {
            scope.define(&param.name);
        }

        let mut chunk = Chunk::new(&decl.name);
        chunk.local_count = scope.next_slot;

        // Emit runtime `require` checks at the top of the function.
        self.emit_require_checks(&mut chunk, &mut scope, &decl.contract)?;

        // Compile the function body.
        self.compile_expr(&mut chunk, &mut scope, &decl.body.node)?;

        // Emit runtime `ensure` checks after the body expression.  The body
        // result is stored in a `result` local; ensure predicates may reference
        // it by name.  The result is restored to the stack before Return.
        self.emit_ensure_checks(&mut chunk, &mut scope, &decl.contract)?;

        chunk.emit(Op::Return);
        chunk.local_count = scope.next_slot;

        self.module.chunks[chunk_idx] = chunk;
        Ok(())
    }

    /// Compile a flow declaration with saga-pattern rollback semantics.
    ///
    /// Each step's body is executed in sequence. If any step produces an Err,
    /// the flow jumps to a rollback chain that executes all completed steps'
    /// rollback expressions in reverse order, then returns the original error.
    fn compile_flow(&mut self, decl: &FlowDecl) -> Result<(), CompileError> {
        let chunk_idx = self
            .fns
            .lookup(&decl.name)
            .ok_or_else(|| CompileError::Internal {
                message: format!("flow `{}` not registered in first pass", decl.name),
            })?;

        let mut scope = Scope::default();
        for param in &decl.params {
            scope.define(&param.name);
        }

        let mut chunk = Chunk::new(&decl.name);
        chunk.local_count = scope.next_slot;

        // Emit require checks
        self.emit_require_checks(&mut chunk, &mut scope, &decl.contract)?;

        // Reserve a slot to store the error value if a step fails
        let error_slot = scope.define("_flow_error");
        chunk.local_count = scope.next_slot;

        let mut step_result_slots: Vec<usize> = Vec::new();
        let mut error_jumps: Vec<usize> = Vec::new();

        // === Compile each step ===
        for step in &decl.steps {
            // Compile step body
            self.compile_expr(&mut chunk, &mut scope, &step.body.node)?;

            // Check if result is Err
            chunk.emit(Op::Dup);
            chunk.emit(Op::IsErr);
            let not_err = chunk.emit(Op::JumpIfFalse(0));

            // Error path: store error, jump to rollback chain
            chunk.emit(Op::StoreLocal(error_slot));
            chunk.local_count = scope.next_slot;
            let to_rollback = chunk.emit(Op::Jump(0));
            error_jumps.push(to_rollback);

            // Success path
            chunk.patch_jump(not_err);

            // If it's Ok, unwrap it
            chunk.emit(Op::Dup);
            chunk.emit(Op::IsOk);
            let not_ok = chunk.emit(Op::JumpIfFalse(0));
            chunk.emit(Op::UnwrapInner);
            chunk.patch_jump(not_ok);

            // Store result in named slot
            let result_slot = scope.define(&format!("{}_result", step.name));
            chunk.emit(Op::StoreLocal(result_slot));
            chunk.local_count = scope.next_slot;
            step_result_slots.push(result_slot);
        }

        // === Success: wrap last step's result in Ok and return ===
        if let Some(&last_slot) = step_result_slots.last() {
            chunk.emit(Op::LoadLocal(last_slot));
            chunk.emit(Op::WrapOk);
        } else {
            let void_idx = chunk.add_constant(Value::Void);
            chunk.emit(Op::LoadConst(void_idx));
            chunk.emit(Op::WrapOk);
        }
        chunk.emit(Op::Return);

        // === Rollback chain ===
        // Patch all error jumps to land here
        for idx in &error_jumps {
            chunk.patch_jump(*idx);
        }

        // Execute rollbacks in reverse
        for step in decl.steps.iter().rev() {
            match &step.rollback.node {
                Expr::Void => {}
                Expr::Ident(name) if name == "nothing" => {}
                rollback_body => {
                    self.compile_expr(&mut chunk, &mut scope, rollback_body)?;
                    let discard = scope.define("_rb_discard");
                    chunk.emit(Op::StoreLocal(discard));
                    chunk.local_count = scope.next_slot;
                }
            }
        }

        // Return the original error
        chunk.emit(Op::LoadLocal(error_slot));
        chunk.emit(Op::Return);

        chunk.local_count = scope.next_slot;
        self.module.chunks[chunk_idx] = chunk;
        Ok(())
    }

    fn compile_main_body(&mut self, body: &Spanned<nous_ast::expr::Expr>) -> Result<(), CompileError> {
        let mut scope = Scope::default();
        let mut chunk = Chunk::new("<main>");

        self.compile_expr(&mut chunk, &mut scope, &body.node)?;
        chunk.emit(Op::Halt);
        chunk.local_count = scope.next_slot;

        let entry = self.module.entry;
        self.module.chunks[entry] = chunk;
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Contract helpers
    // -----------------------------------------------------------------------

    fn emit_require_checks(
        &mut self,
        chunk: &mut Chunk,
        scope: &mut Scope,
        contract: &Contract,
    ) -> Result<(), CompileError> {
        for req in &contract.requires {
            self.compile_expr_into(chunk, scope, &req.condition.node)?;
            let msg = format!("require violated: {}", expr_preview(&req.condition.node));
            chunk.emit(Op::CheckRequire(msg));
        }
        Ok(())
    }

    fn emit_ensure_checks(
        &mut self,
        chunk: &mut Chunk,
        scope: &mut Scope,
        contract: &Contract,
    ) -> Result<(), CompileError> {
        if contract.ensures.is_empty() {
            return Ok(());
        }

        // The function body result sits on top of the stack.  Save it in a
        // temp local so we can reload it for each ensure clause and restore
        // it to the top afterwards.
        let result_slot = scope.define("result");
        chunk.emit(Op::StoreLocal(result_slot));
        chunk.local_count = scope.next_slot;

        for ensure in &contract.ensures {
            // Reload the body result into the `result` slot so that any
            // reference to `result` inside the ensure expression resolves
            // to the function's return value.
            chunk.emit(Op::LoadLocal(result_slot));
            chunk.emit(Op::StoreLocal(result_slot));

            // Compile the ensure predicate.  It may reference `result`.
            self.compile_expr_into(chunk, scope, &ensure.node)?;

            let msg = format!("ensure violated: {}", expr_preview(&ensure.node));
            chunk.emit(Op::CheckEnsure(msg));
        }

        // Restore the original body result back onto the stack for `Return`.
        chunk.emit(Op::LoadLocal(result_slot));
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Expression compiler
    // -----------------------------------------------------------------------

    fn compile_expr(
        &mut self,
        chunk: &mut Chunk,
        scope: &mut Scope,
        expr: &Expr,
    ) -> Result<(), CompileError> {
        self.compile_expr_into(chunk, scope, expr)
    }

    /// Core recursive expression compiler.  Emits instructions into `chunk`
    /// that, when executed, leave one value on top of the stack.
    fn compile_expr_into(
        &mut self,
        chunk: &mut Chunk,
        scope: &mut Scope,
        expr: &Expr,
    ) -> Result<(), CompileError> {
        match expr {
            // ----------------------------------------------------------------
            // Literals
            // ----------------------------------------------------------------
            Expr::IntLit(n) => {
                let idx = chunk.add_constant(Value::Int(*n));
                chunk.emit(Op::LoadConst(idx));
            }
            Expr::DecLit(s) => {
                // Represent Dec literals as precision-0 strings until a
                // proper parsing pass assigns the precision.
                // TODO: parse decimal string into (digits, precision) pair.
                let idx = chunk.add_constant(Value::Dec(s.replace('.', ""), 0));
                chunk.emit(Op::LoadConst(idx));
            }
            Expr::StringLit(s) => {
                let idx = chunk.add_constant(Value::Text(s.clone()));
                chunk.emit(Op::LoadConst(idx));
            }
            Expr::BoolLit(b) => {
                let idx = chunk.add_constant(Value::Bool(*b));
                chunk.emit(Op::LoadConst(idx));
            }
            Expr::Void => {
                let idx = chunk.add_constant(Value::Void);
                chunk.emit(Op::LoadConst(idx));
            }

            // ----------------------------------------------------------------
            // Identifiers
            // ----------------------------------------------------------------
            Expr::Ident(name) => {
                if let Some(slot) = scope.resolve(name) {
                    chunk.emit(Op::LoadLocal(slot));
                } else if let Some(fn_idx) = self.fns.lookup(name) {
                    // Emit a reference to the function by chunk index.
                    // The arity is unknown without the AST here, so use 0 as
                    // a placeholder; a proper type-directed pass would fix this.
                    let idx = chunk.add_constant(Value::Fn {
                        name: name.clone(),
                        arity: 0,
                    });
                    chunk.emit(Op::LoadConst(idx));
                } else {
                    return Err(CompileError::UndefinedName { name: name.clone() });
                }
            }

            // ----------------------------------------------------------------
            // Let binding
            // ----------------------------------------------------------------
            Expr::Let { pattern, value, .. } => {
                // Compile the RHS first.
                self.compile_expr_into(chunk, scope, &value.node)?;

                // Bind the result into a local slot.
                match &pattern.node {
                    nous_ast::expr::Pattern::Ident(name) => {
                        let slot = scope.define(name);
                        chunk.emit(Op::StoreLocal(slot));
                        // `let` produces Void.
                        let void_idx = chunk.add_constant(Value::Void);
                        chunk.emit(Op::LoadConst(void_idx));
                        // Update local_count to track growth.
                        chunk.local_count = scope.next_slot;
                    }
                    nous_ast::expr::Pattern::Wildcard => {
                        // Discard the value.
                        let void_idx = chunk.add_constant(Value::Void);
                        chunk.emit(Op::LoadConst(void_idx));
                    }
                    _ => {
                        // TODO: destructuring patterns
                        return Err(CompileError::Unsupported {
                            description: "destructuring let patterns".into(),
                        });
                    }
                }
            }

            // ----------------------------------------------------------------
            // Block
            // ----------------------------------------------------------------
            Expr::Block(stmts) => {
                if stmts.is_empty() {
                    let idx = chunk.add_constant(Value::Void);
                    chunk.emit(Op::LoadConst(idx));
                } else {
                    for (i, stmt) in stmts.iter().enumerate() {
                        self.compile_expr_into(chunk, scope, &stmt.node)?;
                        // Discard intermediate values (all but the last).
                        // We do this by storing to a temp slot, then loading
                        // the last one.  Simpler: just store-and-discard via
                        // a scratch local for non-final statements.
                        if i < stmts.len() - 1 {
                            let scratch = scope.define("_discard");
                            chunk.emit(Op::StoreLocal(scratch));
                            chunk.local_count = scope.next_slot;
                        }
                    }
                }
            }

            // ----------------------------------------------------------------
            // Binary operations
            // ----------------------------------------------------------------
            Expr::BinOp { op, left, right } => {
                self.compile_expr_into(chunk, scope, &left.node)?;
                self.compile_expr_into(chunk, scope, &right.node)?;
                let vm_op = match op {
                    AstBinOp::Add => Op::Add,
                    AstBinOp::Sub => Op::Sub,
                    AstBinOp::Mul => Op::Mul,
                    AstBinOp::Div => Op::Div,
                    AstBinOp::Mod => Op::Mod,
                    AstBinOp::Eq => Op::Eq,
                    AstBinOp::Neq => Op::Neq,
                    AstBinOp::Lt => Op::Lt,
                    AstBinOp::Lte => Op::Lte,
                    AstBinOp::Gt => Op::Gt,
                    AstBinOp::Gte => Op::Gte,
                    AstBinOp::And => Op::And,
                    AstBinOp::Or => Op::Or,
                    AstBinOp::Implies => {
                        // `a implies b` ≡ `(not a) or b`
                        // Stack before: [left, right]
                        // We need: [not_left, right] then Or.
                        // Emit Not for the left value by swapping approach:
                        // TODO: add Swap instruction; for now use a temp local.
                        // Simplified: recompile left with Not.
                        // (Both left and right are already on the stack; this
                        // path is reached after both are emitted above, so we
                        // need a different strategy.)
                        // For now: pop right into temp, Not left, push temp back, Or.
                        let tmp = scope.define("_implies_rhs");
                        chunk.emit(Op::StoreLocal(tmp));
                        chunk.emit(Op::Not);
                        chunk.emit(Op::LoadLocal(tmp));
                        chunk.local_count = scope.next_slot;
                        Op::Or
                    }
                };
                chunk.emit(vm_op);
            }

            // ----------------------------------------------------------------
            // Unary operations
            // ----------------------------------------------------------------
            Expr::UnaryOp { op, operand } => {
                self.compile_expr_into(chunk, scope, &operand.node)?;
                match op {
                    AstUnaryOp::Neg => {
                        chunk.emit(Op::Neg);
                    }
                    AstUnaryOp::Not => {
                        chunk.emit(Op::Not);
                    }
                }
            }

            // ----------------------------------------------------------------
            // If / then / else
            // ----------------------------------------------------------------
            Expr::If { condition, then_branch, else_branch } => {
                // Compile condition.
                self.compile_expr_into(chunk, scope, &condition.node)?;

                // Emit a conditional jump (target patched later).
                let jif_idx = chunk.emit(Op::JumpIfFalse(0));

                // Then branch.
                self.compile_expr_into(chunk, scope, &then_branch.node)?;

                if let Some(eb) = else_branch {
                    // Jump over the else branch.
                    let jmp_idx = chunk.emit(Op::Jump(0));
                    chunk.patch_jump(jif_idx);
                    self.compile_expr_into(chunk, scope, &eb.node)?;
                    chunk.patch_jump(jmp_idx);
                } else {
                    // No else: produce Void.
                    chunk.patch_jump(jif_idx);
                    let void_idx = chunk.add_constant(Value::Void);
                    chunk.emit(Op::LoadConst(void_idx));
                }
            }

            // ----------------------------------------------------------------
            // Function calls
            // ----------------------------------------------------------------
            Expr::Call { func, args } => {
                // Resolve callee first; push Fn value, then arguments.
                match &func.node {
                    Expr::Ident(name) => {
                        if self.fns.lookup(name).is_some() {
                            // Push the Fn constant before arguments so the VM
                            // can find it at stack[top - arg_count - 1].
                            let fn_val_idx = chunk.add_constant(Value::Fn {
                                name: name.clone(),
                                arity: args.len(),
                            });
                            chunk.emit(Op::LoadConst(fn_val_idx));
                        } else {
                            return Err(CompileError::UndefinedName { name: name.clone() });
                        }
                    }
                    _ => {
                        // TODO: first-class function call via Fn value on stack
                        return Err(CompileError::Unsupported {
                            description: "non-identifier callee in call expression".into(),
                        });
                    }
                }
                // Push arguments left-to-right.
                for arg in args {
                    self.compile_expr_into(chunk, scope, &arg.node)?;
                }
                chunk.emit(Op::Call(args.len()));
            }

            // ----------------------------------------------------------------
            // Ok / Err constructors
            // ----------------------------------------------------------------
            Expr::Ok(inner) => {
                self.compile_expr_into(chunk, scope, &inner.node)?;
                chunk.emit(Op::WrapOk);
            }
            Expr::Err(inner) => {
                self.compile_expr_into(chunk, scope, &inner.node)?;
                chunk.emit(Op::WrapErr);
            }

            // ----------------------------------------------------------------
            // Try / error propagation (`?`)
            // ----------------------------------------------------------------
            Expr::Try(inner) => {
                self.compile_expr_into(chunk, scope, &inner.node)?;
                chunk.emit(Op::Unwrap);
            }

            // ----------------------------------------------------------------
            // Inline `require` expressions
            // ----------------------------------------------------------------
            Expr::Require { condition, .. } => {
                self.compile_expr_into(chunk, scope, &condition.node)?;
                let msg = format!("require violated: {}", expr_preview(&condition.node));
                chunk.emit(Op::CheckRequire(msg));
                // `require` produces Void on success.
                let void_idx = chunk.add_constant(Value::Void);
                chunk.emit(Op::LoadConst(void_idx));
            }

            // ----------------------------------------------------------------
            // Return
            // ----------------------------------------------------------------
            Expr::Return(val) => {
                self.compile_expr_into(chunk, scope, &val.node)?;
                chunk.emit(Op::Return);
                // Unreachable from here, but push Void to keep the stack
                // consistent for the surrounding expression context.
                let void_idx = chunk.add_constant(Value::Void);
                chunk.emit(Op::LoadConst(void_idx));
            }

            // ----------------------------------------------------------------
            // Field access
            // ----------------------------------------------------------------
            Expr::FieldAccess { object, field } => {
                self.compile_expr_into(chunk, scope, &object.node)?;
                chunk.emit(Op::LoadField(field.clone()));
            }

            // ----------------------------------------------------------------
            // SelfRef
            // ----------------------------------------------------------------
            Expr::SelfRef => {
                if let Some(slot) = scope.resolve("self") {
                    chunk.emit(Op::LoadLocal(slot));
                } else {
                    return Err(CompileError::UndefinedName { name: "self".into() });
                }
            }

            // ----------------------------------------------------------------
            // TODO: unimplemented constructs
            // ----------------------------------------------------------------
            Expr::Record { name, fields } => {
                let field_names: Vec<String> = fields.iter().map(|(n, _)| n.clone()).collect();
                for (_, val) in fields {
                    self.compile_expr_into(chunk, scope, &val.node)?;
                }
                chunk.emit(Op::MakeRecord {
                    name: name.clone(),
                    field_names,
                });
            }
            Expr::RecordUpdate { base, updates } => {
                self.compile_expr_into(chunk, scope, &base.node)?;
                for (field, val) in updates {
                    self.compile_expr_into(chunk, scope, &val.node)?;
                    chunk.emit(Op::UpdateField(field.clone()));
                }
            }
            Expr::Tuple(elems) => {
                // TODO: tuple construction
                for elem in elems {
                    self.compile_expr_into(chunk, scope, &elem.node)?;
                }
                chunk.emit(Op::MakeTuple(elems.len()));
            }
            Expr::List(elems) => {
                // TODO: list construction
                for elem in elems {
                    self.compile_expr_into(chunk, scope, &elem.node)?;
                }
                chunk.emit(Op::MakeList(elems.len()));
            }
            Expr::Lambda { params, body } => {
                // Compile the lambda as a standalone anonymous function chunk.
                let lambda_id = self.lambda_count;
                self.lambda_count += 1;
                let lambda_name = format!("_lambda_{}", lambda_id);

                // Build the lambda's scope with its parameters as locals.
                let mut lambda_scope = Scope::default();
                for param in params {
                    lambda_scope.define(&param.name);
                }

                let mut lambda_chunk = Chunk::new(&lambda_name);
                lambda_chunk.local_count = lambda_scope.next_slot;

                // Compile the body into the lambda's chunk.
                self.compile_expr_into(&mut lambda_chunk, &mut lambda_scope, &body.node)?;
                lambda_chunk.emit(Op::Return);
                lambda_chunk.local_count = lambda_scope.next_slot;

                // Add the lambda's chunk to the module and register it.
                let lambda_idx = self.module.add_chunk(lambda_chunk);
                self.fns.register(&lambda_name, lambda_idx);

                // In the parent chunk, emit a LoadConst of a Fn value pointing
                // to the lambda's chunk.
                let fn_val_idx = chunk.add_constant(Value::Fn {
                    name: lambda_name,
                    arity: params.len(),
                });
                chunk.emit(Op::LoadConst(fn_val_idx));
            }
            Expr::Pipe { value, func, args } => {
                // Desugar: `x |> f(a, b)` → `f(x, a, b)`,  `x |> f` → `f(x)`
                let name = match &func.node {
                    Expr::Ident(n) => n,
                    _ => {
                        return Err(CompileError::Unsupported {
                            description: "non-identifier callee in pipe expression".into(),
                        });
                    }
                };
                if self.fns.lookup(name).is_none() {
                    return Err(CompileError::UndefinedName { name: name.clone() });
                }
                let total_args = 1 + args.len();
                let fn_val_idx = chunk.add_constant(Value::Fn {
                    name: name.clone(),
                    arity: total_args,
                });
                chunk.emit(Op::LoadConst(fn_val_idx));
                // Push the left-hand side as the first argument.
                self.compile_expr_into(chunk, scope, &value.node)?;
                // Push any additional arguments.
                for arg in args {
                    self.compile_expr_into(chunk, scope, &arg.node)?;
                }
                chunk.emit(Op::Call(total_args));
            }
            Expr::Match { scrutinee, arms } => {
                self.compile_match(chunk, scope, scrutinee, arms)?;
            }
            Expr::Transaction(inner) => {
                // TODO: transactional semantics / rollback
                self.compile_expr_into(chunk, scope, &inner.node)?;
            }
            Expr::Primed(_) | Expr::Pre(_) => {
                // TODO: pre/post state tracking for verification
                return Err(CompileError::Unsupported {
                    description: "primed / pre-state expression (verification context only)".into(),
                });
            }
        }

        Ok(())
    }

    // -----------------------------------------------------------------------
    // Match expression compilation
    // -----------------------------------------------------------------------

    /// Compile a `match` expression.
    ///
    /// Strategy:
    /// 1. Compile the scrutinee and store it in a fresh temp local so it can
    ///    be reloaded cheaply for each arm without re-evaluating it.
    /// 2. For each arm:
    ///    a. Load the temp, then emit a pattern check that leaves a `Bool` on
    ///       the stack (`compile_pattern_check`).
    ///    b. Emit `JumpIfFalse` to skip this arm.
    ///    c. If the pattern binds variables, emit loads that place the captured
    ///       values into their named local slots.
    ///    d. Compile the arm body.
    ///    e. Emit `Jump(0)` to jump past all remaining arms; record the index
    ///       for back-patching.
    ///    f. Patch the `JumpIfFalse` so it lands here (start of next arm).
    /// 3. After all arms, emit a runtime panic for non-exhaustive match: push
    ///    an error string constant and `Halt` (the cleanest available
    ///    signalling mechanism given the current instruction set).
    fn compile_match(
        &mut self,
        chunk: &mut Chunk,
        scope: &mut Scope,
        scrutinee: &Spanned<Expr>,
        arms: &[MatchArm],
    ) -> Result<(), CompileError> {
        // 1. Compile scrutinee and store in a temp local.
        self.compile_expr_into(chunk, scope, &scrutinee.node)?;
        let scrutinee_slot = scope.define("_match_scrutinee");
        chunk.emit(Op::StoreLocal(scrutinee_slot));
        chunk.local_count = scope.next_slot;

        // Collect jump indices that need to be patched to the end.
        let mut end_jumps: Vec<usize> = Vec::new();

        for arm in arms {
            // 2a. Load the scrutinee for this arm's check.
            chunk.emit(Op::LoadLocal(scrutinee_slot));

            // Emit the pattern test; leaves Bool on the stack.
            // For Wildcard/Ident the test always succeeds — we emit `true`.
            let always_matches = self.emit_pattern_test(chunk, scope, &arm.pattern.node)?;

            // 2b. Jump past this arm if the test failed.
            let skip_arm_jump = if always_matches {
                None
            } else {
                Some(chunk.emit(Op::JumpIfFalse(0)))
            };

            // 2c. Bind pattern variables and set up the arm's local scope.
            //     The scrutinee is still in `scrutinee_slot`; bindings that
            //     need sub-values reload it (or use `Dup`/`UnwrapInner`).
            self.emit_pattern_bindings(chunk, scope, &arm.pattern.node, scrutinee_slot)?;

            // 2d. Compile the arm body.
            self.compile_expr_into(chunk, scope, &arm.body.node)?;

            // 2e. Jump to end (past remaining arms).
            let end_jump = chunk.emit(Op::Jump(0));
            end_jumps.push(end_jump);

            // 2f. Patch the skip-this-arm jump to land here.
            if let Some(idx) = skip_arm_jump {
                chunk.patch_jump(idx);
            }
        }

        // 3. Non-exhaustive match: emit a Halt with a sentinel error value.
        //    We push a descriptive Err string so the halted program result
        //    carries useful information.
        let err_msg_idx =
            chunk.add_constant(Value::Text("non-exhaustive match".into()));
        chunk.emit(Op::LoadConst(err_msg_idx));
        chunk.emit(Op::WrapErr);
        chunk.emit(Op::Halt);

        // Patch all end-of-arm jumps to land here (after the Halt).
        for idx in end_jumps {
            chunk.patch_jump(idx);
        }

        Ok(())
    }

    /// Emit instructions that test whether the top-of-stack value (the
    /// scrutinee, already loaded) matches `pattern`.  The instructions leave
    /// one `Bool` on top of the original scrutinee value — i.e. after this
    /// call the stack has grown by exactly one `Bool`.
    ///
    /// Returns `true` if the pattern always matches (Wildcard / Ident), in
    /// which case no `JumpIfFalse` is needed.
    fn emit_pattern_test(
        &mut self,
        chunk: &mut Chunk,
        scope: &mut Scope,
        pattern: &Pattern,
    ) -> Result<bool, CompileError> {
        match pattern {
            // These always match. We return `true` so `compile_match` skips
            // emitting a `JumpIfFalse`. We do NOT push any extra value onto
            // the stack; `emit_pattern_bindings` reloads from `scrutinee_slot`
            // when it needs to. The scrutinee that the caller loaded for this
            // test pass is popped off below.
            Pattern::Wildcard | Pattern::Ident(_) => {
                // Pop the scrutinee we loaded; it is not needed for the test.
                // `emit_pattern_bindings` will reload it from scrutinee_slot.
                let discard = scope.define("_pat_discard");
                chunk.emit(Op::StoreLocal(discard));
                chunk.local_count = scope.next_slot;
                Ok(true)
            }

            Pattern::Literal(lit_expr) => {
                // Stack: [..., scrutinee]
                // Push the literal value, then Eq.
                self.compile_expr_into(chunk, scope, lit_expr)?;
                chunk.emit(Op::Eq);
                // Stack: [..., Bool]  (scrutinee consumed by Eq)
                Ok(false)
            }

            Pattern::Constructor { name, fields } => {
                // Stack going in: [..., scrutinee]
                // We peek (non-consuming), push a Bool, then pop the scrutinee.
                // The result is: [..., Bool].
                // `emit_pattern_bindings` reloads from scrutinee_slot directly.
                match name.as_str() {
                    "Ok" => {
                        chunk.emit(Op::IsOk);
                        // Stack: [..., scrutinee, Bool]
                        let _ = fields; // inner checks deferred to bindings
                    }
                    "Err" => {
                        chunk.emit(Op::IsErr);
                    }
                    _ => {
                        chunk.emit(Op::IsVariant(name.clone()));
                    }
                }
                // Stack: [..., scrutinee, Bool]
                // Swap Bool and scrutinee, then discard scrutinee.
                // We don't have Swap, so store Bool in a temp, pop scrutinee, reload Bool.
                let bool_slot = scope.define("_constructor_test_bool");
                chunk.emit(Op::StoreLocal(bool_slot)); // store Bool; stack: [..., scrutinee]
                chunk.local_count = scope.next_slot;
                let discard_slot = scope.define("_constructor_scrutinee_discard");
                chunk.emit(Op::StoreLocal(discard_slot)); // store scrutinee; stack: [...]
                chunk.local_count = scope.next_slot;
                chunk.emit(Op::LoadLocal(bool_slot)); // stack: [..., Bool]
                Ok(false)
            }

            Pattern::Tuple(sub_patterns) => {
                // Check that each element matches its sub-pattern.
                // For simplicity (no nested jump patching), we compile this
                // as a sequence of AND-ed checks.
                //
                // Stack going in: [..., scrutinee (Tuple)]
                // We emit: Dup, index into element 0, test, AND, ...
                //
                // Because we lack a TupleIndex instruction, we use a temp
                // local to hold the Tuple and load it repeatedly.
                let tuple_slot = scope.define("_tuple_scrutinee");
                chunk.emit(Op::StoreLocal(tuple_slot));
                chunk.local_count = scope.next_slot;

                // Start with `true`.
                let t_idx = chunk.add_constant(Value::Bool(true));
                chunk.emit(Op::LoadConst(t_idx));

                for (i, sub_pat) in sub_patterns.iter().enumerate() {
                    // Load the i-th element via a temp store trick:
                    // We store the element in a temp, test it, AND with accumulator.
                    // Without TupleIndex we must load the whole tuple and extract.
                    // Use a helper constant index to simulate tuple indexing with
                    // a sequence: LoadLocal(tuple_slot) + MakeList trick...
                    //
                    // Since there is no TupleIndex op, we fall back to storing the
                    // whole tuple in a local and using a Rust-side helper that
                    // emits a field-extraction via a known positional index.
                    // The simplest approach: push a fake "always true" for now and
                    // perform the binding at bind-time, which reads the actual
                    // element slot.  For the *test* pass we only need the Bool.
                    //
                    // Reality: if the sub-pattern is Wildcard/Ident we skip;
                    // for Literal we'd need an index.  Since the VM has no
                    // TupleIndex, we emit IsOk/IsErr/IsVariant checks at the
                    // top-level only, and defer element checks to runtime errors.
                    //
                    // Since there is no TupleIndex op, sub-pattern checking
                    // for tuple elements is deferred. For now we only check
                    // that the scrutinee is a Tuple (always true if the program
                    // type-checks). Element bindings are done in
                    // `emit_pattern_bindings` via the `_tuple_scrutinee` slot.
                    let _ = i;
                    let _ = sub_pat;
                }

                // Stack is now [..., Bool(true)].
                // The tuple is stored in tuple_slot; the Bool is what JumpIfFalse
                // (or the caller's always_matches path) will consume.
                Ok(false)
            }
        }
    }

    /// Emit instructions to bind pattern variables after a successful test.
    ///
    /// `scrutinee_slot` is the local variable holding the full scrutinee.
    /// After this call, all names introduced by `pattern` are bound in `scope`
    /// and stored in the appropriate local slots.
    fn emit_pattern_bindings(
        &mut self,
        chunk: &mut Chunk,
        scope: &mut Scope,
        pattern: &Pattern,
        scrutinee_slot: usize,
    ) -> Result<(), CompileError> {
        match pattern {
            Pattern::Wildcard => {
                // No binding; discard the extra Bool left by emit_pattern_test.
                // (The scrutinee is still in scrutinee_slot; the Bool was pushed
                // by the test and consumed by JumpIfFalse — or if always_matches
                // we never checked it, so we need to pop the dummy true.)
                // Actually the Bool is consumed by JumpIfFalse (or skipped if
                // always_matches).  We have nothing to pop here.
            }
            Pattern::Ident(name) => {
                // Bind the scrutinee to this name.
                let slot = scope.define(name);
                chunk.emit(Op::LoadLocal(scrutinee_slot));
                chunk.emit(Op::StoreLocal(slot));
                chunk.local_count = scope.next_slot;
            }
            Pattern::Literal(_) => {
                // No variable binding; the test already consumed the scrutinee
                // via Eq.  Nothing else to do.
            }
            Pattern::Constructor { name, fields } => {
                // Extract the inner value(s) and bind sub-patterns.
                match name.as_str() {
                    "Ok" | "Err" => {
                        if let Some(inner_pat) = fields.first() {
                            // Unwrap the Ok/Err to get the inner value.
                            chunk.emit(Op::LoadLocal(scrutinee_slot));
                            chunk.emit(Op::UnwrapInner);
                            let inner_slot = scope.define("_inner");
                            chunk.emit(Op::StoreLocal(inner_slot));
                            chunk.local_count = scope.next_slot;
                            // Recursively bind the inner pattern.
                            self.emit_pattern_bindings(
                                chunk,
                                scope,
                                &inner_pat.node,
                                inner_slot,
                            )?;
                        }
                        // If there are more fields (unusual for Ok/Err), ignore them.
                    }
                    _ => {
                        // General enum: extract positional fields from
                        // `Value::Enum { fields, .. }`.
                        // We need an index-access op; since none exists we store
                        // the whole enum in a slot and bind sub-patterns to the
                        // scrutinee slot itself (field access would require a new op).
                        // For now we support only zero-field constructors and
                        // single-field bindings via a wildcard / ident.
                        for (i, sub_pat) in fields.iter().enumerate() {
                            // Use a synthetic temp name to hold the i-th field.
                            let field_slot = scope.define(&format!("_enum_field_{i}"));
                            // We can't index into Enum fields without a new op.
                            // Emit a Nop as placeholder and store Void; the
                            // binding will be Void which is semantically wrong
                            // but lets the bytecode remain valid.
                            // TODO: add Op::TupleIndex(usize) / Op::EnumField(usize).
                            let void_idx = chunk.add_constant(Value::Void);
                            chunk.emit(Op::LoadConst(void_idx));
                            chunk.emit(Op::StoreLocal(field_slot));
                            chunk.local_count = scope.next_slot;
                            self.emit_pattern_bindings(
                                chunk,
                                scope,
                                &sub_pat.node,
                                field_slot,
                            )?;
                        }
                    }
                }
            }
            Pattern::Tuple(sub_patterns) => {
                // The tuple was saved in a temp local by emit_pattern_test.
                // We resolve it by name convention.
                let tuple_slot = scope
                    .resolve("_tuple_scrutinee")
                    .unwrap_or(scrutinee_slot);
                for (i, sub_pat) in sub_patterns.iter().enumerate() {
                    let elem_slot = scope.define(&format!("_tuple_elem_{i}"));
                    // Without TupleIndex we can't extract. Same limitation as
                    // enum fields — store Void and bind sub-patterns.
                    // TODO: add Op::TupleIndex(usize).
                    let void_idx = chunk.add_constant(Value::Void);
                    chunk.emit(Op::LoadConst(void_idx));
                    chunk.emit(Op::StoreLocal(elem_slot));
                    chunk.local_count = scope.next_slot;
                    self.emit_pattern_bindings(chunk, scope, &sub_pat.node, elem_slot)?;
                }
                // Reload the tuple so it's on the stack (mirroring what the
                // caller left there before the test).
                let _ = tuple_slot;
            }
        }
        Ok(())
    }
}

impl Default for CompilerCtx {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Utility
// ---------------------------------------------------------------------------

/// Return a short preview string for an expression (used in error messages).
fn expr_preview(expr: &Expr) -> String {
    match expr {
        Expr::BoolLit(b) => b.to_string(),
        Expr::IntLit(n) => n.to_string(),
        Expr::DecLit(s) => s.clone(),
        Expr::StringLit(s) => format!("\"{s}\""),
        Expr::Ident(n) => n.clone(),
        Expr::BinOp { .. } => "<binop>".into(),
        _ => "<expr>".into(),
    }
}
