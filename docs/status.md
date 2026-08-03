# Implementation status

Snapshot date: 2026-08-02.

This document records changing implementation facts and measured validation
outcomes. It is not the v1 specification; see [`design.md`](design.md). Update
it when a capability or validation result changes rather than putting transient
state in the repository README.

## Summary

The implementation is a v1 review candidate. Its automated release contract
currently passes with no known implementation, IR-layer, path-enumeration, or
release-evidence gaps in the declared pure-function scope. The crate remains at
`0.1.0`; v1 has not been tagged or released while review and refinement
continue.

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
- records every demanded choice in the trace, including a concretely resolved
  choice that produces no fork;
- qualifies choice identities by callsite and loop iteration;
- uses Z3 to discard infeasible traces and build complete typed XLS witnesses;
  and
- accepts caller constraints, a per-query solver timeout, and a returned-path
  limit, and reports every solver or resource limit explicitly as incomplete
  enumeration.

Merged evaluation remains available as supporting infrastructure. Canonical
enumeration returns residual path values, symbolic conditions, sparse selection
traces, concrete witnesses, explicit completeness, and construction/solver
statistics. The pinned operation matrix contains no in-scope partial or gap
row.

## Validation snapshot

The checked support matrix contains 65 supported pure-value operations and 8
explicit exclusions. Every supported row names an executable semantic or path
test, and the matrix test verifies that those coverage targets exist.

The curated, offline upstream corpus checks six functions in optimized and
unoptimized IR. Its 12 validation rows account for 72 curated-vector replays,
20,488 deterministic fuzz replays, and 21 all-symbolic path witnesses. It also
checks 28 one-argument-concrete partitions. Every completed enumeration is
checked for canonical-trace uniqueness, domain coverage, piecewise equality to
merged evaluation, XLS result replay, and fully concrete trace replay.

The curated corpus covers:

- widened addition (`tiny_adder`);
- nested selection (`nested_sel`);
- the opcode decoder from `riscv_simple`;
- tuple- and multiplication-heavy overflow detection (`overflow_detect`);
- a bounded-loop eight-bit LFSR; and
- array/tuple-heavy `find_index`.

The executable outcome table is
[`tests/corpus/curated/validation.tsv`](../tests/corpus/curated/validation.tsv).
The independent completeness evidence additionally includes:

- exhaustive concrete trace-set comparison for four generated nested-selection
  trees over all 64 selector assignments each;
- exhaustive priority/one-hot cross-product comparison over all 16 selector
  assignments;
- rejection of omitted, duplicated, relabeled, weakened, strengthened, and
  incorrectly activated path mutations;
- 256 stable-seed generated arithmetic/bitwise graph comparisons; and
- concrete comparison, all-symbolic evaluation, and per-argument-mixed
  evaluation across ordinary value operations, plus dedicated mixed tests for
  choice forms, calls, and loops.

Nine applicable whole-function comparisons with XLS's independent SMT
translator return `UNSAT`. Three unoptimized comparisons remain
`blocked:xls-reference-translator`; they are external translator limitations,
not passes, and are compensated by XLS interpreter differential testing,
merged-versus-enumerated proofs, bounded trace-set comparison, and witness
replay. The validation table remains the detailed source for per-function
outcomes.

## Toolchain and measured ceilings

The supported release environment is:

- crate `xlsynth-symex` `0.1.0` (untagged v1 review candidate);
- Rust `nightly-2026-08-02`, `rustc 1.99.0-nightly`
  (`73dc9167f1cd099e525c9ade2e068d1907b78564`), and Cargo
  `1.99.0-nightly` (`7c83d4cc0`);
- `xlsynth` and `xlsynth-pir` revision
  `92bc9b932981c776bb4bb197cd6b6726f17ec090`;
- Z3 `4.8.12`;
- Linux `x86_64` development image
  `ghcr.io/qobilidop/xlsynth-symex/dev@sha256:093ef781f8556c61dec75e91d32e47a966327a9ddff4a20f64c9b324a6e1da8a`;
  and
- curated XLS fixtures pinned to upstream revision
  `12bb182e4d842228878d6caf5489df5565c81aa0`.

On a Mac15,9 ARM64 host running the pinned `x86_64` image under Docker, a warm
dependency-cache `./dev.sh cargo test --workspace` run completed in 120.529
seconds. The container's cgroup peak, including the incremental compile and all
tests, was 1,768,996,864 bytes. These are reproducible observations, not
cross-machine promises.

The executable stress gate enumerates all 64 masks of a six-case
`one_hot_sel`. The release sample completed in 6.373 seconds: 6 ms constructing
155 expression nodes and 6.345 seconds in 64 solver queries. The test enforces
a conservative 30-second ceiling in the supported container.

The default per-query solver timeout is 10 seconds. Callers may set a returned
path limit, and the evaluator has a 1,000,000-syntactic-branch safety ceiling;
reaching any limit produces `Incomplete`, never a full-coverage claim. Starting
a fresh Z3 process per candidate dominates the stress measurement and is a
clear optional optimization beyond v1.

## Review state

The automated evidence currently meets the documented v1 gate, but release is
intentionally deferred for human review and refinement. After v1 is accepted,
future upstreaming, additional solver adapters, broader corpora, and performance
work are listed in [`roadmap.md`](roadmap.md).
