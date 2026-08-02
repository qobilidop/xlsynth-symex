# 2026-08-02 02:25 PDT: structured array values

The upstream `find_index.x` example was added to make fixed-size arrays part of
the executable corpus rather than an aspirational milestone. Its unoptimized
IR requires array construction, symbolic one-dimensional indexing and update,
structural selection, bounded `counted_for`, `one_hot`, and `encode`. Supporting
that complete chain lets the native evaluator accept recursively structured
parameters and results and expose their bits leaves to test and solver clients.

Arrays remain finite structural values. A symbolic index is lowered to a
merged selection over elements, and an update is lowered to one equality-guarded
selection per element. This matches the project boundary: XLS arrays are values,
not an unbounded SMT memory model. It is also deliberately simple; expression
sharing and a backend-neutral DAG should address duplication before larger
arrays become routine corpus inputs.

Both directed differential testing and differential fuzz testing are required
for optimized and unoptimized `find_index` IR. Symbolic equivalence is recorded
as `blocked:structured-symbolic-interface`: the native evaluator flattens bits
leaves, while the independent XLS SMT translation retains structured sorts, and
the harness does not yet reconstruct a common function signature. This is an
interface limitation rather than evidence about equivalence. Path-witness replay
remains blocked on selection traces, consistently with the rest of the corpus.
