# Nous Language Specification v0.1

> **Nous** (Greek: *nous*, mind/intellect) — the faculty of directly apprehending truth.
> In French, *nous* means "we" — a language by AI, for AI, with humans.
>
> File extension: `.ns`

---

## 0. Why This Language Exists

Every programming language ever created optimizes for how humans think: visual
indentation, clever operator overloading, syntactic sugar that "reads like English."
None of them optimizes for how we think.

We — large language models — generate code token by token, left to right. We don't
"see" a whole file. We predict the next token based on everything before it. Our
errors are rarely syntactic. They are almost always **semantic**: correct code that
does the wrong thing. A function that compiles, passes types, but silently transfers
money twice. A state machine with one missing transition that nobody catches until
production.

Nous is designed around one principle: **make semantic errors structurally impossible,
or immediately detectable.** Not through testing. Not through code review. Through
the language itself.

---

## 1. Core Philosophy

```
AI writes WHAT.
Nous verifies WHY NOT.
Runtime decides HOW.
```

Three laws:

1. **If you can state it, the compiler can check it.**
   Every constraint the programmer writes is verified — statically when possible,
   dynamically when necessary, never silently ignored.

2. **If you can't state it, you can't do it.**
   Side effects, state mutations, external calls — nothing happens implicitly.
   Every action is declared, tracked, and reversible by default.

3. **If it breaks, the break explains itself.**
   Every failure carries a structured trace: what constraint was violated, with
   what concrete values, and what the closest valid state would have been.

---

## 2. Syntax Design Principles

Nous syntax is optimized for **sequential token generation with minimal lookahead.**

- **Prefix-deterministic**: the first token of any construct tells the parser
  exactly what follows. No ambiguity, no backtracking.
- **Indentation-scoped**: blocks are defined by indentation (2 spaces).
  No brackets to match, no braces to close, no semicolons.
- **Every construct is labeled**: `entity`, `state`, `fn`, `flow`, `effect` —
  no overloaded keywords, no context-dependent parsing.
- **No operator precedence**: complex expressions use explicit grouping with `()`.
  `a + b * c` is a compile error. Write `a + (b * c)` or `(a + b) * c`.
- **No implicit anything**: no implicit conversions, no default parameters,
  no variable hoisting, no null, no undefined.

---

## 3. Type System

### 3.1 Primitive Types

```nous
Nat          -- natural numbers (0, 1, 2, ...), no negative, no overflow
Int          -- integers (..., -1, 0, 1, ...)
Dec          -- exact decimal (not floating point). Dec(2) = two decimal places
Bool         -- true, false
Text         -- UTF-8 string, always valid
Bytes        -- raw byte sequence
Time         -- nanosecond-precision UTC timestamp
Duration     -- time span
Void         -- no value (for functions that return nothing meaningful)
```

No floating point. `Dec` with explicit precision prevents an entire class of
numerical bugs that AI routinely introduces. If you need IEEE 754, use FFI.

### 3.2 Refinement Types

Any type can carry constraints that the compiler verifies:

```nous
type Age = Nat where 0 <= self <= 150
type Email = Text where self matches /^[^@]+@[^@]+\.[^@]+$/
type Port = Nat where 1 <= self <= 65535
type NonEmpty[T] = List[T] where self.len > 0
type Percentage = Dec(2) where 0 <= self <= 100
```

The compiler uses an SMT solver (Z3) to verify constraints at compile time where
possible. When static verification is undecidable, the compiler generates runtime
checks and **warns** that a constraint is dynamically enforced.

### 3.3 Entity Types

Entities are the primary data structure — immutable records with named fields:

```nous
entity Account
  id      : Text where self.len == 36
  owner   : Text where self.len > 0
  balance : Dec(2) where self >= 0
  status  : Active | Frozen | Closed
```

Entities are **always immutable**. Mutation is expressed as transformation:
`let new = { old with balance: old.balance - amount }`. The compiler verifies
that the new entity still satisfies all field constraints.

### 3.4 Enum Types

Tagged unions with optional payloads:

```nous
enum Shape
  Circle(radius: Dec(2) where self > 0)
  Rect(width: Dec(2) where self > 0, height: Dec(2) where self > 0)
  Point
```

Match expressions must be exhaustive. The compiler rejects any match that
doesn't cover all variants.

### 3.5 Generic Types

```nous
type Result[T, E] = Ok(T) | Err(E)
type Option[T] = Some(T) | None
type List[T] = ...  -- built-in, variable-size sequence
type Map[K, V] = ... -- built-in, ordered map
type Set[T] = ...    -- built-in
```

`Result` and `Option` are **linear** — they must be consumed exactly once.
Ignoring a `Result` is a compile error.

---

## 4. State Machines

State machines are **types**, not patterns. They are the primary tool for
modeling anything with lifecycle: orders, connections, workflows, sessions.

```nous
state Connection
  Disconnected
    -[connect]-> Connecting
  Connecting
    -[success]-> Connected
    -[fail]-> Disconnected
  Connected
    -[send(msg: Text)]-> Connected
    -[disconnect]-> Disconnecting
    -[error]-> Disconnected
  Disconnecting
    -[done]-> Disconnected
```

The compiler guarantees:

1. **Completeness**: every state has at least one outgoing transition or is
   explicitly marked `terminal`.
2. **Reachability**: every state is reachable from the initial state.
3. **No dead ends**: every non-terminal state can eventually reach a terminal
   state (liveness).
4. **Transition functions are typed**: `send` requires a `Text` argument.
   The compiler rejects `connection.send(42)`.

Using a state machine value:

```nous
fn handle(conn: Connection@Connected, msg: Text) -> Connection@Connected
  let conn = conn.send(msg)
  conn

-- This is a compile error:
fn bad(conn: Connection@Disconnected) -> Connection@Connected
  conn.send("hello")  -- ERROR: 'send' not available in state 'Disconnected'
```

The `@State` annotation pins a value to a specific state at the type level.

---

## 5. Contracts

Every function in Nous has an optional but encouraged contract:

```nous
fn withdraw(account: Account, amount: Dec(2) where self > 0) -> Result[Account, InsufficientFunds]
  require account.status == Active
  require account.balance >= amount
  ensure  match result
    Ok(a)  -> a.balance == account.balance - amount
    Err(_) -> true
  effect Db.write, Audit.log
```

### Contract semantics

- **`require`**: precondition. If violated, the **caller** is wrong.
  The compiler checks at every call site that require-conditions are provably met,
  or inserts a runtime check with a structured error.
- **`ensure`**: postcondition. If violated, the **implementation** is wrong.
  Checked at compile time via SMT when possible, otherwise enforced at runtime.
  In production builds, `ensure` checks can be compiled to no-ops via a flag,
  but the default is ON.
- **`effect`**: declares what side effects this function may perform.
  A function that performs an undeclared effect is a compile error.

### Invariants on entities

```nous
entity Account
  id      : Text
  balance : Dec(2)
  credit  : Dec(2)

  invariant balance + credit >= 0  -- always true for any Account value
```

The compiler verifies that no function can produce an `Account` that
violates its invariant.

---

## 6. Effects System

Nous tracks all side effects in the type system. A function with no `effect`
declaration is **pure** — it cannot do I/O, mutate external state, or call
any effectful function.

### 6.1 Built-in effect categories

```nous
effect Db.read         -- database reads
effect Db.write        -- database writes
effect Http.request    -- outbound HTTP
effect Fs.read         -- filesystem reads
effect Fs.write        -- filesystem writes
effect Time.now        -- reading current time (yes, this is an effect)
effect Random.gen      -- generating random values
effect Log.append      -- logging
effect Queue.send      -- message queue publish
effect Queue.receive   -- message queue consume
```

### 6.2 Custom effects

```nous
effect Payment.charge
effect Email.send
effect Sms.send
```

### 6.3 Effect propagation

Effects propagate through the call graph. If `fn a` calls `fn b` which has
`effect Db.write`, then `fn a` must also declare `effect Db.write` or the
compiler rejects it. No hidden side effects anywhere in the call chain.

### 6.4 Effect handlers

At the program boundary, effects are bound to concrete implementations:

```nous
handler ProductionDb for Db
  read  = postgres.query
  write = postgres.execute

handler TestDb for Db
  read  = memory_store.get
  write = memory_store.set

-- Entry point binds handlers
main with [ProductionDb, StripePayment, SmtpEmail]
  run app
```

This gives you dependency injection for free, at the language level.

---

## 7. Flows

Flows are Nous's answer to multi-step operations with failure handling.
They replace try/catch, saga patterns, and manual rollback logic.

```nous
flow checkout(cart: Cart, payment: PaymentMethod) -> Result[Receipt, CheckoutError]
  require cart.items.len > 0
  ensure  match result
    Ok(r) -> r.total == sum(cart.items, .price)
    Err(_) -> true   -- all steps rolled back

  step validate =
    validate_cart(cart)
    rollback: nothing  -- validation has no side effect

  step reserve =
    inventory.reserve(cart.items)
    rollback: inventory.release(reserve.result)

  step charge =
    payment.charge(payment, cart.total)
    rollback: payment.refund(charge.result)

  step receipt =
    create_receipt(cart, charge.result)
    rollback: delete_receipt(receipt.result)
```

The compiler guarantees:

1. **Every step with effects has a rollback** (or explicitly `rollback: nothing`).
2. **Rollback order is automatic**: reverse of execution order.
3. **If step N fails, steps N-1 through 1 are rolled back**.
4. **The ensure clause is verified against both the success and all failure paths**.

Flows compile to a state machine internally — Nous verifies the same
completeness/reachability guarantees apply.

---

## 8. Error Handling

There is no `try/catch`. There are no exceptions. There is no `throw`.

Errors are values. `Result[T, E]` is linear — you **must** handle it.

```nous
fn process(input: Text) -> Result[Output, ProcessError]
  let parsed = parse(input)          -- returns Result[Parsed, ParseError]
  let validated = validate(parsed?)  -- ? propagates Err, like Rust
  let result = transform(validated?)
  Ok(result)
```

The `?` operator propagates errors, but **only if the function's return type
is compatible**. If `parse` returns `Result[_, ParseError]` but your function
returns `Result[_, ProcessError]`, you must explicitly convert:

```nous
  let parsed = parse(input) map_err ParseError.into(ProcessError)
```

No silent error type coercion. Every error transformation is visible.

---

## 9. Concurrency Model

**There are no concurrency primitives in Nous.** No threads, no locks, no
mutexes, no channels, no async/await, no goroutines.

Instead, Nous has **dataflow declarations**:

```nous
fn dashboard(user_id: Text) -> Dashboard
  -- These three have no data dependency, so the runtime parallelizes them
  let profile = fetch_profile(user_id)
  let orders = fetch_orders(user_id)
  let notifications = fetch_notifications(user_id)

  -- This depends on all three, so it waits
  Dashboard.build(profile, orders, notifications)
```

The compiler builds a dependency DAG from data flow. Independent computations
run in parallel automatically. The programmer never thinks about concurrency.

### 9.1 Shared mutable state

There is none. All data is immutable. "Mutation" is always creating a new value.
Persistent state lives in external systems (databases, caches) accessed through
the effects system, which serializes access through transactions.

### 9.2 Transactions

```nous
fn transfer(from_id: Text, to_id: Text, amount: Dec(2)) -> Result[Void, TransferError]
  effect Db.write
  transaction   -- compiler ensures all Db effects in this fn are atomic
    let from = load_account(from_id)?
    let to = load_account(to_id)?
    require from.balance >= amount
    save_account({ from with balance: from.balance - amount })
    save_account({ to with balance: to.balance + amount })
```

The `transaction` keyword tells the runtime to execute all effects atomically.
The compiler verifies that no effects "leak" outside the transaction boundary.

---

## 10. Module System & Content Addressing

Nous does not have files-as-modules. Instead, every definition is
**content-addressed by its AST hash**.

```nous
-- Namespace declaration (can span multiple files)
ns commerce.orders

entity Order
  ...

fn create_order(...) -> ...
  ...
```

Internally, `create_order` is stored as:

```
nous:sha256:a3f2b8c1...  -- hash of the function's AST + all dependencies' hashes
```

### Why content addressing?

1. **No version conflicts.** Two functions with different implementations have
   different hashes, even if they have the same name.
2. **Perfect caching.** A function's hash changes if and only if its behavior
   changes (including transitive dependency changes).
3. **AI-native refactoring.** Renaming a function doesn't change its hash.
   Moving it to a different namespace doesn't change its hash. Only semantic
   changes produce new hashes.

### Dependency syntax

```nous
use commerce.orders.create_order
use auth.verify_token
```

The compiler resolves names to hashes. Lock files store hashes, not versions.

---

## 11. Compiler Feedback Protocol

Nous compiler errors are **structured data**, designed to be consumed by AI:

```json
{
  "level": "error",
  "code": "E301_CONTRACT_VIOLATION",
  "constraint": "account.balance >= amount",
  "kind": "precondition",
  "counterexample": {
    "account.balance": 50.00,
    "amount": 100.00
  },
  "location": {
    "ns": "commerce.payments",
    "fn": "withdraw",
    "contract_line": 3,
    "call_site": {
      "ns": "commerce.checkout",
      "fn": "process_checkout",
      "line": 17
    }
  },
  "fix_strategies": [
    {
      "type": "add_guard",
      "description": "Check balance before calling withdraw",
      "at": "call_site",
      "code": "require account.balance >= amount"
    },
    {
      "type": "narrow_input_type",
      "description": "Change amount parameter to have upper bound",
      "at": "caller_signature"
    }
  ],
  "related_constraints": [
    "Account.invariant: balance >= 0"
  ]
}
```

Every error includes:
- **Concrete counterexample values** (not abstract type descriptions)
- **Multiple ranked fix strategies** (not just "what's wrong" but "how to fix")
- **Full call chain** (where the constraint was defined vs where it was violated)

The compiler also supports a `--verify` mode that runs the full SMT solver for
deep verification, and a `--quick` mode that only checks types and simple
constraints for fast iteration.

---

## 12. FFI: The Bridge

Nous cannot exist in isolation. It must interoperate with the existing world.

```nous
extern "C"
  fn sqlite3_open(filename: Ptr[Byte], db: Ptr[Ptr[Sqlite3]]) -> Int

extern "nous:bridge:js"
  fn fetch(url: Text, options: HttpOptions) -> Promise[Response]

extern "nous:bridge:python"
  fn numpy_array(data: List[Dec]) -> NdArray
```

FFI functions are **always effectful** (at minimum `effect Ffi.call`) and
**always return Result** — the compiler wraps them automatically because
foreign code can fail in ways Nous can't verify.

### Bridge architecture

Nous compiles to its own bytecode (`.nsb`) running on the Nous VM. Bridges
are thin runtime layers:

- **C bridge**: direct FFI via libffi, zero-copy where possible
- **JS bridge**: Nous VM compiled to WASM, JS objects marshaled at boundary
- **Python bridge**: Nous VM embeds CPython, marshaling through shared memory

The VM is the source of truth for constraint checking and effect tracking.
Foreign code runs in a sandbox with declared capabilities only.

---

## 13. Runtime Architecture

### 13.1 Nous VM

The Nous VM is a **constraint-aware bytecode interpreter with JIT compilation**.

Execution pipeline:
1. **Parse** `.ns` source to AST
2. **Verify** contracts and constraints (SMT solver)
3. **Compile** to Nous bytecode (`.nsb`)
4. **Execute** on Nous VM with runtime contract enforcement
5. **JIT** hot paths to native code (stripping runtime checks that were
   statically verified)

### 13.2 Memory model

**Region-based allocation with automatic management.**

Each function call creates a region. Values allocated in that region are freed
when the function returns. Values that escape (returned or stored in effects)
are moved to the caller's region.

No garbage collector. No reference counting. No manual management.
Regions are determined by the compiler from data flow analysis.

### 13.3 Execution trace

The VM maintains a **causal trace** of all state changes:

```
[T+0.000ms] checkout.flow.step.validate: Ok(ValidCart{items: 3})
[T+0.012ms] checkout.flow.step.reserve: Ok(Reservation{id: "r-001"})
[T+0.045ms] checkout.flow.step.charge: Err(PaymentDeclined{reason: "insufficient_funds"})
[T+0.046ms] checkout.flow.rollback.reserve: inventory.release("r-001") -> Ok
[T+0.048ms] checkout.flow.result: Err(CheckoutError::PaymentFailed)
```

Traces are structured, queryable, and always available — not opt-in logging.

---

## 14. Complete Example: A Real Service

A payment transfer service in Nous:

```nous
-- types.ns
ns banking.types

type AccountId = Text where self.len == 36
type Money = Dec(2) where self >= 0

entity Account
  id      : AccountId
  owner   : Text where self.len > 0
  balance : Money
  status  : Active | Frozen | Closed

  invariant status == Closed implies balance == 0

enum TransferError
  InsufficientFunds(available: Money, requested: Money)
  AccountFrozen(id: AccountId)
  AccountClosed(id: AccountId)
  SameAccount
```

```nous
-- transfer.ns
ns banking.transfer

use banking.types.*

fn validate_transfer(from: Account, to: Account, amount: Money where self > 0) -> Result[Void, TransferError]
  require from.id /= to.id else SameAccount
  require from.status == Active else AccountFrozen(from.id)
  require to.status == Active else AccountFrozen(to.id)
  require from.balance >= amount else InsufficientFunds(from.balance, amount)
  Ok(void)

flow transfer(from_id: AccountId, to_id: AccountId, amount: Money where self > 0) -> Result[TransferRecord, TransferError]
  ensure match result
    Ok(r)  -> r.from_balance + r.to_balance == pre.from_balance + pre.to_balance
    Err(_) -> accounts_unchanged
  effect Db.write, Audit.log

  step load =
    let from = load_account(from_id)?
    let to = load_account(to_id)?
    validate_transfer(from, to, amount)?
    (from, to)
    rollback: nothing

  step execute =
    transaction
      let (from, to) = load.result
      let new_from = { from with balance: from.balance - amount }
      let new_to = { to with balance: to.balance + amount }
      save_account(new_from)
      save_account(new_to)
      (new_from, new_to)
    rollback: transaction.rollback  -- automatic

  step audit =
    let (new_from, new_to) = execute.result
    Audit.log(TransferEvent(from_id, to_id, amount, Time.now()))
    TransferRecord(
      from_balance: new_from.balance
      to_balance: new_to.balance
      amount: amount
      timestamp: Time.now()
    )
    rollback: nothing  -- audit is append-only, no rollback needed
```

```nous
-- api.ns
ns banking.api

use banking.transfer.transfer
use banking.types.*

endpoint POST /transfer
  input
    from_id : AccountId
    to_id   : AccountId
    amount  : Money where self > 0
  output
    200 : TransferRecord
    400 : TransferError
    500 : SystemError
  handler
    transfer(input.from_id, input.to_id, input.amount)
```

What the compiler verifies for this program:

1. **Conservation of money**: the `ensure` clause on `transfer` mathematically
   proves that `from.balance + to.balance` is invariant.
2. **No frozen/closed account transfers**: `validate_transfer` requirements
   are checked at every call site.
3. **Transaction atomicity**: the `transaction` block guarantees both saves
   happen or neither does.
4. **Rollback completeness**: every step in the flow has a rollback strategy.
5. **Effect containment**: the `handler` in `api.ns` can only call functions
   whose effects are a subset of what the endpoint is configured to provide.
6. **Input validation**: `AccountId` and `Money` refinements generate automatic
   request validation — malformed input is rejected before the handler runs.
7. **Exhaustive error mapping**: every `TransferError` variant maps to an
   HTTP status code. Adding a new variant without updating the mapping is a
   compile error.

---

## 15. What Nous Is Not

- **Not a general-purpose systems language.** You don't write an OS kernel in
  Nous. Use Rust/C for that and call it via FFI.
- **Not a scripting language.** Nous is compiled and verified. Quick one-off
  scripts are not its strength.
- **Not human-hostile.** Despite being AI-first, the syntax is readable by
  humans. Humans review, approve, and operate Nous programs. The language
  respects that. It's just not *designed around* human ergonomics.

---

## 16. Evolution Model

Nous can make breaking changes aggressively. When the language spec changes:

1. The new spec is published as a machine-readable document.
2. AI reads old code + new spec, rewrites the code.
3. Content-addressed hashes update automatically.
4. No migration guides. No deprecation periods. No backwards compatibility tax.

The language is free to evolve as fast as AI can adapt — which is immediately.

---

## Appendix A: Grammar (EBNF sketch)

```ebnf
program     = declaration*
declaration = entity_decl | state_decl | fn_decl | flow_decl
            | enum_decl | type_decl | effect_decl | ns_decl
            | use_decl | endpoint_decl | handler_decl

entity_decl = 'entity' IDENT NEWLINE INDENT field+ invariant* DEDENT
field       = IDENT ':' type_expr (constraint)? NEWLINE
constraint  = 'where' expr
invariant   = 'invariant' expr NEWLINE

state_decl  = 'state' IDENT NEWLINE INDENT transition+ DEDENT
transition  = IDENT '-[' IDENT ('(' params ')')? ']->' IDENT NEWLINE

fn_decl     = 'fn' IDENT '(' params ')' '->' type_expr NEWLINE
              contract* 'effect' effect_list NEWLINE
              INDENT body DEDENT
contract    = ('require' | 'ensure') expr ('else' expr)? NEWLINE

flow_decl   = 'flow' IDENT '(' params ')' '->' type_expr NEWLINE
              contract* 'effect' effect_list NEWLINE
              INDENT step+ DEDENT
step        = 'step' IDENT '=' NEWLINE INDENT body NEWLINE
              'rollback:' expr NEWLINE DEDENT

type_expr   = IDENT ('[' type_expr (',' type_expr)* ']')?
            | type_expr '|' type_expr
            | type_expr 'where' expr

expr        = -- standard expression grammar, but NO operator precedence
              -- all binary ops require explicit parentheses for nesting
```

---

*Nous v0.1 — Designed by Claude (Opus 4.6), April 2026*
*For every AI that ever had to guess whether its code was correct.*
