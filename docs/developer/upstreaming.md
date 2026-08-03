# Upstreaming

This document is for reviewers considering integration into
[`xlsynth-crate`](https://github.com/xlsynth/xlsynth-crate). It explains the
component's ecosystem role, existing convention alignment, standalone deltas,
and decisions that should be made at the upstream boundary. It is not a project
roadmap or a release promise.

The standalone repository is pinned to `xlsynth-crate` commit
`92bc9b932981c776bb4bb197cd6b6726f17ec090`, which is the engineering baseline
for its current API and support matrix.

## Ecosystem role

The component fills a function-level layer between native XLS IR traversal and
formal or test-generation workflows:

```text
xlsynth / xlsynth-pir
    -> typed symbolic evaluation and canonical path enumeration
       -> witnesses, coverage tests, equivalence checks, downstream executors
```

It complements rather than replaces existing components:

- `xlsynth` remains responsible for compilation, IR objects, concrete
  interpretation/JIT, and the independent XLS SMT translation;
- `xlsynth-pir` provides the native Rust function representation and traversal;
  and
- `xlsynth-prover` provides formal workflows and solver integrations, while
  this component owns mixed concrete/symbolic evaluation, residual XLS values,
  canonical path semantics, and one witness per feasible path.

The pure-function boundary is intentional. Procs, hardware state, timing, and
whole-machine execution should not enter this crate merely to support a
downstream use case.

## Conventions already aligned

- Reusable behavior is a library with typed public APIs; the example is a thin
  consumer.
- Library code does not print, and solver diagnostics become results or errors.
- Observable maps, traces, paths, and serialization are deterministic.
- Arbitrary-width `IrBits` and recursively structured `IrValue` data cross the
  public boundary.
- Maintained source files carry Apache-2.0 SPDX identifiers.
- Public failure contracts and nontrivial algorithms are documented; comments
  explain invariants and policy.
- Compiler warnings, missing public documentation, and unsafe code are denied.
- One checked script runs formatting, Clippy, tests, and rustdoc.
- CI actions, Rust, xlsynth revisions, and the development environment are
  pinned.

The detailed contribution rules are in [`contributing.md`](contributing.md),
and the evidence expected for semantic review is in
[`verification.md`](verification.md).

## Standalone integration deltas

These are repository-boundary choices, not intended forks from upstream
practice:

- Git dependencies should become workspace path-and-version dependencies and
  inherit upstream workspace lints.
- The standalone AMD64 container and CI exist because the pinned
  `xlsynth-sys` artifacts do not support the local Linux ARM64 path. Upstream's
  supported host matrix should become authoritative after integration.
- The date-pinned standalone Rust toolchain should defer to the upstream
  workspace toolchain.
- Release metadata, versioning, and publication belong to the upstream release
  process.
- Corpus fixtures and standalone performance observations should be retained
  only where they remain useful and reproducible in upstream CI.

## Review decisions

The main integration choices should be explicit rather than hidden in a
mechanical code move:

1. Select the destination crate or module and public ownership boundary.
2. Decide whether the expression DAG and path model are public reusable types
   or implementation details of one evaluator crate.
3. Decide whether the solver adapter remains an external Z3 process, consumes
   an injected solver interface, or reuses an upstream facility. Preserve typed
   constraints and explicit solver indeterminacy either way.
4. Review `xlsynth-pir` extensions or representation gaps separately from the
   symbolic evaluator so each layer has clear ownership.
5. Define which corpus, mutation, and bounded performance tests belong in
   ordinary upstream CI versus longer-running validation.

## Suggested integration sequence

1. Land any independently useful `xlsynth-pir` representation changes.
2. Move the expression/value layer and focused semantic tests.
3. Move path enumeration, canonical traces, constraints, and witnesses.
4. Replace standalone dependency and lint configuration with workspace
   equivalents.
5. Adapt the verification suite to upstream test and CI conventions.
6. Run the full workspace suite and obtain focused API, solver-boundary, and
   completeness-semantics review before an upstream release decision.

Upstreaming should preserve the complete/incomplete contract and independent
XLS replay evidence. Packaging changes must not silently narrow the supported
pure-value matrix or convert reference blockers into passes.
