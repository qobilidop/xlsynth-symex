# User guide

This guide is for Rust users of `xlsynth-symex`. It is the source of truth for
the library's public semantics: what it evaluates, what a path means, and when
an enumeration constitutes full coverage. Item-level signatures and fields
belong in the crate's Rust API documentation.

## When to use this library

Use `xlsynth-symex` when you have a finite, pure XLS function and need either:

- every feasible IR-level selection path and a concrete input reaching it; or
- one merged symbolic result for equivalence checking or further SMT work.

The primary use case is exhaustive test generation. Inputs may be concrete,
symbolic, or recursively mixed within tuples and fixed-size arrays. A common
downstream pattern evaluates a concrete instruction and symbolic state as one
pure transition function; composing transitions into a processor executor is a
separate concern.

This library is not an XLS proc simulator, an RTL timing model, or a
whole-program symbolic executor.

## First enumeration

The repository's supported development and verification environment is the
checked-in AMD64 container. It includes the required Z3 executable:

```text
./dev.sh cargo run --example enumerate
```

The example parses an `IrPackage`, calls `enumerate_package`, checks
`EnumerationResult::completeness`, and prints each trace and condition. The
complete source is [`../../examples/enumerate.rs`](../../examples/enumerate.rs).

The enumeration entry points form three families:

| Input boundary | Leaf function | Package function | Textual package IR |
|---|---|---|---|
| All symbolic | `enumerate` | `enumerate_package` | `enumerate_ir_package` |
| Mixed inputs | `enumerate_with_inputs` | `enumerate_package_with_inputs` | `enumerate_ir_package_with_inputs_and_options` |
| Options | `enumerate_with_options` | `enumerate_package_with_options` | `enumerate_ir_package_with_options` |

Combined mixed-input-and-options variants are available where applicable. Use
a package entry point whenever the selected function invokes another function;
a standalone `IrFunction` does not carry its callees.

## Reading a result

An `EnumerationResult` contains shared symbolic parameters, deterministically
ordered paths, explicit completeness, and construction/solver statistics. Each
`PathResult` contains:

- a feasible path condition, including caller assumptions;
- a residual symbolic XLS value;
- a sparse canonical selection trace; and
- a complete concrete XLS input witness that reaches the path.

The path condition and residual bits expressions can be rendered as SMT-LIB.
Tuples and arrays remain recursive XLS values whose bits leaves carry symbolic
expressions.

### Complete and incomplete results

`EnumerationCompleteness::Complete` means that every feasible canonical path
under the default path policy and caller constraints is present exactly once.
Together, the returned conditions cover the constrained symbolic-input domain.

`Incomplete(reason)` means the paths may still be useful, but they are not full
coverage. Reasons include a returned-path limit, the internal syntactic-branch
safety ceiling, and a failed, timed-out, or indeterminate solver query. Never
infer completeness from the path count or from a successful function return.

The default solver timeout is ten seconds per feasibility/model query. The
internal safety ceiling is 1,000,000 syntactic branches. `max_paths` limits the
number of paths returned; the current implementation constructs and solves
candidates before truncating the result, so it is not a computational budget.

## Inputs and assumptions

`EvaluationInput` describes every argument recursively:

- `Symbolic` makes every bits leaf below that value symbolic;
- `Concrete(IrValue)` supplies a fully concrete value; and
- `Tuple` and `Array` independently describe their elements.

Concrete choices are resolved without symbolic forking and do not demand
inactive case cones. They still appear in the trace when they are active, which
makes witness replay describe the same choice map as symbolic enumeration.

`EnumerationOptions::constraints` narrows the covered domain. Constraints are
backend-neutral expressions over `InputLeaf` identities: an argument index
followed by tuple or array element indices. They do not depend on generated Z3
names. When enumeration is complete, it is complete exactly within the domain
satisfying those assumptions.

## What a path means

XLS IR is an eager dataflow graph, not a control-flow graph. A path is therefore
a canonical partial valuation of declared selection nodes in the demanded
dynamic slice. It is not a sequence of executed basic blocks and is not a
stable reconstruction of DSLX source branches.

The default policy is:

- `sel`: one outcome for each feasible case or default;
- `priority_sel`: one outcome for each feasible priority-resolved case or
  default, excluding all higher-priority cases;
- `one_hot_sel`: one outcome for each feasible selected-case bitmask, or the
  feasible one-hot outcomes when a one-hot assumption is established; and
- nested choices: no trace entry when an outer outcome makes a choice
  structurally inactive.

A demanded choice remains active even if all of its cases happen to calculate
the same value. Dynamic array indices, bit-slice positions, shift amounts, and
similar data selectors are ordinary symbolic data under the default policy;
they are not additional path sites.

Choice identities include the function and XLS node id, plus callsite and loop
iteration context. They are deterministic for one exact evaluation artifact,
but are not stable across IR optimization, node renumbering, input partitions,
or path-policy changes.

## Supported semantic domain

The supported value domain is recursive fixed-size bits, tuples, and arrays,
including zero-width bits where XLS permits them. Pure calls and `counted_for`
with a static trip count are supported. The checked inventory is
[`support-matrix.md`](support-matrix.md); every supported row names an
executable coverage target.

The following remain deliberately outside the project boundary:

- procs, blocks, channels, tokens, and effect ordering;
- assertions, traces, covers, and other token-consuming diagnostics;
- clocks, schedules, pipeline timing, registers, and hidden persistent state;
- instantiations and block ports;
- general or unbounded memory;
- cyclic or nonterminating recursion;
- instruction fetch, ISA dispatch, and instruction-sequence exploration; and
- stable DSLX source-path reconstruction.

An XLS array is an ordinary finite value, not a solver or processor memory
model. Explicit state encoded in function arguments and results is supported.

## Merged evaluation

The `evaluate*` entry points preserve selections as symbolic expressions such
as `ite` and return one `SymexResult`. Merged evaluation is useful for compact
whole-function representation, symbolic equivalence checking, and retaining
symbolic data operations inside an enumerated path.

Merged evaluation does not provide explicit path coverage. Use `enumerate*`
when generating tests or making a claim about every feasible canonical path.

## Performance expectations

Symbolic construction is generally small compared with solver work. The
current adapter starts a fresh Z3 process for each candidate, so runtime grows
roughly with the number and difficulty of path queries, while selection
combinations can grow exponentially. Release-mode Rust optimization therefore
has little effect on solver-dominated workloads.

The executable v1 smoke test enumerates 64 `one_hot_sel` masks under a
30-second ceiling. This is a bounded release guard, not a general scalability
claim. Consult [`../developer/verification.md`](../developer/verification.md)
for the measured environment and evidence, and inspect
`EnumerationResult::statistics` for each real workload.

## Getting help or contributing

The Rust API documentation describes every public item. Architecture and
contribution guidance live under [`../developer/`](../developer/), beginning
with [`../developer/design.md`](../developer/design.md) and
[`../developer/contributing.md`](../developer/contributing.md).
