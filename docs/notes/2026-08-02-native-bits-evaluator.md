# Native bits evaluator implementation note

Recorded: 2026-08-02 02:03 PDT (America/Los_Angeles)

This note records expedient choices made while replacing the initial XLS SMT
adapter with the first native evaluator. `docs/design.md` remains authoritative.

## Traversal boundary

The implementation uses the `xlsynth-pir` crate at the same pinned
`xlsynth-crate` revision as `xlsynth`. `IrFunction::to_ir_string` supports leaf
evaluation, while package evaluation parses `IrPackage::to_string` so invoked
pure functions are available recursively. This avoids writing a second XLS IR
parser and keeps the repository focused on symbolic semantics.

## Expression representation

The first native value is a typed bits expression containing its width and an
SMT-LIB string. This was chosen to reach an independent equivalence boundary
quickly and expose missing XLS operations through the curated corpus. It is not
the intended long-term representation: repeated subexpressions are duplicated,
SMT syntax is embedded in evaluation, and solver-independent rewriting is
difficult.

A later refactor should replace strings with an interned typed expression DAG
and move SMT rendering into a solver adapter. The public result already
separates parameters and the result value, which leaves room for that change.

## First supported slice

The evaluator currently handles bits parameters and results, literals, add,
subtract, multiply, Boolean n-ary operations, negate, not, identity, signed and
unsigned comparisons, shifts, zero/sign extension, concatenation, static and
dynamic slicing, merged selects, and pure invokes.

The upstream `riscv_simple::decode_opcode` function immediately exposed that
DSLX constant slicing can remain a `dynamic_bit_slice` in unoptimized IR. Its
semantics were implemented as a logical right shift followed by low-bit
extraction. Curated-vector, differential-fuzz, and whole-function equivalence
checks pass for the initial corpus in both IR forms.

## Deferred work

Concrete/symbolic mixed values, tuples, arrays, expression interning, explicit
paths, and selection traces remain. Zero-width bit vectors are rejected because
SMT-LIB bit vectors require positive widths; a structural value representation
should decide how to carry XLS zero-width values without presenting them to the
solver.
