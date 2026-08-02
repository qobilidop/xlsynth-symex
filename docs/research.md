# Research and prior art

This document collects projects and concepts relevant to `xlsynth-symex`. It is
an annotated starting point rather than a comprehensive literature review.

## XLS and xlsynth

- [Google XLS](https://github.com/google/xls) is the upstream C++ high-level
  synthesis toolchain. Its pure functions are SSA-like dataflow graphs rather
  than control-flow graphs.
- [XLS IR semantics](https://google.github.io/xls/ir_semantics/) specifies
  operations including `sel`, `priority_sel`, and `one_hot_sel`, and explicitly
  describes eager evaluation of selection cases.
- [XLS solvers](https://google.github.io/xls/solvers/) translate XLS IR into SMT
  bit-vector formulas for property checking, equivalence, and counterexample
  generation. The implementation includes Z3 and Bitwuzla translators under
  [`xls/solvers`](https://github.com/google/xls/tree/main/xls/solvers).
- The XLS [abstract evaluator](https://google.github.io/xls/adding_ir_operation/#10-abstract-evaluator)
  parameterizes IR evaluation over alternative value domains and is used for
  ternary reasoning and solver translation. It is conceptually close to the
  evaluator architecture proposed here.
- [xlsynth-crate](https://github.com/xlsynth/xlsynth-crate) wraps the shared XLS
  library for Rust. The `xlsynth` crate provides compilation, IR objects,
  interpretation, JIT execution, and SMT-LIB emission.
- [`xlsynth-pir`](https://github.com/xlsynth/xlsynth-crate/tree/main/xlsynth-pir)
  is a partial native-Rust IR focused on functions. It already contains parsing,
  evaluation, rewriting, inlining, range analysis, cone extraction, and node
  environments.
- [`xlsynth-prover`](https://github.com/xlsynth/xlsynth-crate/tree/main/xlsynth-prover)
  contains Rust formal workflows and integrations with Z3, Bitwuzla, and
  Boolector.

## ISA semantics and symbolic execution

- [Sail](https://github.com/rems-project/sail) is a language for executable ISA
  specifications. It generates emulators and theorem-prover definitions and is
  used for architectures including Arm, RISC-V, CHERI, and MIPS.
- [Isla](https://github.com/rems-project/isla) symbolically evaluates Sail
  models. Its architectural state, guarded events, path constraints, and
  instruction-sequence exploration motivate the downstream use of lifted XLS
  instruction semantics. `xlsynth-symex` itself deliberately stops at one pure
  function invocation.
- [Instruction-Level Abstraction](https://arxiv.org/abs/1801.01114) generalizes
  ISA-style state-transition specifications to accelerators. Its decode guards,
  architectural state, and per-command update functions are a useful model for
  register-state hardware without a general memory.

## Register and transition machines

- Classical [register machines](https://mathworld.wolfram.com/RegisterMachine.html)
  use unbounded natural-number registers and tiny instruction sets as models of
  computability. The name is potentially misleading for fixed-width hardware.
- Register automata store values from an infinite alphabet and traditionally
  permit equality comparisons and assignment rather than general bit-vector
  arithmetic. They are also not a direct match.
- More applicable terms are *extended finite-state machine*, *guarded symbolic
  transition system*, *word-level transition system*, and, where commands are
  central, *instruction-level abstraction*.

## Paths in predicated and hardware representations

- Traditional dynamic symbolic execution records an ordered sequence of CFG
  branch decisions. XLS IR has no corresponding execution order.
- [Dataflow predication](https://www.microsoft.com/en-us/research/publication/dataflow-predication/)
  associates dataflow computation with predicates and distinguishes active from
  falsely predicated work. It motivates activity-aware selection traces.
- RTL verification uses branch, condition, expression, toggle, FSM, and cross
  coverage. Mux-choice and cross coverage provide practical analogues for XLS
  selection traces when full path combinations are too numerous.
- [Piecewise composition for hardware symbolic execution](https://arxiv.org/abs/2304.05445)
  demonstrates the importance of merging symbolic hardware paths and delegating
  parts of exploration to SMT rather than eagerly enumerating every path.
- The project therefore uses a *selection trace*: a partial map from demanded IR
  choice sites to outcomes. An absent choice is inactive, analogous to a
  don't-care in a Boolean cube or masked valuation.

## Candidate evaluation corpora

No single maintained corpus is tailored to pure-function XLS symbolic
evaluation. The following sources are complementary.

### Curated upstream examples

[`xls/examples`](https://github.com/google/xls/tree/main/xls/examples) contains
human-readable pure functions including:

- tiny adders and overflow detection;
- nested selects;
- prefix sums and bitonic sort;
- dot products, FIR filters, and matrix multiplication;
- Adler-32, CRC32, and SHA-256;
- GCD and cubic Bezier evaluation;
- floating-point conversion and arithmetic; and
- [`riscv_simple.x`](https://github.com/google/xls/blob/main/xls/examples/riscv_simple.x),
  whose decode and instruction arithmetic are especially relevant to concrete
  instruction plus symbolic state evaluation.

Stateful examples and functions that cannot lower to IR must be filtered out.

### DSLX standard library

[`xls/dslx/stdlib`](https://github.com/google/xls/tree/main/xls/dslx/stdlib)
contains fixed- and floating-point routines, rounding, bit-vector utilities, and
parametric functions. Selected concrete instantiations can stress arbitrary
widths, tuples, arrays, multiplication, division, and selection-heavy logic.

### Generated and adversarial programs

- The [XLS fuzzer](https://google.github.io/xls/fuzzer/) generates random DSLX
  functions and inputs and supports deterministic seeds. Fixed versions,
  options, seeds, and size bounds can form a reproducible broad corpus.
- [`xls/fuzzer/crashers`](https://github.com/google/xls/tree/main/xls/fuzzer/crashers)
  contains minimized historical failures with unusual operation combinations.
  These are valuable robustness cases but less useful as headline benchmarks.

## Validation implications

The strongest practical guardrail combines independently valuable methods:

- exhaustive differential evaluation for small input spaces;
- structured and random differential fuzzing against the XLS interpreter/JIT;
- SMT-generated witnesses for enumerated selection paths;
- concrete replay of symbolic values and selection traces; and
- whole-function symbolic equivalence against the existing XLS SMT translator.

Path coverage and value correctness are distinct. One model per path can expose
rare control flow but does not test all arithmetic within the path. Conversely,
random fuzzing exercises values but can miss rare guards. The methods should be
reported separately and used together.
