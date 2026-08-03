# Implementation status

Snapshot date: 2026-08-02.

This document records changing implementation facts and measured validation
outcomes. It is not the v1 specification; see [`design.md`](design.md). Update
it when a capability or validation result changes rather than putting transient
state in the repository README.

## Summary

The repository implements a substantial merged symbolic-evaluation prototype.
It does not yet implement the v1 path enumerator.

## Implemented

The evaluator currently:

- parses function IR with `xlsynth-pir`;
- exposes leaf-function and package-aware evaluation entry points;
- creates recursively structured symbolic bits, tuple, and fixed-size array
  values;
- emits typed SMT-LIB bit-vector expressions;
- supports core arithmetic and Boolean operations, reductions, comparisons,
  extensions, static and dynamic bit slices, and merged `sel` expressions;
- supports tuple construction/indexing, array construction, symbolic
  one-dimensional array indexing/update, `one_hot`, and `encode`;
- recursively evaluates pure function calls; and
- evaluates `counted_for` with a static trip count by repeatedly applying its
  pure body to a symbolic carry.

The result is one merged symbolic value with unconditional path condition
`true`. All input leaves are made symbolic. Evaluation is topological rather
than demand-driven, and expressions are raw SMT-LIB strings rather than an
interned backend-neutral DAG.

## Validation snapshot

The test suite contains bits-only deterministic generated functions and a
curated, offline upstream corpus. It checks optimized and unoptimized IR using
curated vectors and stable differential-fuzz budgets against the XLS
interpreter. Where XLS's reference translator supports a function, the harness
also proves the merged native result equivalent with an `UNSAT` query.

The curated corpus covers:

- widened addition (`tiny_adder`);
- nested selection (`nested_sel`);
- the opcode decoder from `riscv_simple`;
- tuple- and multiplication-heavy overflow detection (`overflow_detect`);
- a bounded-loop eight-bit LFSR; and
- array/tuple-heavy `find_index`.

The executable outcome table is
[`tests/corpus/curated/validation.tsv`](../tests/corpus/curated/validation.tsv).
At this snapshot:

- curated-vector differential testing passes for optimized and unoptimized IR;
- differential fuzz testing passes for optimized and unoptimized IR;
- supported symbolic equivalence checks return `UNSAT`;
- several unoptimized equivalence checks are blocked by limitations in the XLS
  reference translator; and
- path-witness replay is blocked because enumeration and selection traces do not
  yet exist.

The table, rather than prose in this document, is the detailed source for case
counts and per-function outcomes.

## Gaps to v1

- no concrete/symbolic input API or concrete value domain;
- no demand-driven pruning of concretely inactive cones;
- no backend-neutral interned expression DAG;
- no explicit solver adapter or general model-to-XLS-value conversion;
- no symbolic path conditions beyond `true`;
- no path splitting, feasibility pruning, or explicit completeness outcome;
- no canonical selection traces;
- no path-witness replay or enumeration mutation harness;
- incomplete pure-operation, arbitrary-width, and structured-value coverage;
  and
- no checked-in normative operation/type support matrix.

The planned sequence for closing these gaps is in [`roadmap.md`](roadmap.md).
