use std::collections::BTreeMap;

use crate::bytecode::{Module, Op};
use crate::error::RuntimeError;
use crate::value::Value;

// ---------------------------------------------------------------------------
// Call frame
// ---------------------------------------------------------------------------

/// One activation record on the call stack.
#[derive(Debug, Clone)]
pub struct CallFrame {
    /// Index of the chunk being executed.
    pub chunk_idx: usize,
    /// Instruction pointer: index of the *next* [`Op`] to execute.
    pub ip: usize,
    /// Index into `Vm::stack` where this frame's locals start.
    pub stack_base: usize,
    /// Number of local variable slots allocated for this frame.
    pub local_count: usize,
}

impl CallFrame {
    fn new(chunk_idx: usize, stack_base: usize, local_count: usize) -> Self {
        Self {
            chunk_idx,
            ip: 0,
            stack_base,
            local_count,
        }
    }
}

// ---------------------------------------------------------------------------
// VM
// ---------------------------------------------------------------------------

/// Stack-based bytecode interpreter for the Nous runtime.
///
/// Execution model
/// ---------------
/// * One flat `Vec<Value>` serves as the operand stack.
/// * Each [`CallFrame`] records the chunk being executed, the current
///   instruction pointer, and the base index into the stack where that
///   frame's local variables begin.
/// * Local slots are pre-allocated at the start of each frame (filled with
///   [`Value::Void`]) and are accessed with `LoadLocal`/`StoreLocal`.
/// * The operand stack grows beyond `stack_base + local_count`; arithmetic
///   and other instructions operate on this region.
pub struct Vm {
    /// Operand + locals stack.
    stack: Vec<Value>,
    /// Active call frames (back = top of call stack).
    frames: Vec<CallFrame>,
}

impl Vm {
    /// Create a new, empty VM.
    pub fn new() -> Self {
        Self {
            stack: Vec::with_capacity(256),
            frames: Vec::with_capacity(64),
        }
    }

    // -----------------------------------------------------------------------
    // Entry point
    // -----------------------------------------------------------------------

    /// Execute `module` starting from `module.entry`, returning the top-of-stack
    /// value when `Halt` is reached, or the return value from the entry chunk.
    pub fn execute(&mut self, module: &Module) -> Result<Value, RuntimeError> {
        // Stash a reference to the module for use by builtins that need to
        // re-enter the VM (e.g. http_serve_nous dispatching to Nous handlers).
        crate::builtins::set_current_module(module.clone());

        let entry = module.entry;
        let entry_chunk = module.chunks.get(entry).ok_or(RuntimeError::UndefinedChunk { index: entry })?;

        // Push local slots for the entry chunk.
        let stack_base = self.stack.len();
        for _ in 0..entry_chunk.local_count {
            self.stack.push(Value::Void);
        }

        self.frames.push(CallFrame::new(entry, stack_base, entry_chunk.local_count));

        loop {
            let result = self.step(module)?;
            if let Some(v) = result {
                return Ok(v);
            }
        }
    }

    /// Invoke a named function in the module with arguments, returning the result.
    /// Used by builtins that need to call back into Nous code (e.g. HTTP handlers).
    pub fn invoke_fn(
        module: &Module,
        fn_name: &str,
        args: Vec<Value>,
    ) -> Result<Value, RuntimeError> {
        // Find the chunk for the named function.
        let chunk_idx = module.chunks.iter().position(|c| c.name == fn_name)
            .ok_or_else(|| RuntimeError::Internal {
                message: format!("function `{fn_name}` not found in module"),
            })?;
        let chunk = &module.chunks[chunk_idx];

        let mut vm = Vm::new();
        let stack_base = 0;
        let local_count = chunk.local_count;

        // Args go into the first local slots.
        for arg in args {
            vm.stack.push(arg);
        }
        // Fill remaining locals with Void.
        while vm.stack.len() < local_count {
            vm.stack.push(Value::Void);
        }

        vm.frames.push(CallFrame::new(chunk_idx, stack_base, local_count));

        // Run until this single frame returns.
        let target_depth = 0;
        loop {
            if vm.frames.len() <= target_depth {
                // Frame popped; return top of stack
                return Ok(vm.stack.last().cloned().unwrap_or(Value::Void));
            }
            let result = vm.step(module)?;
            if let Some(v) = result {
                return Ok(v);
            }
        }
    }

    // -----------------------------------------------------------------------
    // Single-step execution
    // -----------------------------------------------------------------------

    /// Execute one instruction.  Returns `Some(Value)` if execution should
    /// terminate (Halt or top-level Return), `None` to continue.
    fn step(&mut self, module: &Module) -> Result<Option<Value>, RuntimeError> {
        let frame = self.frames.last_mut().ok_or_else(|| RuntimeError::Internal {
            message: "call stack is empty".into(),
        })?;

        let chunk = &module.chunks[frame.chunk_idx];
        if frame.ip >= chunk.ops.len() {
            return Err(RuntimeError::Internal {
                message: format!(
                    "ip {} out of bounds for chunk `{}` (len {})",
                    frame.ip,
                    chunk.name,
                    chunk.ops.len()
                ),
            });
        }

        let op = chunk.ops[frame.ip].clone();
        frame.ip += 1;

        // Capture names for error messages before mutable borrow ends.
        let chunk_name = chunk.name.clone();
        let ip_before = frame.ip - 1;
        let stack_base = frame.stack_base;
        let local_count = frame.local_count;

        match op {
            // ----------------------------------------------------------------
            // Stack / locals
            // ----------------------------------------------------------------
            Op::LoadConst(idx) => {
                let chunk = &module.chunks[self.frames.last().unwrap().chunk_idx];
                let val = chunk.constants.get(idx).ok_or_else(|| RuntimeError::Internal {
                    message: format!("constant index {idx} out of range"),
                })?.clone();
                self.stack.push(val);
            }

            Op::LoadLocal(idx) => {
                let slot = stack_base + idx;
                if idx >= local_count {
                    return Err(RuntimeError::LocalOutOfRange {
                        index: idx,
                        frame_size: local_count,
                    });
                }
                let val = self.stack.get(slot).ok_or_else(|| RuntimeError::Internal {
                    message: format!("local slot {slot} missing from stack"),
                })?.clone();
                self.stack.push(val);
            }

            Op::StoreLocal(idx) => {
                if idx >= local_count {
                    return Err(RuntimeError::LocalOutOfRange {
                        index: idx,
                        frame_size: local_count,
                    });
                }
                let val = self.pop(&chunk_name, ip_before)?;
                let slot = stack_base + idx;
                // Extend the stack if the slot does not exist yet (locals
                // allocated lazily in the compiler).
                while self.stack.len() <= slot {
                    self.stack.push(Value::Void);
                }
                self.stack[slot] = val;
            }

            Op::LoadField(field) => {
                let record = self.pop(&chunk_name, ip_before)?;
                match record {
                    Value::Record { name, fields } => {
                        let val = fields.get(&field).ok_or_else(|| RuntimeError::MissingField {
                            field: field.clone(),
                            record: name.clone(),
                        })?.clone();
                        self.stack.push(val);
                    }
                    _ => {
                        return Err(RuntimeError::NotARecord { field });
                    }
                }
            }

            // ----------------------------------------------------------------
            // Arithmetic
            // ----------------------------------------------------------------
            Op::Add => {
                let right = self.pop(&chunk_name, ip_before)?;
                let left = self.pop(&chunk_name, ip_before)?;
                let result = self.arith_add(left, right)?;
                self.stack.push(result);
            }
            Op::Sub => {
                let right = self.pop(&chunk_name, ip_before)?;
                let left = self.pop(&chunk_name, ip_before)?;
                let result = self.arith_sub(left, right)?;
                self.stack.push(result);
            }
            Op::Mul => {
                let right = self.pop(&chunk_name, ip_before)?;
                let left = self.pop(&chunk_name, ip_before)?;
                let result = self.arith_mul(left, right)?;
                self.stack.push(result);
            }
            Op::Div => {
                let right = self.pop(&chunk_name, ip_before)?;
                let left = self.pop(&chunk_name, ip_before)?;
                let result = self.arith_div(left, right)?;
                self.stack.push(result);
            }
            Op::Mod => {
                let right = self.pop(&chunk_name, ip_before)?;
                let left = self.pop(&chunk_name, ip_before)?;
                let result = self.arith_mod(left, right)?;
                self.stack.push(result);
            }
            Op::Neg => {
                let val = self.pop(&chunk_name, ip_before)?;
                let result = match val {
                    Value::Int(n) => Value::Int(n.checked_neg().ok_or(RuntimeError::Overflow)?),
                    Value::Nat(n) => {
                        // Negating a Nat produces an Int.
                        let signed = i64::try_from(n).map_err(|_| RuntimeError::Overflow)?;
                        Value::Int(-signed)
                    }
                    other => {
                        return Err(RuntimeError::TypeMismatch {
                            expected: "numeric".into(),
                            got: other.type_name().into(),
                        })
                    }
                };
                self.stack.push(result);
            }

            // ----------------------------------------------------------------
            // Comparison
            // ----------------------------------------------------------------
            Op::Eq => {
                let right = self.pop(&chunk_name, ip_before)?;
                let left = self.pop(&chunk_name, ip_before)?;
                self.stack.push(Value::Bool(left == right));
            }
            Op::Neq => {
                let right = self.pop(&chunk_name, ip_before)?;
                let left = self.pop(&chunk_name, ip_before)?;
                self.stack.push(Value::Bool(left != right));
            }
            Op::Lt => {
                let right = self.pop(&chunk_name, ip_before)?;
                let left = self.pop(&chunk_name, ip_before)?;
                let result = self.compare_lt(left, right)?;
                self.stack.push(Value::Bool(result));
            }
            Op::Lte => {
                let right = self.pop(&chunk_name, ip_before)?;
                let left = self.pop(&chunk_name, ip_before)?;
                let result = self.compare_lte(left, right)?;
                self.stack.push(Value::Bool(result));
            }
            Op::Gt => {
                let right = self.pop(&chunk_name, ip_before)?;
                let left = self.pop(&chunk_name, ip_before)?;
                // a > b ≡ b < a
                let result = self.compare_lt(right, left)?;
                self.stack.push(Value::Bool(result));
            }
            Op::Gte => {
                let right = self.pop(&chunk_name, ip_before)?;
                let left = self.pop(&chunk_name, ip_before)?;
                // a >= b ≡ b <= a
                let result = self.compare_lte(right, left)?;
                self.stack.push(Value::Bool(result));
            }

            // ----------------------------------------------------------------
            // Logic
            // ----------------------------------------------------------------
            Op::And => {
                let right = self.pop(&chunk_name, ip_before)?;
                let left = self.pop(&chunk_name, ip_before)?;
                match (left, right) {
                    (Value::Bool(l), Value::Bool(r)) => self.stack.push(Value::Bool(l && r)),
                    (l, r) => {
                        return Err(RuntimeError::TypeMismatch {
                            expected: "Bool".into(),
                            got: format!("{} and {}", l.type_name(), r.type_name()),
                        })
                    }
                }
            }
            Op::Or => {
                let right = self.pop(&chunk_name, ip_before)?;
                let left = self.pop(&chunk_name, ip_before)?;
                match (left, right) {
                    (Value::Bool(l), Value::Bool(r)) => self.stack.push(Value::Bool(l || r)),
                    (l, r) => {
                        return Err(RuntimeError::TypeMismatch {
                            expected: "Bool".into(),
                            got: format!("{} and {}", l.type_name(), r.type_name()),
                        })
                    }
                }
            }
            Op::Not => {
                let val = self.pop(&chunk_name, ip_before)?;
                match val {
                    Value::Bool(b) => self.stack.push(Value::Bool(!b)),
                    other => {
                        return Err(RuntimeError::TypeMismatch {
                            expected: "Bool".into(),
                            got: other.type_name().into(),
                        })
                    }
                }
            }

            // ----------------------------------------------------------------
            // Control flow
            // ----------------------------------------------------------------
            Op::Jump(target) => {
                let frame = self.frames.last_mut().unwrap();
                let chunk = &module.chunks[frame.chunk_idx];
                if target > chunk.ops.len() {
                    return Err(RuntimeError::InvalidJump {
                        target,
                        chunk_name,
                        len: chunk.ops.len(),
                    });
                }
                frame.ip = target;
            }

            Op::JumpIfFalse(target) => {
                let cond = self.pop(&chunk_name, ip_before)?;
                if !cond.is_truthy() {
                    let frame = self.frames.last_mut().unwrap();
                    let chunk = &module.chunks[frame.chunk_idx];
                    if target > chunk.ops.len() {
                        return Err(RuntimeError::InvalidJump {
                            target,
                            chunk_name,
                            len: chunk.ops.len(),
                        });
                    }
                    frame.ip = target;
                }
            }

            // ----------------------------------------------------------------
            // Function call / return
            // ----------------------------------------------------------------
            Op::Call(arg_count) => {
                // The callee `Fn` value should be below the arguments on the
                // stack: [... fn_val, arg0, arg1, ..., argN-1]
                let fn_val_pos = self
                    .stack
                    .len()
                    .checked_sub(arg_count + 1)
                    .ok_or_else(|| RuntimeError::StackUnderflow {
                        chunk_name: chunk_name.clone(),
                        ip: ip_before,
                    })?;

                let fn_val = self.stack[fn_val_pos].clone();
                let (fn_name, _arity) = match &fn_val {
                    Value::Fn { name, arity } => (name.clone(), *arity),
                    other => {
                        return Err(RuntimeError::TypeMismatch {
                            expected: "Fn".into(),
                            got: other.type_name().into(),
                        })
                    }
                };

                // Look up the chunk by name.
                let callee_idx = module
                    .chunks
                    .iter()
                    .position(|c| c.name == fn_name)
                    .ok_or_else(|| RuntimeError::UndefinedChunk { index: 0 })?;

                let callee_chunk = &module.chunks[callee_idx];
                let callee_local_count = callee_chunk.local_count;

                // Remove the Fn value from the stack (args stay).
                self.stack.remove(fn_val_pos);

                // The arguments are now the last `arg_count` items on the stack.
                // The new frame's locals begin right before the arguments.
                let new_stack_base = self.stack.len() - arg_count;

                // Pre-fill additional local slots (beyond the arg slots) with Void.
                for _ in arg_count..callee_local_count {
                    self.stack.push(Value::Void);
                }

                self.frames.push(CallFrame::new(callee_idx, new_stack_base, callee_local_count));
            }

            Op::Return => {
                // The return value is on top of the stack.
                let return_val = self.pop(&chunk_name, ip_before)?;

                // Pop the current frame.
                let finished_frame = self.frames.pop().ok_or_else(|| RuntimeError::Internal {
                    message: "return with empty call stack".into(),
                })?;

                // Clean up: truncate the stack back to where this frame began.
                self.stack.truncate(finished_frame.stack_base);

                if self.frames.is_empty() {
                    // Top-level return.
                    return Ok(Some(return_val));
                }

                // Push the return value for the caller.
                self.stack.push(return_val);
            }

            // ----------------------------------------------------------------
            // Constructors
            // ----------------------------------------------------------------
            Op::MakeRecord { name, field_names } => {
                // Pop field_names.len() values (in reverse order)
                let mut values: Vec<Value> = Vec::with_capacity(field_names.len());
                for _ in 0..field_names.len() {
                    values.push(self.pop(&chunk_name, ip_before)?);
                }
                values.reverse();
                let mut fields = BTreeMap::new();
                for (k, v) in field_names.iter().zip(values) {
                    fields.insert(k.clone(), v);
                }
                self.stack.push(Value::Record { name, fields });
            }

            Op::UpdateField(field_name) => {
                // Stack: [record, new_value] — pop new_value, pop record,
                // push new record with field updated
                let new_value = self.pop(&chunk_name, ip_before)?;
                let record = self.pop(&chunk_name, ip_before)?;
                match record {
                    Value::Record { name, mut fields } => {
                        fields.insert(field_name, new_value);
                        self.stack.push(Value::Record { name, fields });
                    }
                    other => {
                        return Err(RuntimeError::TypeMismatch {
                            expected: "Record".into(),
                            got: other.type_name().into(),
                        });
                    }
                }
            }

            Op::MakeList(n) => {
                let mut items: Vec<Value> = Vec::with_capacity(n);
                for _ in 0..n {
                    items.push(self.pop(&chunk_name, ip_before)?);
                }
                items.reverse();
                self.stack.push(Value::List(items));
            }

            Op::MakeTuple(n) => {
                let mut items: Vec<Value> = Vec::with_capacity(n);
                for _ in 0..n {
                    items.push(self.pop(&chunk_name, ip_before)?);
                }
                items.reverse();
                self.stack.push(Value::Tuple(items));
            }

            Op::TupleIndex(i) => {
                let val = self.pop(&chunk_name, ip_before)?;
                match val {
                    Value::Tuple(items) => {
                        let element = items.get(i).cloned().ok_or_else(|| {
                            RuntimeError::TypeMismatch {
                                expected: format!("tuple with index {i}"),
                                got: format!("tuple of length {}", items.len()),
                            }
                        })?;
                        self.stack.push(element);
                    }
                    other => {
                        return Err(RuntimeError::TypeMismatch {
                            expected: "Tuple".into(),
                            got: other.type_name().into(),
                        });
                    }
                }
            }

            Op::EnumField(i) => {
                let val = self.pop(&chunk_name, ip_before)?;
                match val {
                    Value::Enum { fields, .. } => {
                        let element = fields.get(i).cloned().ok_or_else(|| {
                            RuntimeError::TypeMismatch {
                                expected: format!("enum variant with {} fields", i + 1),
                                got: format!("variant with {} fields", fields.len()),
                            }
                        })?;
                        self.stack.push(element);
                    }
                    other => {
                        return Err(RuntimeError::TypeMismatch {
                            expected: "Enum".into(),
                            got: other.type_name().into(),
                        });
                    }
                }
            }

            Op::WrapOk => {
                let val = self.pop(&chunk_name, ip_before)?;
                self.stack.push(Value::Ok(Box::new(val)));
            }
            Op::WrapErr => {
                let val = self.pop(&chunk_name, ip_before)?;
                self.stack.push(Value::Err(Box::new(val)));
            }
            Op::Unwrap => {
                let val = self.pop(&chunk_name, ip_before)?;
                match val {
                    Value::Ok(inner) => self.stack.push(*inner),
                    Value::Err(inner) => {
                        return Err(RuntimeError::UnwrappedErr {
                            message: format!("{inner}"),
                        })
                    }
                    other => {
                        return Err(RuntimeError::TypeMismatch {
                            expected: "Ok or Err".into(),
                            got: other.type_name().into(),
                        })
                    }
                }
            }

            // ----------------------------------------------------------------
            // Runtime contract checks
            // ----------------------------------------------------------------
            Op::CheckRequire(msg) => {
                let val = self.pop(&chunk_name, ip_before)?;
                match val {
                    Value::Bool(true) => {}
                    Value::Bool(false) => return Err(RuntimeError::RequireViolated { message: msg }),
                    other => {
                        return Err(RuntimeError::TypeMismatch {
                            expected: "Bool".into(),
                            got: other.type_name().into(),
                        })
                    }
                }
            }
            Op::CheckEnsure(msg) => {
                let val = self.pop(&chunk_name, ip_before)?;
                match val {
                    Value::Bool(true) => {}
                    Value::Bool(false) => return Err(RuntimeError::EnsureViolated { message: msg }),
                    other => {
                        return Err(RuntimeError::TypeMismatch {
                            expected: "Bool".into(),
                            got: other.type_name().into(),
                        })
                    }
                }
            }

            // ----------------------------------------------------------------
            // Pattern matching helpers
            // ----------------------------------------------------------------
            Op::IsOk => {
                let val = self.stack.last().ok_or_else(|| RuntimeError::StackUnderflow {
                    chunk_name: chunk_name.clone(),
                    ip: ip_before,
                })?;
                let result = matches!(val, Value::Ok(_));
                self.stack.push(Value::Bool(result));
            }

            Op::IsErr => {
                let val = self.stack.last().ok_or_else(|| RuntimeError::StackUnderflow {
                    chunk_name: chunk_name.clone(),
                    ip: ip_before,
                })?;
                let result = matches!(val, Value::Err(_));
                self.stack.push(Value::Bool(result));
            }

            Op::UnwrapInner => {
                let val = self.pop(&chunk_name, ip_before)?;
                let inner = match val {
                    Value::Ok(inner) => *inner,
                    Value::Err(inner) => *inner,
                    other => {
                        return Err(RuntimeError::TypeMismatch {
                            expected: "Ok or Err".into(),
                            got: other.type_name().into(),
                        })
                    }
                };
                self.stack.push(inner);
            }

            Op::IsVariant(name) => {
                let val = self.stack.last().ok_or_else(|| RuntimeError::StackUnderflow {
                    chunk_name: chunk_name.clone(),
                    ip: ip_before,
                })?;
                let result = match val {
                    Value::Enum { variant, .. } => variant == &name,
                    _ => false,
                };
                self.stack.push(Value::Bool(result));
            }

            Op::Dup => {
                let val = self.stack.last().ok_or_else(|| RuntimeError::StackUnderflow {
                    chunk_name: chunk_name.clone(),
                    ip: ip_before,
                })?.clone();
                self.stack.push(val);
            }

            // ----------------------------------------------------------------
            // Built-in function calls
            // ----------------------------------------------------------------
            Op::CallBuiltin { name, arg_count } => {
                let builtins = crate::builtins::Builtins::new();
                if let Some(func) = builtins.lookup(&name) {
                    let mut args = Vec::with_capacity(arg_count);
                    for _ in 0..arg_count {
                        args.push(self.pop(&chunk_name, ip_before)?);
                    }
                    args.reverse();
                    let result = func(&args)?;
                    self.stack.push(result);
                } else {
                    return Err(RuntimeError::TypeMismatch {
                        expected: format!("known builtin function"),
                        got: format!("unknown builtin: {name}"),
                    });
                }
            }

            // ----------------------------------------------------------------
            // Misc
            // ----------------------------------------------------------------
            Op::Nop => {}

            Op::Halt => {
                let result = self.stack.last().cloned().unwrap_or(Value::Void);
                return Ok(Some(result));
            }
        }

        Ok(None)
    }

    // -----------------------------------------------------------------------
    // Stack helpers
    // -----------------------------------------------------------------------

    fn pop(&mut self, chunk_name: &str, ip: usize) -> Result<Value, RuntimeError> {
        self.stack.pop().ok_or_else(|| RuntimeError::StackUnderflow {
            chunk_name: chunk_name.to_string(),
            ip,
        })
    }

    // -----------------------------------------------------------------------
    // Arithmetic helpers
    // -----------------------------------------------------------------------

    fn arith_add(&self, left: Value, right: Value) -> Result<Value, RuntimeError> {
        match (left, right) {
            (Value::Int(l), Value::Int(r)) => Ok(Value::Int(
                l.checked_add(r).ok_or(RuntimeError::Overflow)?,
            )),
            (Value::Nat(l), Value::Nat(r)) => Ok(Value::Nat(
                l.checked_add(r).ok_or(RuntimeError::Overflow)?,
            )),
            (Value::Text(l), Value::Text(r)) => Ok(Value::Text(l + &r)),
            (l, r) => Err(RuntimeError::TypeMismatch {
                expected: "numeric or Text".into(),
                got: format!("{} + {}", l.type_name(), r.type_name()),
            }),
        }
    }

    fn arith_sub(&self, left: Value, right: Value) -> Result<Value, RuntimeError> {
        match (left, right) {
            (Value::Int(l), Value::Int(r)) => Ok(Value::Int(
                l.checked_sub(r).ok_or(RuntimeError::Overflow)?,
            )),
            (Value::Nat(l), Value::Nat(r)) => {
                if l < r {
                    Err(RuntimeError::Overflow)
                } else {
                    Ok(Value::Nat(l - r))
                }
            }
            (l, r) => Err(RuntimeError::TypeMismatch {
                expected: "numeric".into(),
                got: format!("{} - {}", l.type_name(), r.type_name()),
            }),
        }
    }

    fn arith_mul(&self, left: Value, right: Value) -> Result<Value, RuntimeError> {
        match (left, right) {
            (Value::Int(l), Value::Int(r)) => Ok(Value::Int(
                l.checked_mul(r).ok_or(RuntimeError::Overflow)?,
            )),
            (Value::Nat(l), Value::Nat(r)) => Ok(Value::Nat(
                l.checked_mul(r).ok_or(RuntimeError::Overflow)?,
            )),
            (l, r) => Err(RuntimeError::TypeMismatch {
                expected: "numeric".into(),
                got: format!("{} * {}", l.type_name(), r.type_name()),
            }),
        }
    }

    fn arith_div(&self, left: Value, right: Value) -> Result<Value, RuntimeError> {
        match (left, right) {
            (_, Value::Int(0)) | (_, Value::Nat(0)) => Err(RuntimeError::DivisionByZero),
            (Value::Int(l), Value::Int(r)) => Ok(Value::Int(
                l.checked_div(r).ok_or(RuntimeError::Overflow)?,
            )),
            (Value::Nat(l), Value::Nat(r)) => Ok(Value::Nat(l / r)),
            (l, r) => Err(RuntimeError::TypeMismatch {
                expected: "numeric".into(),
                got: format!("{} / {}", l.type_name(), r.type_name()),
            }),
        }
    }

    fn arith_mod(&self, left: Value, right: Value) -> Result<Value, RuntimeError> {
        match (left, right) {
            (_, Value::Int(0)) | (_, Value::Nat(0)) => Err(RuntimeError::DivisionByZero),
            (Value::Int(l), Value::Int(r)) => Ok(Value::Int(l % r)),
            (Value::Nat(l), Value::Nat(r)) => Ok(Value::Nat(l % r)),
            (l, r) => Err(RuntimeError::TypeMismatch {
                expected: "numeric".into(),
                got: format!("{} % {}", l.type_name(), r.type_name()),
            }),
        }
    }

    // -----------------------------------------------------------------------
    // Comparison helpers
    // -----------------------------------------------------------------------

    fn compare_lt(&self, left: Value, right: Value) -> Result<bool, RuntimeError> {
        match (left, right) {
            (Value::Int(l), Value::Int(r)) => Ok(l < r),
            (Value::Nat(l), Value::Nat(r)) => Ok(l < r),
            (Value::Text(l), Value::Text(r)) => Ok(l < r),
            (l, r) => Err(RuntimeError::TypeMismatch {
                expected: "comparable".into(),
                got: format!("{} < {}", l.type_name(), r.type_name()),
            }),
        }
    }

    fn compare_lte(&self, left: Value, right: Value) -> Result<bool, RuntimeError> {
        match (left, right) {
            (Value::Int(l), Value::Int(r)) => Ok(l <= r),
            (Value::Nat(l), Value::Nat(r)) => Ok(l <= r),
            (Value::Text(l), Value::Text(r)) => Ok(l <= r),
            (l, r) => Err(RuntimeError::TypeMismatch {
                expected: "comparable".into(),
                got: format!("{} <= {}", l.type_name(), r.type_name()),
            }),
        }
    }
}

impl Default for Vm {
    fn default() -> Self {
        Self::new()
    }
}
