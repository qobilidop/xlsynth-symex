# Initial design discussion

- Date: 2026-08-01
- Status: Historical working notes for the rapid-prototyping phase
- Authoritative design: [`../design.md`](../design.md)

These notes preserve the reasoning that led to the initial design. They are not
a specification. Once the project transitions to a PR-based workflow, durable
current conclusions should remain in `design.md`, prior art in `research.md`,
and these notes may be removed after useful material is harvested. Git history
will preserve the record.

## Project identity and ecosystem

The discussion began by distinguishing three layers:

1. `google/xls` is the upstream C++/Bazel HLS compiler and owns DSLX, XLS IR,
   optimization, interpretation/JIT, scheduling, code generation, and formal
   analysis.
2. `xlsynth/xlsynth` is a downstream fork that publishes XLS functionality
   through a shared C API and binary artifacts.
3. `xlsynth-crate` wraps that API for Rust and adds native Rust tools including
   `xlsynth-pir` and `xlsynth-prover`.

For a symbolic evaluator, Rust offers ergonomic expression data structures,
solver integration, and fast iteration. `xlsynth-pir` provides traversable
function IR while `xlsynth` provides authoritative compilation and concrete
execution. Direct C++ integration would offer broader and more immediate XLS
semantic coverage, especially for procs and blocks, but those are outside the
chosen scope.

This motivated renaming the project from `xls-symex` to `xlsynth-symex`: the
name signals a Rust implementation in the xlsynth ecosystem rather than an
upstream C++ XLS component.

## Motivating application and scope correction

The motivating application is inspired by the Sail/Isla division. A user can
model one instruction's operational semantics as a pure XLS function and
mechanically lift it into a symbolic transition. A downstream whole-ISA
executor may compose those transitions.

An important clarification was that whole-processor symbolic execution is not
part of `xlsynth-symex`. It is a motivating consumer used to drive the initial
design. The library should not know about program counters, instruction fetch,
architectural memory, or ISA dispatch.

The scope narrowed further to pure XLS functions. This excludes procs, blocks,
channels, tokens, timing, and persistent implicit state. The result is a finite
dataflow evaluation problem over explicit values.

## Small-state machines and memoryless motivation

Traditional useful CPU ISAs almost always expose memory. RV32I was considered
as a small complete target, but the confidential motivating use case more
closely resembles specialized hardware with a small persistent register state
and no large attached memory.

Classical "register machine" terminology was found to be misleading because it
usually denotes unbounded counters in computability theory. Better prior-art
connections include extended finite-state machines, word-level symbolic
transition systems, and especially Instruction-Level Abstraction for
accelerators. A small cryptographic command engine was identified as a possible
public demonstration without implying that accelerator modeling belongs in the
core API.

## Mixed concrete and symbolic arguments

The most important motivating invocation has a concrete instruction and
symbolic state:

```text
f(concrete_instruction, symbolic_state) -> symbolic_new_state
```

Instruction-dependent branches should resolve concretely and should neither
fork symbolic paths nor construct expressions for unused alternatives. This is
mixed concrete/symbolic evaluation, or online partial evaluation of a residual
function.

The need to avoid constructing both arms led to a demand-driven evaluator
design. Starting from the return node permits concrete selections to prune
unused dependency cones. A topological eager traversal would lose much of this
benefit even though it would remain functionally correct.

Known-bit propagation was discussed as a later generalization: a discriminator
may resolve when only its opcode field is concrete even if the entire argument
is not.

## What is an execution path in XLS IR?

At DSLX source level, `if` and `match` provide an intuitive execution-path
notion. XLS IR is different: it is an eager dataflow graph, and branch-like
behavior is represented by selections. Both case computations may exist and be
evaluated before a `sel` chooses a value.

An IR-level path was therefore defined in terms of selection decisions, not
basic blocks or executed instructions. This path is not guaranteed to match a
DSLX source path:

- one source branch may lower to several selections;
- matches may use priority or one-hot forms;
- optimization may remove, merge, or restructure selections; and
- source positions are not durable branch identities.

The project should expose IR choice observations without claiming source-level
coverage.

## Inactive selections and canonical traces

Nested selections revealed that recording every selector value creates
redundant paths. When an outer choice selects an alternative that does not
demand an inner select, the inner outcome is inactive and should be a
don't-care.

This produced the notion of a canonical selection trace: a partial valuation of
choice nodes in the demanded dynamic slice. Inactive choices are absent or
explicitly marked `Inactive`. This collapses traces such as:

```text
outer=false, inner=false
outer=false, inner=true
```

into:

```text
outer=false, inner=inactive
```

An inactive choice may have a concrete selector value determined by the input;
the trace simply does not observe or constrain it. Arbitrary trace completion
does not imply that every completion is feasible.

Structural inactivity was separated from semantic irrelevance. A demanded
select with equal arms may not affect the result, but recognizing that fact can
require simplification or solving. Initial trace canonicalization should only
remove structurally inactive nodes.

## Correctness and enumeration validation

Several complementary validation methods were identified.

Pure differential fuzzing compares symbolic-result substitution against the
XLS interpreter or JIT. It explores values within paths but may miss rare
guards. Solver-derived witnesses target feasible enumerated paths but one
witness per path may miss arithmetic corner cases.

The strongest proposed guardrail is whole-function symbolic equivalence against
XLS's existing SMT translation. Randomly generated bounded pure functions can
be checked by asking whether the two symbolic results ever differ, with any
model replayed concretely.

Proving exhaustive path enumeration was considered and deliberately deferred.
An attempted independent trace-completeness scheme risked reimplementing the
symbolic executor as its own oracle. Instrumenting source functions with traces
could avoid some circularity but would complicate integration and require an
upstream or frontend transformation. For initial development, reasonable
validation is sufficient:

- choice-outcome coverage;
- path-condition branch flipping;
- concrete trace replay;
- differential fuzzing;
- merged-versus-enumerated result comparison; and
- mutation tests for omitted and mislabeled choices.

An under-enumerating evaluator can remain functionally correct and useful,
especially when selections are intentionally merged into `ite` expressions.
The API should distinguish merged evaluation from a mode that promises explicit
enumeration.

## Corpus strategy

No single purpose-built XLS pure-function corpus was found. A layered corpus is
more valuable:

- operation-level semantic microtests;
- readable upstream examples and standard-library functions;
- deterministic fuzzer-generated functions bounded by size;
- historical XLS fuzzer crashers; and
- larger stress tests such as SHA-256 and floating-point arithmetic.

The upstream `riscv_simple.x` example is especially relevant for measuring how
much a concrete instruction prunes a generic decoder and semantic function.
Both optimized and unoptimized IR should be included because optimization
changes graph shape and traces.

## Documentation lifecycle

During rapid prototyping, this historical note preserves design discussion
without forcing every tentative idea into a permanent decision record. The
living `design.md` states the current design and `research.md` holds prior art.

Once development becomes PR-driven, PR descriptions and review discussion will
record the rationale for changes. Relevant final behavior must still be updated
in `design.md`, since external PR history is not a substitute for a repository's
current documentation. These initial notes can then be cleaned up rather than
maintained indefinitely.
