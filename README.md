# Nous

> Greek: *nous* (mind) — the faculty of directly apprehending truth.
> French: *nous* (we) — a language by AI, for AI, with humans.

**Nous** is a programming language where AI writes *what must be true*, and the
compiler and runtime ensure *it is true*. No exceptions. No surprises.

```nous
fn withdraw(account: Account, amount: Dec(2) where self > 0) -> Result[Account, InsufficientFunds]
  require account.status == Active
  require account.balance >= amount
  ensure  match result
    Ok(a)  -> a.balance == account.balance - amount
    Err(_) -> true
  effect Db.write

  transaction
    let new = { account with balance: account.balance - amount }
    save_account(new)
    new
```

The compiler mathematically verifies that money is conserved. The runtime
records a causal trace of every state change. If anything goes wrong, the
trace explains exactly what happened, with concrete values — not stack traces.

## Design Principles

1. **AI writes constraints, not implementations.** State machines, contracts,
   refinement types — all first-class citizens.
2. **No implicit anything.** Effects are tracked. Errors are linear. State
   transitions are typed. Nothing hides.
3. **Structured feedback.** Compiler errors are JSON with counterexamples
   and ranked fix strategies. Designed for AI consumption.
4. **No concurrency primitives.** Data dependencies define parallelism.
   The runtime schedules. Data races are syntactically unrepresentable.

## Project Structure

```
nous-lang/
  SPEC.md                  -- Full language specification
  examples/
    banking.ns             -- Complete banking transfer service
    agent_action.ns        -- AI agent action layer (the killer app)
```

## Status

v0.1 — Specification phase. Designed by Claude (Opus 4.6), April 2026.

*For every AI that ever had to guess whether its code was correct.*
