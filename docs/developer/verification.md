# Verification

This document is for contributors and reviewers evaluating whether
`xlsynth-symex` implements its public contract. It owns the correctness and
complete-enumeration argument, the executable evidence inventory, measured
release ceilings, and the release gate. The project does not claim a formal
proof of its own implementation.

## Claims to establish

Verification has two independent obligations:

1. **Value correctness:** every residual guarded result agrees with XLS semantics.
2. **Enumeration completeness:** every feasible canonical trace under the
   declared policy and caller domain is present exactly once.

A merged evaluator can calculate the right value while omitting selection
traces. A set of guards can cover the input domain while carrying incorrect
values or trace labels. The evidence therefore checks values, guards, and traces
separately.

## Semantic validation modes

The project uses four canonical names in tests, manifests, and reports.

### Curated-vector differential testing

Deliberately chosen concrete assignments are evaluated by the symbolic
implementation and the XLS interpreter or JIT. Cases include zero, all ones,
signed boundaries, powers of two, alternating bits, boundary indices and
shifts, upstream examples, and minimized regressions.

### Differential fuzz testing

Generated concrete assignments repeat the independent XLS comparison. Small
domains are exhausted; larger domains use stable seeds and checked budgets.
Failures retain enough IR, input, and seed information for exact replay.

### Symbolic equivalence checking

Where XLS's independent translator supports a function, Z3 checks whether the
merged native result can differ from the XLS-produced symbolic result. `UNSAT`
establishes equivalence for that modeled function; `SAT` supplies a concrete
counterexample. Timeout, `UNKNOWN`, and unsupported translation remain named
non-passes.

### Witness replay

Every feasible guarded result carries a solver model converted into complete
typed XLS arguments. The XLS interpreter or JIT replays those arguments, checks
the complete result, and confirms that the concrete canonical trace matches the
symbolic trace. A witness establishes feasibility and selection behavior; other
validation modes remain necessary for arithmetic values within the guarded
result.

## Completeness evidence

The enumeration harness combines checks that fail when selection traces are missing,
duplicated, or mislabeled:

- solver proof that the union of feasible guards covers the caller's
  constrained input domain;
- solver proof that every pair of returned guards is disjoint;
- solver proof that the piecewise enumerated result equals the merged result;
- canonical-trace uniqueness and deterministic ordering;
- per-selection outcome and cross-selection trace coverage;
- exhaustive concrete trace-set comparison for bounded generated functions;
- feasibility queries for neighboring outcomes;
- witness result and trace replay;
- comparison across concrete/symbolic argument partitions; and
- mutations that omit, duplicate, relabel, weaken, strengthen, or incorrectly
  activate selections and must be rejected by the harness.

Merged-result equality is supporting evidence, not a completeness oracle. Two
active selections may compute equal values, so trace-set and mutation checks are
release requirements.

## Executable inventory

The evidence is layered and checked into the repository:

| Evidence | Source |
|---|---|
| Operation and type coverage | [`../user/support-matrix.md`](../user/support-matrix.md) and [`tests/support_matrix.rs`](https://github.com/qobilidop/xlsynth-symex/blob/main/tests/support_matrix.rs) |
| Primitive and structural semantics | [`tests/operation_semantics.rs`](https://github.com/qobilidop/xlsynth-symex/blob/main/tests/operation_semantics.rs) |
| Generated value comparison | [`tests/differential.rs`](https://github.com/qobilidop/xlsynth-symex/blob/main/tests/differential.rs) |
| Selection and mixed-input semantics | [`tests/selection_enumeration.rs`](https://github.com/qobilidop/xlsynth-symex/blob/main/tests/selection_enumeration.rs) |
| Coverage, trace sets, and mutations | [`tests/enumeration_completeness.rs`](https://github.com/qobilidop/xlsynth-symex/blob/main/tests/enumeration_completeness.rs) |
| Pinned upstream corpus | [`tests/curated_corpus.rs`](https://github.com/qobilidop/xlsynth-symex/blob/main/tests/curated_corpus.rs) and [`validation.tsv`](https://github.com/qobilidop/xlsynth-symex/blob/main/tests/corpus/curated/validation.tsv) |
| Bounded performance guard | [`tests/release_metrics.rs`](https://github.com/qobilidop/xlsynth-symex/blob/main/tests/release_metrics.rs) |

The support matrix currently contains 65 supported pure-value operations and 8
explicit exclusions. Every supported row names an executable test target, and
the matrix test rejects missing targets and in-scope gaps.

The offline curated corpus evaluates six functions in optimized and unoptimized
IR. Its 12 rows account for 72 curated-vector replays, 20,488 deterministic
fuzz replays, 21 all-symbolic witnesses, and 28 one-argument-concrete
partitions. Completed enumerations check trace uniqueness, domain coverage,
piecewise equality, result replay, and concrete trace replay.

Additional bounded evidence includes four generated nested-selection trees over
all 64 selector assignments each, a priority/one-hot cross-product over all 16
selector assignments, enumeration mutation rejection, and 256 stable-seed
generated arithmetic/bitwise graph comparisons.

Nine applicable whole-function comparisons with XLS's independent SMT
translator return `UNSAT`. Three unoptimized corpus comparisons are recorded as
`blocked:xls-reference-translator`: the pinned translator does not support the
relevant `counted_for` or zero-width-heavy graph. Those cells are not passes;
interpreter differential testing, merged-versus-enumerated proof, bounded trace
comparison, and witness replay provide independent applicable evidence.

## Toolchain and performance evidence

The supported verification environment is the checked-in `linux/amd64`
development image used by `./dev.sh` and CI. Rust is pinned by
`rust-toolchain.toml`; xlsynth revisions are pinned by `Cargo.toml` and
`Cargo.lock`; the container pins Z3 and system dependencies; and corpus
manifests pin fixture provenance. These files, rather than copied prose version
lists, are authoritative.

On a Mac15,9 ARM64 host running the pinned x86_64 image under Docker, a warm
dependency-cache `./dev.sh cargo test --workspace` run completed in 120.529
seconds. The container cgroup peak was 1,768,996,864 bytes, including incremental
compilation and the entire test suite; it is not evaluator-only memory.

The executable selection stress case enumerates all 64 masks of a six-case
`one_hot_sel`. Three warm release runs completed in 6.620, 6.627, and 7.639
seconds. The median run spent 3 ms constructing the expression graph and 6.610
seconds across 64 solver queries. The test enforces a conservative 30-second
ceiling in the supported container.

These observations are reproducible baselines, not cross-machine promises or
general scaling claims. Starting a fresh Z3 process per candidate dominates the
measurement. Users should inspect per-request statistics and the performance
guidance in the [`user guide`](../user/guide.md).

## Release gate

A release claiming complete v1 behavior requires all of the following:

- every in-scope pure-value row in the pinned operation matrix is supported and
  names executable coverage;
- the complete containerized check script passes with no formatting, Clippy,
  test, or rustdoc failure;
- every claimed validation result is rerun and checked by tests;
- all in-scope curated functions complete under their declared budgets;
- completed enumerations satisfy domain coverage, trace uniqueness, piecewise
  value equality, and witness replay;
- bounded trace-set comparison and mutations demonstrate that omissions and
  trace errors are detected;
- reference limitations remain named and have independent compensating evidence
  where possible;
- supported toolchain inputs and bounded performance ceilings are pinned and
  measured; and
- the public guide, Rust API documentation, internal design, support matrix,
  and observed implementation agree.

The standard release command is:

```text
./dev.sh ./scripts/check.sh
```

The release process may add packaging or upstream-workspace checks, but it must
not weaken these semantic and enumeration obligations.
