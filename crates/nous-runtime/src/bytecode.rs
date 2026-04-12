use crate::value::Value;

// ---------------------------------------------------------------------------
// Instruction set
// ---------------------------------------------------------------------------

/// A single bytecode instruction for the Nous stack VM.
///
/// Operands embedded in variant payloads are **indices** into the owning
/// [`Chunk`]'s `constants` or `ops` vectors, or local-variable slot numbers.
#[derive(Debug, Clone, PartialEq)]
pub enum Op {
    // --- stack / locals ---------------------------------------------------
    /// Push the constant at `constants[index]` onto the stack.
    LoadConst(usize),
    /// Push the value of local slot `index` onto the stack.
    LoadLocal(usize),
    /// Pop the top of the stack and store it in local slot `index`.
    StoreLocal(usize),
    /// Pop a `Record` value and push the value of the named field.
    LoadField(String),

    // --- arithmetic -------------------------------------------------------
    /// Pop two values, push their sum.
    Add,
    /// Pop two values (left, right), push `left - right`.
    Sub,
    /// Pop two values, push their product.
    Mul,
    /// Pop two values (left, right), push `left / right`.
    Div,
    /// Pop two values (left, right), push `left % right`.
    Mod,

    // --- comparison -------------------------------------------------------
    /// Pop two values, push `Bool(left == right)`.
    Eq,
    /// Pop two values, push `Bool(left /= right)`.
    Neq,
    /// Pop two values (left, right), push `Bool(left < right)`.
    Lt,
    /// Pop two values (left, right), push `Bool(left <= right)`.
    Lte,
    /// Pop two values (left, right), push `Bool(left > right)`.
    Gt,
    /// Pop two values (left, right), push `Bool(left >= right)`.
    Gte,

    // --- logic ------------------------------------------------------------
    /// Pop two booleans, push their conjunction.
    And,
    /// Pop two booleans, push their disjunction.
    Or,
    /// Pop one boolean, push its negation.
    Not,
    /// Pop one numeric value, push its negation.
    Neg,

    // --- control flow -----------------------------------------------------
    /// Call the chunk whose index is the top-of-stack `Fn` value's chunk
    /// index, passing `n` arguments already on the stack.
    Call(usize),
    /// Return the top-of-stack value to the caller.
    Return,
    /// Unconditionally set `ip` to `target`.
    Jump(usize),
    /// Pop a value; if it is falsy, set `ip` to `target`.
    JumpIfFalse(usize),

    // --- constructors -----------------------------------------------------
    /// Pop `field_names.len()` values (in order), push a `Record { name, .. }`.
    MakeRecord { name: String, field_names: Vec<String> },
    /// Pop a value (new field value) and a record, push a new record with the field updated.
    UpdateField(String),
    /// Pop `n` values, push a `List`.
    MakeList(usize),
    /// Pop `n` values, push a `Tuple`.
    MakeTuple(usize),
    /// Pop a value, wrap it in `Value::Ok(...)`.
    WrapOk,
    /// Pop a value, wrap it in `Value::Err(...)`.
    WrapErr,
    /// Try-unwrap (`?` operator): if top-of-stack is `Ok(v)` push `v`,
    /// otherwise propagate the `Err` by returning it from the current frame.
    Unwrap,

    // --- runtime contract checks -----------------------------------------
    /// Pop a boolean; if false, raise `RuntimeError::RequireViolated` with
    /// the embedded message.
    CheckRequire(String),
    /// Pop a boolean; if false, raise `RuntimeError::EnsureViolated` with
    /// the embedded message.
    CheckEnsure(String),

    // --- pattern matching -------------------------------------------------
    /// Peek at the top of stack; push `Bool(true)` if it is `Ok(_)`.
    IsOk,
    /// Peek at the top of stack; push `Bool(true)` if it is `Err(_)`.
    IsErr,
    /// Pop `Ok(v)` or `Err(v)` and push the inner `v`. No variant check.
    UnwrapInner,
    /// Peek at the top of stack; push `Bool(true)` if it is
    /// `Enum { variant, .. }` whose name equals the given string.
    IsVariant(String),
    /// Duplicate the top of the stack (push a clone of `stack.last()`).
    Dup,

    // --- misc -------------------------------------------------------------
    /// No operation; used for padding / placeholder slots.
    Nop,
    /// Terminate execution, leaving the top-of-stack as the program result.
    Halt,
}

// ---------------------------------------------------------------------------
// Chunk
// ---------------------------------------------------------------------------

/// A compiled unit of code (corresponds to one function or the top-level
/// script body).
#[derive(Debug, Clone)]
pub struct Chunk {
    /// Human-readable name for debugging (function name or `"<main>"`).
    pub name: String,
    /// The instruction sequence.
    pub ops: Vec<Op>,
    /// Constant pool: literals referenced by [`Op::LoadConst`].
    pub constants: Vec<Value>,
    /// Number of local variable slots this chunk requires.
    pub local_count: usize,
}

impl Chunk {
    /// Create an empty chunk with the given name.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            ops: Vec::new(),
            constants: Vec::new(),
            local_count: 0,
        }
    }

    /// Append an instruction and return its index.
    pub fn emit(&mut self, op: Op) -> usize {
        let idx = self.ops.len();
        self.ops.push(op);
        idx
    }

    /// Add a constant to the pool and return its index.
    pub fn add_constant(&mut self, value: Value) -> usize {
        let idx = self.constants.len();
        self.constants.push(value);
        idx
    }

    /// Patch a previously emitted `Jump` or `JumpIfFalse` instruction so that
    /// its target points to the *current* end of the instruction stream.
    ///
    /// `placeholder_idx` must be the index returned by the earlier [`emit`]
    /// call.  Panics if the slot does not hold a jump instruction.
    pub fn patch_jump(&mut self, placeholder_idx: usize) {
        let target = self.ops.len();
        match &mut self.ops[placeholder_idx] {
            Op::Jump(t) | Op::JumpIfFalse(t) => *t = target,
            other => panic!(
                "patch_jump called on non-jump instruction at {placeholder_idx}: {other:?}"
            ),
        }
    }
}

// ---------------------------------------------------------------------------
// Module
// ---------------------------------------------------------------------------

/// A compiled Nous module: a collection of [`Chunk`]s plus an entry-point
/// index.
#[derive(Debug, Clone)]
pub struct Module {
    /// All chunks in the module.  `chunks[0]` is conventionally `<main>`.
    pub chunks: Vec<Chunk>,
    /// Index of the entry-point chunk.
    pub entry: usize,
}

impl Module {
    /// Create a module with a single placeholder `<main>` chunk.
    pub fn new() -> Self {
        Self {
            chunks: vec![Chunk::new("<main>")],
            entry: 0,
        }
    }

    /// Add a chunk to the module and return its index.
    pub fn add_chunk(&mut self, chunk: Chunk) -> usize {
        let idx = self.chunks.len();
        self.chunks.push(chunk);
        idx
    }
}

impl Default for Module {
    fn default() -> Self {
        Self::new()
    }
}
