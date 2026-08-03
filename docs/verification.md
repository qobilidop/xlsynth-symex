# V1 verification

This document defines the evidence required to call `xlsynth-symex` v1
complete. The semantic target is specified in [`design.md`](design.md); current
results are recorded in [`status.md`](status.md).

The objective is a strong, independently checkable argument for value
correctness and complete path enumeration. The project does not claim a formal
proof of its own implementation.

## What must be established

Verification has two distinct obligations:

1. **Value correctness:** every path result agrees with XLS semantics.
2. **Enumeration completeness:** every feasible canonical trace under the
   declared path policy is present exactly once.

Either property can hold without the other. A merged evaluator can compute the
right value while omitting paths. Conversely, path conditions can partition the
input domain while carrying incorrect result expressions. V1 verifies both.

## Semantic validation modes

The four canonical validation modes are:

### Curated-vector differential testing

For deliberately selected concrete assignments:

1. substitute the assignment into the symbolic result;
2. run the same inputs through the XLS interpreter or JIT; and
3. compare the complete typed values.

Vectors include zero, all ones, signed boundaries, powers of two, alternating
bits, boundary indices and shifts, and minimized failures promoted to permanent
regressions.

### Differential fuzz testing

Differential fuzzing repeats the concrete comparison over generated inputs.
Small total input domains are exhausted. Larger cases use stable per-corpus
seeds and bounded ordinary-CI budgets, with broader campaigns available
separately. Failures report enough metadata to replay the exact case.

### Symbolic equivalence checking

Where XLS's independent translation supports the function, compare the merged
native result with the XLS result by asking whether:

\[
\hat{f}_{\text{xlsynth-symex}}(X) \ne \hat{f}_{\text{XLS}}(X)
\]

is satisfiable. `UNSAT` establishes equivalence for the modeled function
relative to that reference. `SAT` yields a counterexample for concrete replay.
A timeout, `UNKNOWN`, or unsupported reference translation is recorded and
does not become a pass.

### Path-witness replay

For every feasible enumerated path:

1. solve its path condition;
2. convert the model into typed XLS input values;
3. replay the function with the XLS interpreter or JIT;
4. compare the complete result; and
5. confirm that the concrete canonical trace agrees with the symbolic trace.

One witness establishes reachability and exercises control. Additional models
within a path are used for arithmetic boundaries because one witness does not
cover all value behavior.

These four names are canonical across documentation, manifests, tests, and
reports. Each corpus entry and IR form classifies every mode as passing, not
applicable for a stated reason, or blocked by a named external capability. A
blocked cell is neither a pass nor a skipped test.

## Enumeration-completeness evidence

Full path coverage requires checks that can detect a value-correct evaluator
with missing, duplicated, or mislabeled traces. The v1 harness combines:

- solver proof that the union of feasible path conditions covers the caller's
  input domain;
- solver proof that the piecewise enumerated result equals the merged result;
- canonical-trace uniqueness checks;
- per-choice outcome and cross-choice trace coverage;
- exhaustive comparison with concrete traces for bounded generated functions;
- branch-flipping generation for uncovered neighboring outcomes;
- witness replay for every feasible path;
- comparison across concrete/symbolic argument partitions; and
- mutation tests that omit, duplicate, relabel, weaken, strengthen, or
  incorrectly activate paths and demonstrate that the harness rejects them.

Merged-result equivalence alone is insufficient: two different active choices
may compute the same value. Trace-set checks and enumeration mutations are
therefore release requirements.

## Coverage inventory

The checked-in operation/type support matrix is normative for the pinned
XLS/xlsynth toolchain. Tests cross-check every supported row against semantic
microtests and ensure that exclusions remain explicit. IR-layer gaps may appear
in pre-v1 status, but no in-scope pure value row may remain a gap at v1.

Validation is layered:

1. **Semantic microtests** cover every supported operation with concrete,
   symbolic, and mixed operands at varied bit widths.
2. **Path microtests** cover each choice form, nested inactivity, infeasible
   outcomes, defaults, and one-hot and non-one-hot selectors.
3. **Curated XLS functions** come from pinned upstream examples and instantiated
   DSLX standard-library routines.
4. **Deterministic generated functions** use fixed XLS fuzzer seeds and bounded
   graph sizes for broad operation, type, and trace coverage.
5. **Historical crashers** preserve adversarial combinations.
6. **Stress benchmarks** measure expression growth, path growth, pruning, and
   solver behavior without all becoming ordinary correctness gates.

Optimized and unoptimized IR are tested independently. Corpus manifests pin the
XLS/xlsynth version, upstream revision, function name, argument partition,
features, seeds, and budgets. Failures identify the IR, inputs, path policy, and
solver query needed for exact replay.

## V1 release gate

V1 may be declared only when:

- every in-scope pure value row in the operation/type matrix is supported and
  covered, with no remaining implementation or IR-layer gap;
- all standard containerized formatting, lint, and test commands pass;
- every claimed result in the validation matrix is rerun by tests;
- path-witness replay is no longer blocked by missing evaluator capability;
- all in-scope curated functions complete enumeration under their declared
  budgets;
- every completed enumeration satisfies domain coverage, trace uniqueness,
  piecewise-result equivalence, and witness replay;
- external reference limitations are named and compensated with independent
  checks where possible;
- mutation tests demonstrate that omissions and trace errors are detected;
- current performance ceilings and supported toolchain versions are measured
  and recorded; and
- the design, public API documentation, status report, and observed behavior
  agree.

Standard local and CI validation commands are:

```text
./dev.sh cargo fmt --all -- --check
./dev.sh cargo clippy --workspace --all-targets -- -D warnings
./dev.sh cargo test --workspace
```

All project development and verification commands run through `./dev.sh` so
that local and CI results use the checked-in AMD64 development environment.
