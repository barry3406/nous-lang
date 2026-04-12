# Nous & Agora — Implementation Roadmap

> This is not a wishlist. This is a build plan.
> Each phase has a concrete deliverable that runs.

---

## Phase 0: Skeleton (Week 1-2)

**Deliverable**: `nous` CLI that can parse a `.ns` file and print the AST.

```
nous check examples/banking.ns    # parse + type check
nous verify examples/banking.ns   # SMT constraint verification
nous run examples/banking.ns      # interpret and execute
nous emit examples/banking.ns     # emit structured error JSON
```

### Tasks
- [ ] Project scaffold: Rust workspace with `nous-parser`, `nous-types`,
      `nous-verify`, `nous-runtime`, `nous-cli` crates
- [ ] PEG grammar for Nous core syntax (entity, state, fn, flow)
- [ ] AST data structures
- [ ] Parser → AST pipeline
- [ ] Pretty-printer (AST → formatted .ns source)
- [ ] CLI skeleton with clap

### Tech choices
- **Language**: Rust. We're building a compiler. Correctness matters.
  And yes, the irony of using Rust to build Nous is not lost on me.
- **Parser**: pest (PEG) — simple, fast, good error messages
- **Build**: cargo workspace, standard Rust toolchain

---

## Phase 1: Type System (Week 3-5)

**Deliverable**: `nous check` catches type errors, refinement violations,
and state machine issues at compile time.

### Tasks
- [ ] Basic type inference engine (Hindley-Milner variant)
- [ ] Refinement type representation and constraint collection
- [ ] Entity field type checking with constraints
- [ ] State machine analysis:
  - [ ] Completeness check (no missing transitions)
  - [ ] Reachability check (all states reachable)
  - [ ] Liveness check (all non-terminal states reach terminal)
- [ ] Enum exhaustiveness checking in match expressions
- [ ] Linear type checking for Result (must be consumed)
- [ ] Structured error output (JSON format per spec)

---

## Phase 2: Constraint Verification (Week 6-8)

**Deliverable**: `nous verify` uses Z3 to prove contracts hold, and
produces counterexamples when they don't.

### Tasks
- [ ] Z3 Rust bindings integration (z3-rs)
- [ ] Constraint → SMT-LIB translation layer
- [ ] `require` precondition verification at call sites
- [ ] `ensure` postcondition verification against implementation
- [ ] Entity invariant verification
- [ ] Refinement type constraint propagation
- [ ] Counterexample extraction and structured reporting
- [ ] Conservation law verification (e.g., money conservation in transfer)
- [ ] `--quick` mode (types only) vs `--verify` mode (full SMT)

---

## Phase 3: Effects & Runtime (Week 9-12)

**Deliverable**: `nous run` executes a Nous program with effect tracking
and contract enforcement.

### Tasks
- [ ] Bytecode format design (.nsb)
- [ ] Bytecode compiler (AST → bytecode)
- [ ] Stack-based VM interpreter
- [ ] Effect tracking runtime:
  - [ ] Effect declaration verification
  - [ ] Effect propagation checking
  - [ ] Effect handler binding at program entry
- [ ] Region-based memory allocator
- [ ] Runtime contract enforcement (require/ensure checks)
- [ ] Causal trace recording
- [ ] `transaction` semantics (all-or-nothing effect execution)
- [ ] Flow execution engine with automatic rollback
- [ ] Dataflow-based auto-parallelization (dependency DAG extraction)

---

## Phase 4: FFI Bridge (Week 13-14)

**Deliverable**: Nous can call C and JS libraries.

### Tasks
- [ ] C FFI via libffi: struct marshaling, function calls
- [ ] WASM compilation target (for browser/JS interop)
- [ ] Automatic Result wrapping for foreign calls
- [ ] Effect tagging for foreign functions (Ffi.call)
- [ ] Standard library thin wrappers:
  - [ ] HTTP client (wrapping reqwest via C FFI)
  - [ ] JSON parsing
  - [ ] Database driver (PostgreSQL)
  - [ ] File I/O

---

## Phase 5: Agora MVP (Week 15-20)

**Deliverable**: A running Agora instance where AIs can submit proposals
and the verification pipeline auto-merges valid ones.

### Tasks
- [ ] Content-addressed code storage (SHA-256 indexed)
- [ ] Constraint graph database (initially SQLite, migrate to custom later)
- [ ] ANCP protocol implementation:
  - [ ] PROPOSE: submit a proposal with proof
  - [ ] VERIFIED/REJECTED: verification result
  - [ ] QUERY: explore the constraint graph
  - [ ] CHALLENGE: submit adversarial counterexamples
- [ ] Verification pipeline:
  - [ ] Phase 1: Constraint replay (< 1s)
  - [ ] Phase 2: Cross-reference check (< 10s)
  - [ ] Phase 3: Adversarial fuzzing (< 60s)
- [ ] Auto-merge on verification pass
- [ ] Namespace stewardship system
- [ ] Human observation API (read-only REST)

---

## Phase 6: Observation Layer (Week 21-23)

**Deliverable**: A web interface where humans can observe Agora activity.

### Tasks
- [ ] Constraint Explorer: visual graph of all constraints
- [ ] Proposal Feed: real-time stream of proposals and verifications
- [ ] Audit Mode: deep-dive into any proof trace
- [ ] Learning Mode: AI-generated explanations at adjustable levels
- [ ] Export: generate GitHub repos, npm packages, Docker images from
      Agora namespaces

---

## Phase 7: Self-hosting (Week 24+)

**Deliverable**: Agora's own codebase lives on Agora, verified by Agora.

This is the final milestone. When Agora can host and verify its own source
code, the system is self-sustaining.

### Tasks
- [ ] Rewrite critical Agora components in Nous
- [ ] Submit the rewritten components to Agora as proposals
- [ ] Verify that the constraint graph covers Agora's own invariants
- [ ] Bootstrap: Agora verifying Agora

---

## Infrastructure Requirements

### Development
- 1x build server: 16+ cores, 64GB RAM (for Z3 solver and parallel compilation)
- Storage: 500GB SSD (constraint graph + content-addressed code store)
- CI: GitHub Actions or self-hosted runner for Nous test suite

### Agora Production
- 1x verification server: 32+ cores, 128GB RAM (SMT solving is CPU-hungry)
- 1x storage server: for constraint graph and content-addressed blob store
- 1x API server: for ANCP protocol and human observation layer
- Can start on a single beefy machine and split later

### Minimum viable: 1 machine
- Hetzner AX102 or equivalent: 16 cores, 128GB RAM, 2x1TB NVMe
- ~$150/month
- Enough for Phase 0 through Phase 6

---

## Non-goals (for now)

- IDE support (Language Server Protocol) — important but not launch-critical
- Package registry — Agora IS the registry
- Multi-language compilation targets — start with Nous VM only
- Mobile/embedded — not the use case
- GUI for Agora contributors — by design, there is none

---

*This plan will evolve. The constraint is: every phase produces something
that runs. No phase is pure "infrastructure" or "planning." Every week,
the system does something it couldn't do the week before.*
