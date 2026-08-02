# xlsynth-symex design

Status: Initial design; no compatibility promises yet.

This document is the authoritative, living description of the intended design.
Historical discussion and alternatives belong in `docs/notes/` during the
initial prototyping phase; references and prior art belong in
[`research.md`](research.md).

## Development environment

All project development commands should run through `./dev.sh` in the
Ubuntu-based development container. The container uses the versioned Ubuntu
24.04 base image and follows `xlsynth-crate` conventions where applicable:
nightly Rust, rustfmt, Clippy, LLVM/libc++, protobuf, and Z3 tooling. The
nightly toolchain is date-pinned in `rust-toolchain.toml`.

The same Dockerfile is used locally and by CI. GitHub Actions validates the
image for pull requests and publishes `main` and immutable commit-SHA tags to
GHCR after changes land on `main`. The image currently targets `linux/amd64`
because `xlsynth-sys` does not provide Linux ARM64 artifacts; Apple Silicon
hosts use Docker's x86 emulation. The wrapper prefers the published image and
falls back to building it locally, so the checked-in Dockerfile remains the
bootstrap path.

## Purpose

`xlsynth-symex` symbolically evaluates pure XLS IR functions. Given a function

\[
f : (T_1, \ldots, T_n) \rightarrow T_r,
\]

it evaluates the function over a mixture of concrete and symbolic arguments to
produce symbolic results, constraints, and, when requested, enumerated paths.

A motivating downstream application models the operational semantics of one
instruction as:

\[
f(\mathit{instruction}, \mathit{state}) \rightarrow \mathit{new\_state}.
\]

When the instruction is concrete and the state is symbolic, `xlsynth-symex`
should residualize instruction-dependent control and enumerate only paths that
depend on symbolic state. A separate project can compose these per-instruction
transitions into a whole-machine executor. Instruction dispatch and
whole-machine execution are not responsibilities of this library.

Although the project uses the familiar term *symbolic execution*, XLS functions
are dataflow graphs rather than imperative control-flow programs. *Symbolic
evaluation* is often the more precise description.

## Scope

The initial target is the pure function subset of XLS IR:

- bits of arbitrary fixed width;
- tuples;
- fixed-size arrays;
- finite, terminating dataflow computations; and
- calls and finite iteration after their semantics are supported by the chosen
  IR layer.

The initial design excludes:

- procs and blocks;
- channels and tokens;
- clocks, schedules, and pipeline timing;
- persistent state outside explicit function values;
- a general or unbounded memory model;
- instruction fetch, dispatch, or sequence exploration;
- ISA-specific conventions such as program counters, traps, and privilege; and
- reconstruction of DSLX source paths from optimized IR.

An XLS array is an ordinary finite value. It does not imply an SMT or processor
memory model.

## System boundary

The expected initial stack is:

```text
DSLX or textual XLS IR
          |
          | xlsynth: parse, typecheck, lower, optimize, concrete execution
          v
      textual XLS IR
          |
          | xlsynth-pir: native Rust representation and traversal
          v
  xlsynth-symex evaluator
          |
          +-- backend-neutral symbolic expression DAG
          +-- path conditions and selection traces
          +-- solver adapter(s)
          +-- model conversion to XLS values
```

`xlsynth` remains the authoritative boundary for compilation and concrete
replay. `xlsynth-pir` provides native Rust node access and is now the evaluator's
IR traversal layer. It is a partial function-focused IR, so its operation and
type coverage must still be measured rather than assumed.

The symbolic value and evaluator layers should not expose processor or
instruction concepts. A state transition is simply one possible XLS function.

## Current native bits slice

The evaluator parses XLS IR with `xlsynth-pir` and constructs its own typed
SMT-LIB bit-vector expressions. The current slice returns one merged result
whose path condition is `true`. It supports bits parameters and results, core
arithmetic and Boolean operations, reductions, comparisons, extensions, static
and dynamic slices, merged selects, structural tuple construction and indexing,
and recursive calls to pure functions in the package. Zero-width bits are
carried structurally with no SMT term and disappear through operations such as
concatenation and extension.

Direct SMT expression strings are an expedient initial representation; they
make the independent validation boundary available early but do not yet provide
structural interning or solver-independent expressions. The expression layer
should become an interned typed DAG as operation coverage and sharing grow.

Semantic validation uses bits-only generated pure functions and curated
upstream examples. Concrete inputs are evaluated with the XLS interpreter and
asserted against the native expression. Whole-function equivalence separately
compares the native result with `IrFunction::to_z3_smtlib`; `UNSAT` is now a
meaningful independent check rather than an adapter self-comparison.

## Symbolic domain

The first implementation should distinguish fully concrete and symbolic
values:

```rust
enum Value {
    Bits(BitsValue),
    Tuple(Vec<Value>),
    Array(Vec<Value>),
}

enum BitsValue {
    Concrete(IrBits),
    Symbolic(ExprId),
}
```

Operations fold concrete operands whenever possible and otherwise construct
symbolic expressions:

```text
Concrete(3) + Concrete(4) -> Concrete(7)
Symbolic(x) + Concrete(4) -> Add(x, 4)
Concrete(0) == Concrete(0) -> Concrete(true)
```

A later refinement may attach known-bit masks to symbolic bit vectors. That
would permit a selector to resolve when only the relevant bits are known,
without requiring the complete value to be concrete.

Symbolic expressions should be backend-neutral. Solver-specific objects should
not be the evaluator's fundamental value representation. This enables multiple
SMT backends, expression interning and simplification, concrete substitution,
and non-SMT analyses over the same evaluator.

## Mixed concrete and symbolic evaluation

For a concrete argument \(i\), evaluating \(f(i, \hat{s})\) computes the
residual function:

\[
f_i(\hat{s}) = f(i, \hat{s}).
\]

This is online partial evaluation. A select with a concrete selector chooses
one operand without creating a symbolic branch or path constraint. A select
with a symbolic selector may either remain a merged `ite` expression or split
into paths according to policy.

Evaluation should be demand-driven and memoized. A topological evaluator would
construct both case cones before discovering that a selector is concrete.
Starting at the return node instead allows the evaluator to visit only the
selected operand cone. Shared nodes are evaluated once per applicable
environment or path.

The pure-function restriction makes this pruning semantically safe: discarded
operands have no side effects. Optional ahead-of-time specialization may later
replace frequently concrete parameters with literals, run XLS optimization,
and cache the residual IR. Online mixed evaluation remains the fundamental
semantics.

## Merged and path-enumerating modes

The library should allow two related modes:

1. **Merged symbolic evaluation** preserves symbolic selections as expressions
   such as `ite`. It promises a symbolic result but not explicit path
   enumeration.
2. **Path-enumerating evaluation** splits at configured choice sites and
   returns a condition, value, and canonical selection trace for each path.

This distinction prevents a correct, compact evaluator from being called
incomplete merely because it deliberately merges control choices.

A conceptual path result is:

```rust
struct PathResult {
    condition: SymBool,
    value: Value,
    trace: SelectionTrace,
}
```

## Choice sites and split policy

XLS IR has no control-flow graph. It evaluates dataflow eagerly and expresses
control-like behavior with selection operations. Paths in this project are
therefore defined relative to explicit IR choice sites and a split policy.

Initial policy:

- `sel` is an exclusive choice site;
- `priority_sel` is an exclusive choice site with guards that exclude all
  higher-priority cases;
- `one_hot_sel` is exclusive only when one-hotness is established; otherwise it
  remains merged or records a selected-case bitmask without enumerating every
  subset; and
- dynamic array indices and bit slices remain symbolic data selection rather
  than path splits by default.

The policy should eventually be configurable. A trace is tied to the exact IR
function, optimization state, node identities, symbolic/concrete argument
partition, demanded root, and split policy. It is not expected to be stable
across optimization or compiler versions.

## Canonical selection traces

A selection trace is a partial valuation of choice sites induced by the
demanded dynamic slice:

\[
\tau : \mathit{ChoiceNodeId} \rightharpoonup \mathit{Outcome}.
\]

It is a map, not a temporal sequence. Node IDs may provide a stable
serialization order within an IR artifact, but graph scheduling order has no
semantic significance.

Consider nested selection:

```text
outer = if x { inner } else { c }
inner = if y { a } else { b }
```

The canonical traces are:

```text
{outer: else}
{outer: then, inner: else}
{outer: then, inner: then}
```

When the outer `else` case is selected, the inner choice is structurally
inactive. Its outcome is a don't-care and must not cause redundant path splits.
Absence from a sparse trace means inactive; a diagnostic full representation
may use an explicit `Inactive` state.

Inactive does not mean that every arbitrary completion is concretely feasible.
It means only that the path condition and result do not observe or constrain
that choice. The actual selector may still be determined by dependencies among
the inputs.

Structural inactivity is distinct from semantic irrelevance. A demanded select
whose cases happen to compute equal values is structurally active even though
its decision does not affect the result. Solver-backed or expression-based
trace minimization may recognize semantic irrelevance later; the initial
canonical form only removes structurally inactive choices.

For binary choice sites, traces may be represented as a mask/value pair. For
general choices, a sparse ordered map from node ID to outcome is clearer.

## Demand semantics for traces

For a fixed path:

1. Demand the function return node.
2. Ordinary operations demand their operands.
3. A concretely resolved select demands its selector and selected case only.
4. A split symbolic select demands its selector and the selected case for that
   path.
5. Unselected cases are not demanded.
6. A shared node is active if any demanded use reaches it.

Selectors required to compute another selector are part of the demanded cone
and may themselves contribute choices. This definition gives traces a precise
operational meaning without claiming to reproduce DSLX source control flow.

## Validation strategy

The initial project seeks strong validation, not a formal proof of the
implementation or exhaustive path enumeration.

### Curated-vector differential testing

For deliberately selected concrete assignments to all symbolic inputs:

1. evaluate the symbolic result under the assignment;
2. run the same inputs through the XLS interpreter or JIT; and
3. compare the complete typed values.

Curated vectors should include upstream example vectors, structured edge cases
such as zero, all ones, signed boundaries, powers of two, alternating bits, and
boundary indices and shifts, plus minimized fuzz failures promoted to permanent
regressions.

### Differential fuzz testing

Differential fuzz testing applies the same concrete comparison to generated
inputs. Small total input spaces should be exhausted. Larger tests should use
stable per-corpus seeds and bounded case counts in ordinary CI, with broader or
rotating campaigns available separately. Failures must report enough metadata
to replay the exact case. Type-aware and correlated generators should be added
as tuples, arrays, and mixed argument partitions enter the supported corpus.

### Path-witness replay

For each feasible enumerated path, solve its path condition, replay one or more
models concretely, compare the result with XLS, and confirm that the concrete
selection trace agrees with the symbolic trace. Multiple models per path should
exercise boundaries within a path; one witness covers control but not all
arithmetic behavior.

### Symbolic equivalence checking

Where supported, compare the complete symbolic result against XLS's independent
SMT translation. Ask whether:

\[
\hat{f}_{\text{xlsynth-symex}}(X) \ne \hat{f}_{\text{XLS}}(X)
\]

is satisfiable. `UNSAT` establishes equivalence for the modeled inputs and
operations relative to the trusted XLS translation. `SAT` yields a concrete
counterexample for replay; timeout or `UNKNOWN` falls back to differential
testing.

These four names are canonical across design documentation, manifests, tests,
and reports: **curated-vector differential testing**, **differential fuzz
testing**, **symbolic equivalence checking**, and **path-witness replay**.
Every corpus entry and IR form declares each validation as required, blocked by
a named capability, or not applicable. A blocked validation is neither a pass
nor a silent skip.

### Enumeration confidence

The initial release does not attempt to prove that path enumeration is
exhaustive. Confidence should come from:

- per-choice outcome coverage;
- branch-flipping test generation;
- concrete trace replay;
- differential fuzzing across concrete/symbolic argument partitions;
- comparison of merged results with reconstructed path results; and
- mutation tests that omit, relabel, or incorrectly activate choices.

Functional equivalence remains meaningful even if enumeration is conservative
or deliberately merged.

## Evaluation corpus

Evaluation should be layered:

1. **Semantic microtests** cover each supported operation with concrete,
   symbolic, and mixed operands at varied bit widths.
2. **Curated XLS functions** draw from upstream examples and instantiated DSLX
   standard-library routines.
3. **Deterministic generated functions** use fixed XLS fuzzer seeds and bounded
   graph sizes for broad operation and type coverage.
4. **Historical crashers** provide adversarial combinations.
5. **Stress benchmarks** such as SHA-256 and floating-point arithmetic measure
   expression growth, pruning, and solver behavior without initially gating
   basic correctness.

Both optimized and unoptimized IR should be tested. Corpus manifests should pin
the XLS/xlsynth version, upstream revision, function name, argument partition,
and expected feature requirements.

The curated upstream tier is stored under `tests/corpus/curated`. Tests must be
offline and reproducible: DSLX fixtures are copied without modification from a
pinned `xlsynth/xlsynth` commit compatible with the repository's
`xlsynth-crate` dependency, retain their upstream license notices, and are
described by a tab-separated manifest. The harness compiles each fixture with
the bundled DSLX standard library and exercises the selected pure function in
both unoptimized and optimized IR forms. Deterministic concrete samples are
evaluated by the XLS interpreter and asserted against the symbolic SMT result.
Stable per-entry seeds and case budgets drive differential fuzz testing; the
small `tiny_adder` domain is exhausted. A separate validation matrix records
requirements and capability blockers for all four validation modes per IR
form.

The curated slice currently covers widened addition (`tiny_adder`), nested
selection (`nested_sel`), the opcode decoder from `riscv_simple`, and tuple- and
multiplication-heavy overflow detection (`overflow_detect`). Corpus
cases gate only when both IR forms are supported. Examples whose unoptimized IR
contains operations rejected by the current XLS SMT translator, such as
`counted_for`, should be added later with an explicit capability-status model
rather than silently skipping a validation mode.

Symbolic equivalence is required for every currently supported curated function
and IR form where XLS's reference translator can produce a query. A known XLS
translator abort on the zero-width-heavy unoptimized overflow example is
recorded as an explicit reference-side blocker; the optimized form proves
equivalent. Until explicit path enumeration and canonical selection traces
exist, path-witness replay remains blocked. The corpus matrix records these
limitations explicitly.

Useful measurements include operation and type coverage, expression DAG size,
paths and choice outcomes, concretely pruned selects, visited IR nodes,
construction and solving time, and peak memory. For mixed evaluation, a useful
metric is the reduction in visited nodes when discriminator arguments are
concrete.

## Non-goals for initial development

- Whole-ISA or whole-processor symbolic execution.
- Unbounded or SMT-array memory semantics.
- Stateful XLS proc execution.
- Stable source-level coverage identities.
- A proof that all paths have been enumerated.
- Solver-independent formal verification of the implementation.
- Exhaustively splitting every mux-like data operation.

## Initial milestones

1. Parse and traverse a small pure function IR.
2. Implement concrete and symbolic bit values and expression interning.
3. Match concrete XLS semantics for a core set of bit operations.
4. Add tuples and fixed-size arrays.
5. Implement demand-driven mixed evaluation and concrete select pruning.
6. Add merged select expressions and configurable path splitting.
7. Define and test canonical selection traces with inactive choices.
8. Integrate one solver and model-to-XLS-value conversion.
9. Build differential, path-witness, and symbolic-equivalence harnesses.
10. Run curated and deterministic generated corpora and publish coverage gaps.

## Open questions

- Which `xlsynth-pir` operations and types are sufficiently complete and stable?
- What expression representation and solver adapter API should be used?
- Which solver should be the first backend?
- What should be the exact default split policy for non-exclusive selections?
- How should invokes and finite loops appear in traces before and after
  inlining?
- How should node identities and function fingerprints be represented?
- When is known-bit propagation worth adding to mixed evaluation?
- Should ahead-of-time specialized Rust or IR generation become a supported
  output, or remain an optimization internal to evaluation?
