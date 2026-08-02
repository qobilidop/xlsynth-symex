# Bounded `counted_for` evaluation

Recorded: 2026-08-02 02:15 PDT (America/Los_Angeles)

The upstream LFSR example was added to test the same function before and after
loop unrolling. Its unoptimized IR contains a `counted_for`; optimized IR is a
straight-line graph.

The evaluator implements the XLS operation by repeatedly calling the loop body
with a concrete symbolic induction literal, the previous symbolic carry, and
the invariant arguments. Trip count and stride are IR attributes, so this is a
finite symbolic construction rather than unbounded symbolic execution. Both IR
forms now pass directed differential and differential fuzz testing. Optimized
IR also proves equivalent to XLS; the XLS reference SMT translator does not
support the unoptimized `counted_for`, which is recorded as a reference-side
blocker.

The first fuzz implementation embedded the entire unrolled expression once per
concrete case. The 2,048-case LFSR run exposed the resulting query blowup. The
harness now defines the candidate expression once as an SMT function and calls
it for each concrete vector. This brought the run back to ordinary CI latency
and is an early example of why expression sharing must remain visible even
before the interned DAG refactor.
