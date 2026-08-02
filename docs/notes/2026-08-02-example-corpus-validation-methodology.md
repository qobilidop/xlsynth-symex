# Example corpus validation methodology

Recorded: 2026-08-02 01:49 PDT (America/Los_Angeles)

This note records the discussion that established consistent names and roles
for the four semantic validation modes used by the curated XLS example corpus.
The current conclusions also live in `docs/design.md`, which remains
authoritative.

## Canonical names

1. **Curated-vector differential testing** compares the candidate evaluator
   with XLS on deliberately selected concrete inputs: upstream vectors,
   boundaries, regressions, and minimized fuzz failures.
2. **Differential fuzz testing** generates many deterministic concrete inputs
   and uses XLS as the differential oracle. Small finite domains should be
   exhausted instead of randomly sampled.
3. **Symbolic equivalence checking** asks whether an independently produced
   native symbolic result can differ from XLS's reference SMT translation for
   any modeled input.
4. **Path-witness replay** solves each enumerated symbolic path, replays the
   concrete witness, and checks both its result and canonical selection trace.

"Directed differential testing" was rejected as the first name because
"directed" is overloaded: it can refer to manually selected cases,
coverage-directed fuzzing, or solver-directed generation. "Curated-vector"
states the input source without implying a particular generation technique.
"Differential fuzz testing" treats differential comparison as the fuzzing
oracle and follows natural testing terminology.

## Capability status

Every example and IR form should state whether each validation is required,
blocked by a named missing capability, or not applicable. Blocked validations
must not be presented as passes or silently skipped.

At the time of this note, curated-vector differential testing and differential
fuzz testing are meaningful and required. Symbolic equivalence is blocked
until `xlsynth-symex` produces a native symbolic encoding independent of XLS's
reference translator. Path-witness replay is blocked until the evaluator emits
enumerated paths and canonical selection traces.

The intended report has one row per example function and IR form, with one
column per validation mode. Concrete differential cells include vector or case
counts; equivalence reports `UNSAT`, `SAT`, timeout, or unsupported; path replay
reports witnessed paths over reported feasible paths.
