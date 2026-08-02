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
for optimized and unoptimized `find_index` IR. The equivalence harness rebuilds
the finite SMT array from native bits leaves and compares both tuple projections,
so the optimized form is proved against the independent XLS translation. The
unoptimized form is explicitly blocked because that translator rejects its
`counted_for`. Path-witness replay remains blocked on selection traces,
consistently with the rest of the corpus.
