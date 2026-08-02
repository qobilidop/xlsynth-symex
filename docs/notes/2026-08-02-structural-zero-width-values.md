# Structural tuples and zero-width values

Recorded: 2026-08-02 02:10 PDT (America/Los_Angeles)

The `overflow_detect` upstream example was added to force the evaluator beyond
a bits-only internal value. Its standard-library callees construct tuple
results, immediately index those tuples, use reductions, and contain many
zero-width intermediates produced by parametric bit extraction.

## Expedient representation

Internal symbolic values are now structural: bits or tuples of values. Tuple
construction and indexing do not require an SMT tuple sort when the enclosing
function ultimately returns bits; invokes pass the structure recursively and
the selected leaves become ordinary SMT expressions.

XLS permits `bits[0]`, while SMT-LIB bit-vector sorts require positive widths.
The evaluator therefore carries zero-width bits as a structural bits value with
no SMT expression. Operations with mathematically determined behavior eliminate
it: concatenation ignores zero-width operands, extension produces a positive-
width zero, a zero-width select remains structural, and reductions use their
identity values. A future typed expression DAG should represent this choice
explicitly instead of relying on an empty internal rendering string.

## Corpus discoveries

The optimized overflow function uses multiplication whose result width differs
from its operand widths. SMT multiplication requires equal operand and result
widths, so unsigned or signed operands are resized to the XLS result width
before `bvmul`.

All directed and fuzz differential checks pass in both IR forms. Symbolic
equivalence passes for optimized IR. XLS's own Z3 translator aborts on the
unoptimized zero-width-heavy standard-library graph, so that single reference
check is recorded as `blocked:xls-reference-translator` rather than skipped or
misreported as a candidate failure.
