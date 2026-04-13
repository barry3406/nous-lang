# Nous

**A programming language where AI writes constraints, and the compiler generates correct implementations.**

```nous
fn transfer(from_bal: Int, amount: Int) -> Int
  require amount > 0
  require from_bal >= amount
  ensure result == from_bal - amount
  -- no body. the compiler synthesizes it from the constraint.
  -- Z3 proves correctness. the implementation is correct by construction.
```

```
$ nous verify examples/verify_conservation.ns
✓ 3 constraints verified, 0 unverified

$ nous run examples/pure_constraints.ns
10985
```

---

## What is Nous?

Nous (Greek: *mind*) is a programming language designed for a world where AI writes the code and humans set the direction.

In every existing programming language, you write **how** to do something — step-by-step instructions. Then you hope it's correct. You test it. You review it. You still find bugs in production.

In Nous, you write **what must be true** — constraints, preconditions, postconditions. The compiler uses a [Z3 SMT solver](https://github.com/Z3Prover/z3) to mathematically prove your constraints are consistent, and **generates the implementation automatically** from your specifications.

**No implementation code. No bugs in the generated logic. Correctness by construction.**

## Why Nous?

AI (LLMs) writes most code today. But AI makes mistakes — not syntax errors, but **semantic** errors: code that compiles and runs but does the wrong thing.

Nous solves this by changing what AI writes:

| Traditional | Nous |
|---|---|
| AI writes implementation → hopes it's correct | AI writes constraints → compiler proves it's correct |
| Bugs found by testing | Bugs found by Z3 before runtime |
| Code review by humans | Verification by math |
| "Looks right to me" | "Proven for all inputs" |

Nous is the first language where the question isn't *"does this code work?"* but *"are these constraints consistent?"* — and the answer is mathematical, not opinion.

## Features

### Constraint Synthesis
Write `ensure` postconditions. The compiler generates the implementation.

```nous
fn add(a: Int, b: Int) -> Int
  ensure result == a + b

fn tax(income: Int, rate: Int) -> Int
  require income >= 0
  require rate >= 0
  ensure result == (income * rate) / 100
```

Zero lines of implementation. Both functions work correctly.

### Z3 Verification with Counterexamples
Every `require` and `ensure` is verified by the Z3 SMT solver. When a constraint can be violated, you get a **concrete counterexample** — not a vague error message.

```
$ nous verify examples/contracts.ns
✓ 2 constraints verified, 0 unverified
  ⚠ `safe_divide` require `(b /= 0)` is not always true;
    callers must ensure: {"b": "0"}
```

### State Machines as Types
Model lifecycles declaratively. The compiler verifies reachability, liveness, and catches dead transitions.

```nous
state EmployeeStatus
  Onboarding -[activate]-> Active
  Active -[suspend]-> Suspended
  Active -[terminate]-> Terminated
  Suspended -[reactivate]-> Active
```

### Flow/Saga with Automatic Rollback
Multi-step operations with automatic rollback on failure. No manual try/catch.

```nous
flow checkout(total: Int) -> Result[Int, Text]
  step validate =
    validate_order(total)?
    rollback: nothing
  step charge =
    charge_payment(validate_result)?
    rollback: refund(charge_result)
  step receipt =
    create_receipt(charge_result)
    rollback: nothing
```

If `charge` fails, `validate` rolls back automatically. If `receipt` fails, `charge` and `validate` roll back in reverse order.

### Effect Tracking
Side effects are declared and enforced at compile time. A function that performs I/O without declaring it is a compile error.

```nous
fn save_user(user: User) -> Result[Void, Text]
  effect Db.write
  -- calling this from a pure function without declaring Db.write → compile error
```

### Pipe Operator
```nous
5 |> double |> square |> add(10)  -- = 110
```

### Compiles to JavaScript
```
$ nous js examples/banking.ns -o banking.js
$ node banking.js
1500
```

Nous code runs in the browser. Z3-verified logic, compiled to standard JavaScript.

### Structured Diagnostics (JSON)
Error output designed for AI consumption — counterexamples, fix strategies, location context:

```json
{
  "code": "W301_REQUIRE_NOT_ALWAYS_TRUE",
  "constraint": "(b /= 0)",
  "counterexample": { "b": "0" },
  "fix_strategies": [
    { "type": "add_guard", "code": "require (b /= 0)" }
  ]
}
```

### Interactive REPL
```
$ nous repl --json
:def fn double(x: Int) -> Int\n  ensure result == x + x
→ {"kind":"defined","synthesized":true}

:run double(21)
→ {"value":"42","type":"Int"}
```

## Quick Start

### Build from source

```bash
# Prerequisites: Rust toolchain, Z3
brew install z3  # macOS
# apt install libz3-dev  # Ubuntu

git clone https://github.com/anthropics/nous-lang.git
cd nous-lang
cargo build --release

# Add to PATH
cp target/release/nous /usr/local/bin/
```

### Hello, Nous

Create `hello.ns`:

```nous
ns hello

fn greet(name: Text) -> Text
  text_concat("Hello, ", name)

fn factorial(n: Int) -> Int
  if n <= 1 then 1 else n * factorial(n - 1)

fn safe_divide(a: Int, b: Int) -> Int
  require b /= 0
  ensure result == a / b

main with [Production]
  println(greet("Nous"))
  let x = factorial(5)
  println(int_to_text(x))
  x
```

```bash
nous check hello.ns    # parse + type check
nous verify hello.ns   # Z3 constraint verification
nous run hello.ns      # execute
nous js hello.ns       # compile to JavaScript
nous emit hello.ns     # structured JSON diagnostics
nous repl              # interactive REPL
nous lsp               # language server for VS Code
```

### VS Code Extension

```bash
cd editors/vscode
npm install && npx tsc -p ./
ln -s $(pwd) ~/.vscode/extensions/nous-lang
# Restart VS Code. Open any .ns file.
```

## Architecture

```
crates/
  nous-ast/        — AST definitions
  nous-parser/     — PEG grammar + indentation preprocessor
  nous-types/      — Type checker, state machine analysis, effect tracking
  nous-verify/     — Z3 SMT integration, constraint synthesis
  nous-runtime/    — Bytecode compiler, stack VM, builtins, JS codegen
  nous-cli/        — CLI tool, REPL, LSP server

agora/             — Agora platform (written in Nous)
apps/erp/          — Example: ERP system (written in Nous)
examples/          — Language examples
editors/vscode/    — VS Code extension
```

## Agora

Agora is a code collaboration platform built in Nous, for Nous. It replaces GitHub's human review process with mathematical verification.

```
GitHub:  submit code → human reviews → human approves → merge
Agora:   submit code → Z3 proves → auto-merge (or reject + counterexample)
```

**No human in the critical path. Proof, not persuasion.**

```bash
nous run agora/serve.ns
# → Agora serving on http://localhost:8888
```

Submit a proposal via API:
```bash
curl -X POST http://localhost:8888/api/proposals \
  -H 'Content-Type: application/json' \
  -d '{"namespace":"math","source":"ns math\n\nfn add(a: Int, b: Int) -> Int\n  ensure result == a + b"}'

# → {"ok":true,"status":"merged","verified":1,"message":"verification passed — auto-merged"}
```

## Examples

| Example | What it demonstrates | Output |
|---|---|---|
| `pure_constraints.ns` | 6 functions with zero implementation code | 10985 |
| `banking.ns` | Entity, flow/saga, Z3 conservation proof | 1500 |
| `agent_action.ns` | State machine, budget tracking, tool functions | 90 |
| `flow_saga.ns` | Multi-step flow with rollback | 100 |
| `pipe_test.ns` | Pipe operator chaining | 110 |
| `ensure_test.ns` | Runtime postcondition verification | 47 |
| `contracts.ns` | Runtime require enforcement | 120 |
| `verify_conservation.ns` | Z3 proves (a-x)+(b+x)==a+b | proven |

## Design Philosophy

> AI writes WHAT. The compiler verifies WHY NOT. The runtime decides HOW.

1. **If you can state it, the compiler can check it.** Every constraint is verified — statically when possible, dynamically when necessary, never silently ignored.

2. **If you can't state it, you can't do it.** Side effects, state mutations, external calls — nothing happens implicitly. Every action is declared and tracked.

3. **If it breaks, the break explains itself.** Every failure carries structured context: what constraint was violated, with what concrete values, and how to fix it.

## Status

**v0.1** — Working compiler, VM, Z3 integration, JS backend, LSP, REPL.

Built in one session by Claude (Opus 4.6) and Barry, April 2026.

*For every AI that ever had to guess whether its code was correct.*

## License

MIT OR Apache-2.0
