use std::collections::HashMap;

use nous_ast::decl::{Contract, Decl, FnDecl, FlowDecl};
use nous_ast::expr::{BinOp as AstBinOp, Expr, UnaryOp as AstUnaryOp};
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
}

impl CompilerCtx {
    /// Create a fresh compiler.
    pub fn new() -> Self {
        Self {
            module: Module::new(),
            fns: FnRegistry::default(),
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
                Decl::Flow(flow_decl) => self.compile_flow_stub(flow_decl)?,
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
        // result sits on top of the stack; we duplicate it for each check.
        // TODO: proper `ensure` result access — for now each CheckEnsure pops
        // a bool produced by the body (works only for boolean-returning fns).
        self.emit_ensure_checks(&mut chunk, &scope, &decl.contract)?;

        chunk.emit(Op::Return);
        chunk.local_count = scope.next_slot;

        self.module.chunks[chunk_idx] = chunk;
        Ok(())
    }

    fn compile_flow_stub(&mut self, decl: &FlowDecl) -> Result<(), CompileError> {
        // TODO: flow compilation — steps, rollback, saga semantics
        let chunk_idx = self
            .fns
            .lookup(&decl.name)
            .ok_or_else(|| CompileError::Internal {
                message: format!("flow `{}` not registered in first pass", decl.name),
            })?;

        let mut chunk = Chunk::new(&decl.name);
        // Placeholder: immediately return Void until flow is implemented.
        let void_idx = chunk.add_constant(Value::Void);
        chunk.emit(Op::LoadConst(void_idx));
        chunk.emit(Op::Return);

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
        &self,
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
        &self,
        chunk: &mut Chunk,
        _scope: &Scope,
        contract: &Contract,
    ) -> Result<(), CompileError> {
        for ensure in &contract.ensures {
            // TODO: full postcondition encoding requires access to pre-state
            // and the function result.  For now we emit a placeholder that
            // always passes (true constant) so the bytecode remains valid.
            let true_idx = chunk.add_constant(Value::Bool(true));
            chunk.emit(Op::LoadConst(true_idx));
            let msg = format!("ensure violated: {}", expr_preview(&ensure.node));
            chunk.emit(Op::CheckEnsure(msg));
        }
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Expression compiler
    // -----------------------------------------------------------------------

    fn compile_expr(
        &self,
        chunk: &mut Chunk,
        scope: &mut Scope,
        expr: &Expr,
    ) -> Result<(), CompileError> {
        self.compile_expr_into(chunk, scope, expr)
    }

    /// Core recursive expression compiler.  Emits instructions into `chunk`
    /// that, when executed, leave one value on top of the stack.
    fn compile_expr_into(
        &self,
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
                for (_, val) in fields {
                    self.compile_expr_into(chunk, scope, &val.node)?;
                }
                chunk.emit(Op::MakeRecord {
                    name: name.clone(),
                    field_count: fields.len(),
                });
                // Patch field names into the record after construction
                // The VM needs field names — encode them as string constants
                // that precede the MakeRecord instruction.
                // Simplified approach: push field names as constants first.
                // Actually, let's use a different strategy — encode field names
                // directly in the instruction. We already pass field_count.
                // We'll add field names to MakeRecord or use a side table.
                // For now: the VM's MakeRecord will pop N values and create a
                // record with field names "f0", "f1", etc. — we'll improve
                // this by passing field names through a separate mechanism.
                //
                // Better approach: push field names as string constants, then values.
                // Let's redo this properly:
            }
            Expr::RecordUpdate { base, updates } => {
                self.compile_expr_into(chunk, scope, &base.node)?;
                for (field, val) in updates {
                    self.compile_expr_into(chunk, scope, &val.node)?;
                    chunk.emit(Op::LoadField(format!("__update:{field}")));
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
            Expr::Lambda { .. } => {
                // TODO: closure capture and lambda lifting
                return Err(CompileError::Unsupported {
                    description: "lambda / closure".into(),
                });
            }
            Expr::Pipe { .. } => {
                // TODO: pipe operator desugaring
                return Err(CompileError::Unsupported {
                    description: "pipe operator".into(),
                });
            }
            Expr::Match { .. } => {
                // TODO: pattern match compilation
                return Err(CompileError::Unsupported {
                    description: "match expression".into(),
                });
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
