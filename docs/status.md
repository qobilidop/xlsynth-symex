# Implementation status

Snapshot date: 2026-08-02.

This document records changing implementation facts and measured validation
outcomes. It is not the v1 specification; see [`design.md`](design.md). Update
it when a capability or validation result changes rather than putting transient
state in the repository README.

## Summary

The repository now implements the v1 symbolic value domain and canonical path
enumerator. Release-evidence work remains before the repository can be tagged
v1.

## Implemented

The evaluator currently:

- parses function IR with `xlsynth-pir`;
- exposes leaf-function and package-aware evaluation entry points;
- accepts concrete or symbolic values independently at recursively structured
  input leaves;
- represents bits and Boolean constraints with a typed, interned expression
  DAG and emits deterministic SMT-LIB at the solver boundary;
- supports all pinned in-scope pure value operations, including arbitrary- and
  zero-width bits, nested arrays, structured gates and one-hot selects,
  multidimensional indexing/update, and `xlsynth-pir` extension desugaring;
- recursively evaluates pure function calls; and
- evaluates `counted_for` with a static trip count by repeatedly applying its
  pure body to a symbolic carry;
- demand-evaluates every feasible canonical `sel`, `priority_sel`, and
  `one_hot_sel` path while pruning inactive case cones;
- qualifies choice identities by callsite and loop iteration;
- uses Z3 to discard infeasible traces and build complete typed XLS witnesses;
  and
- reports path and resource limits explicitly as incomplete enumeration.

Merged evaluation remains available as supporting infrastructure. Canonical
enumeration returns residual path values, symbolic conditions, sparse selection
traces, and concrete witnesses. The pinned operation matrix contains no
remaining in-scope partial or gap row.

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
- the original curated validation table still records path-witness replay as
  blocked and must be regenerated against the implemented enumerator.

The table, rather than prose in this document, is the detailed source for case
counts and per-function outcomes.

## Gaps to v1

- regenerate the curated validation matrix with completed path counts and
  witness replay for optimized and unoptimized IR;
- add bounded generated-function trace-set comparison independent of the
  symbolic enumerator;
- add mutations that omit, duplicate, relabel, weaken, strengthen, and
  incorrectly activate paths and show the release harness rejects them;
- exercise more concrete/symbolic argument partitions in the curated corpus;
- record release performance ceilings and the supported toolchain versions;
  and
- reconcile the public API documentation and release evidence with the final
  observed results.

The planned sequence for closing these gaps is in [`roadmap.md`](roadmap.md).
