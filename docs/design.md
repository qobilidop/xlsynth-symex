# xlsynth-symex v1 design

This document is the authoritative specification of the v1 end state. It
describes what the repository means to build, independent of the current
implementation sequence.

In this repository, **v1 means done**: the core symbolic-evaluation and path
enumeration promise is complete. After v1, the next project-level question is
whether and how to upstream the work, not which essential capability is still
missing.

For changing implementation facts, see [`status.md`](status.md). For the work
sequence, see [`roadmap.md`](roadmap.md). For the evidence required to call v1
complete, see [`verification.md`](verification.md). Prior art belongs in
[`research.md`](research.md), and historical discussion belongs in
[`notes/`](notes/).

## Purpose

`xlsynth-symex` symbolically evaluates finite, pure XLS IR functions over a
mixture of concrete and symbolic arguments. Its primary value is exhaustive
test generation over the function's IR-level control choices:

```text
pure XLS function + concrete/symbolic arguments
    -> every feasible canonical path
       + path condition
       + symbolic result
       + selection trace
       + solver-derived concrete witness
```

The motivating downstream application models one instruction as:

\[
f(\mathit{instruction}, \mathit{state}) \rightarrow \mathit{new\_state}.
\]

With a concrete instruction and symbolic state, the evaluator resolves
instruction-dependent choices without generating irrelevant paths and
enumerates every feasible path that still depends on symbolic state. A separate
project may compose these per-instruction transitions into a whole-machine
executor.

Although the project uses the familiar term *symbolic execution*, XLS functions
are dataflow graphs rather than imperative control-flow programs. *Symbolic
evaluation* is the more precise description of the core mechanism.

## V1 contract

### Supported semantic domain

V1 supports finite, terminating, value-producing XLS functions whose values are
formed recursively from:

- bits of arbitrary fixed width, including zero-width bits where XLS permits
  them;
- tuples; and
- fixed-size arrays, including nested arrays and structured elements.

The supported computation includes ordinary pure calls and finite iteration.
DSLX parametrics and source-level types matter only through the XLS IR to which
they lower.

A checked-in operation-and-type support matrix pins the XLS/xlsynth version and
lists every function-level IR operation in that pinned toolchain, together with
its representation status in the selected Rust IR layer. Every entry is
classified as:

- supported and covered by executable semantic tests;
- outside the pure value domain for a stated reason; or
- a pre-v1 implementation or IR-layer gap.

At v1, no in-scope pure value entry remains a gap. If the selected Rust IR layer
cannot represent such an operation, that layer is extended or replaced rather
than silently narrowing the semantic claim. The v1 design does not exclude a
finite value operation merely because it was absent from an earlier prototype.

### Mixed concrete and symbolic evaluation

The public evaluator accepts a concrete or symbolic value at every input leaf.
For a concrete argument \(i\), evaluating \(f(i, \hat{s})\) computes the
residual function:

\[
f_i(\hat{s}) = f(i, \hat{s}).
\]

Operations fold concrete operands when possible. A concretely resolved select
demands only its selected case, creates no symbolic fork for that choice, and
does not visit unused case cones. Its resolved outcome is still recorded in the
canonical trace, so a solver witness can be replayed against the same active
choice map.

Evaluation is demand-driven and memoized. Starting from the return node lets a
concrete selector prune unused dependencies before they are constructed. Shared
demanded nodes are evaluated once per applicable environment or path. The
pure-function restriction makes this pruning semantically safe because discarded
operands have no side effects.

### Complete enumeration

For the declared path policy, v1 enumerates every feasible canonical path. It
never silently preserves an in-scope symbolic choice as a merged expression and
reports enumeration as complete.

Completion is an explicit outcome. A conceptual result is:

```text
EnumerationResult {
    paths: [PathResult],
    completeness: Complete | Incomplete(reason),
}

PathResult {
    condition,
    value,
    trace,
    witness,
}
```

The exact Rust API may differ, but it preserves the distinction. A timeout,
resource limit, unsupported operation, or solver failure may yield useful
partial paths, but the outcome is incomplete and is not full path coverage.

For a completed enumeration:

- every returned path condition is feasible;
- the path conditions cover the input domain under the caller's constraints;
- every trace is canonical and unique;
- every feasible canonical trace under the declared policy is represented;
- the piecewise union of path results equals the merged function result; and
- every path has a concrete witness whose XLS replay agrees with its result and
  trace.

Callers may restrict the covered input domain with backend-neutral constraints
over symbolic input leaves. Path conditions include those assumptions, and
`Complete` then means complete exactly within that declared domain. Callers may
also set path and solver budgets; hitting one changes the result to
`Incomplete`.

### Path and choice semantics

XLS IR has no control-flow graph. It evaluates a dataflow graph eagerly and
expresses control-like behavior through selection operations. A v1 path is
therefore a canonical partial valuation of declared IR choice sites in the
demanded dynamic slice. It is not an ordered sequence of executed basic blocks
and does not claim to reconstruct DSLX source control flow.

The default v1 choice policy is:

- `sel`: enumerate every feasible selected case and default outcome;
- `priority_sel`: enumerate every feasible priority-resolved case and default
  outcome, with each guard excluding all higher-priority cases;
- `one_hot_sel`: enumerate every feasible selected-case bitmask unless the
  selector is constrained or established to be one-hot, in which case enumerate
  the feasible one-hot outcomes; and
- nested choices: omit a choice from the trace when an outer outcome makes it
  structurally inactive.

Dynamic array indices, dynamic bit-slice positions, shift amounts, and similar
data selectors are not path sites under the default policy. Their semantics and
boundary values remain subject to differential testing and data-domain
coverage. Optional policies may expose them as additional coverage dimensions
without changing the default meaning of a v1 path.

An exhaustive mode never silently merges one of the declared choice sites to
control path growth. It either completes the declared enumeration or reports an
incomplete result.

### Canonical selection traces

A selection trace is a sparse map from choice-node identity to outcome:

\[
\tau : \mathit{ChoiceNodeId} \rightharpoonup \mathit{Outcome}.
\]

It is a map, not a temporal sequence. Its identity is tied to the exact IR
function, optimization state, node identities, concrete/symbolic input
partition, demanded root, and path policy. It need not remain stable when any of
those inputs change.

Every demanded declared choice appears in the map. A concrete choice contributes
one outcome without forking; a symbolic choice contributes one outcome to each
feasible fork. A structurally inactive choice contributes no entry.

For nested selections:

```text
outer = if x { inner } else { c }
inner = if y { a } else { b }
```

the canonical traces are:

```text
{outer: else}
{outer: then, inner: else}
{outer: then, inner: then}
```

When `outer: else` is selected, `inner` is structurally inactive. Its absence
does not claim that every arbitrary value of its selector is feasible; it says
only that the path neither observes nor constrains that choice.

Structural inactivity is distinct from semantic irrelevance. A demanded select
whose cases happen to compute equal values remains an active choice in v1.
Recognizing and collapsing semantically irrelevant active choices is an
optional optimization beyond v1.

### Demand semantics

For a fixed path:

1. Demand the function return node.
2. Ordinary operations demand their operands.
3. A concretely resolved select demands its selector and selected case only.
4. A split symbolic select demands its selector and selected case for that path.
5. Unselected cases are not demanded.
6. A shared node is active if any demanded use reaches it.

Selectors needed to compute another selector are part of the demanded cone and
may themselves contribute choices. This definition gives traces a precise
operational meaning without depending on graph scheduling order.

### Symbolic representation and solver boundary

Symbolic bits and Boolean constraints use a typed, backend-neutral, interned
expression DAG rather than solver-owned objects or raw SMT-LIB strings as their
fundamental representation. Here, *backend* means a solver such as Z3,
Bitwuzla, or Boolector.

Solver adapters lower the common expression language to a backend and translate
models back into typed XLS values. V1 requires one complete solver adapter and
model-conversion path. Multiple solver backends are not a v1 requirement.

The backend-neutral representation provides expression sharing, concrete
folding, simplification, deterministic serialization,
merged-versus-enumerated comparison, and a stable boundary for future solver
changes.

Enumeration also reports expression size, evaluated-node and memoization
counts, concrete and symbolic choice counts, solver-query outcomes, and
construction and solver wall time. These measurements make performance limits
observable without changing completeness semantics.

### Role of merged evaluation

Merged evaluation compactly preserves symbolic selections as expressions such
as `ite`. It is supporting infrastructure for v1 rather than the primary
product:

- it represents the whole function without path duplication;
- it provides an independent comparison target for the piecewise enumerated
  result;
- it enables whole-function equivalence checking against XLS; and
- it retains symbolic data operations within an enumerated path.

A separately stabilized public merged-mode API is optional for v1. The complete
path-enumeration API is not optional.

## System boundary

The intended stack is:

```text
DSLX or textual XLS IR
          |
          | xlsynth: parse, typecheck, lower, optimize, concrete replay
          v
      textual XLS IR
          |
          | xlsynth-pir: native Rust representation and traversal
          v
  xlsynth-symex evaluator
          |
          +-- concrete and symbolic structured values
          +-- backend-neutral interned expression DAG
          +-- complete path enumeration and canonical traces
          +-- solver adapter and feasibility checks
          +-- model conversion to XLS values
```

`xlsynth` remains the authoritative boundary for compilation and concrete
replay. `xlsynth-pir` is the evaluator's native traversal layer, but it is a
partial function-focused IR. Its coverage is measured and representational
gaps are handled explicitly rather than assumed away.

The symbolic value and evaluator layers do not expose processor or instruction
concepts. A state transition is simply one possible pure XLS function.

## Deliberate exclusions

The following are outside the repository's v1 boundary:

- procs and blocks;
- channels, `send`, and `receive`;
- tokens and effect-ordering operations;
- clocks, schedules, pipeline timing, registers, and implicit persistent state;
- instantiations and block ports;
- a general or unbounded memory model;
- cyclic or nonterminating recursion;
- instruction fetch, ISA dispatch, and instruction-sequence exploration;
- ISA-specific conventions such as program counters, traps, and privilege; and
- stable reconstruction of DSLX source paths from optimized IR.

Token-consuming diagnostics such as assertions, traces, and covers are outside
the pure-value contract. Treating assertions as input constraints would require
an explicit semantic design rather than discarding their effects.

An XLS array is an ordinary finite value. It does not imply an SMT or processor
memory model. Explicit architectural state encoded in input and output values
is supported; hidden state is not.

Whole-machine symbolic execution, proc execution, unbounded memory semantics,
and hardware timing remain downstream or separate-project concerns after v1 as
well; they are not deferred obligations for this repository.
