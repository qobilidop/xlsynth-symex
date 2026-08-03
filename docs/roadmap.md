# Roadmap

This document records the dependency order used to build the v1 review
candidate described by [`design.md`](design.md). The stages are an
implementation record, not a claim that v1 has been tagged or released.
Measured candidate state belongs in [`status.md`](status.md), and the enduring
release gate belongs in [`verification.md`](verification.md).

Work proceeds in thin, independently validated vertical slices. Each stage
keeps the XLS interpreter or JIT as an independent semantic oracle and updates
the status and validation reports when observed capabilities change.

## 1. Freeze the v1 inventory

- Generate the operation/type inventory for the pinned XLS and `xlsynth-pir`
  revisions.
- Check in the normative support matrix and explicit exclusions.
- Turn the path policy and complete/incomplete outcome into public API tests.
- Add minimal fixtures for `sel`, `priority_sel`, one-hot and non-one-hot
  `one_hot_sel`, nested inactivity, and infeasible outcomes.

## 2. Establish the symbolic core

- Replace raw expression strings with typed, interned expression and Boolean
  constraint DAGs.
- Add concrete bits and recursively mixed concrete/symbolic values.
- Preserve concrete folding and existing merged semantics.
- Add one solver adapter and typed model conversion.
- Differentially validate each migrated operation before expanding coverage.

## 3. Make evaluation demand-driven

- Evaluate from demanded roots instead of visiting every node topologically.
- Memoize by function, environment, path, and demanded node as required.
- Resolve concrete selectors before demanding case cones.
- Measure visited nodes and concretely pruned choices on mixed-input fixtures.

## 4. Deliver the first complete path slice

- Split symbolic `sel` nodes into guarded path results.
- Define stable-within-artifact choice identities and sparse traces.
- Implement structural inactivity for nested choices.
- Prune infeasible paths with the solver.
- Return an explicit complete or incomplete enumeration outcome.
- Add witness replay and merged-versus-enumerated checking for this slice.

## 5. Complete v1 choice semantics

- Add exact priority guards for `priority_sel`.
- Add bitmask outcomes for unconstrained `one_hot_sel` and specialized one-hot
  behavior when the precondition is established.
- Canonicalize and deterministically order traces without merging active choices
  that merely compute equal values.
- Prove input-domain coverage and trace uniqueness for completed enumerations.
- Add exhaustive bounded trace comparison and enumeration mutation tests.

## 6. Close the pure-operation and type matrix

- Implement every remaining in-scope pure value operation, including structural
  and arbitrary-width cases.
- Extend the selected IR layer when an in-scope pinned operation cannot be
  represented.
- Add microtests and generated coverage for every matrix entry.
- Promote minimized fuzz or solver failures into permanent regressions.

## 7. Harden the corpus and release contract

- Expand curated functions, standard-library instantiations, deterministic
  generated graphs, and historical crashers.
- Exercise optimized/unoptimized IR and mixed argument partitions.
- Require path witnesses and complete enumeration for every applicable corpus
  row.
- Record construction time, solver time, expression sharing, path counts,
  pruned choices, visited nodes, and peak memory.
- Run the complete release gate in [`verification.md`](verification.md).
- Reconcile design, API docs, status, support matrix, and observed behavior as
  the v1 release gate.

## Beyond v1

V1 completes the repository's core promise. Later work is optional enhancement
or integration work:

- decide whether and how to upstream the evaluator, expression layer, path
  model, or supporting `xlsynth-pir` changes;
- add solver adapters such as Bitwuzla or Boolector;
- add known-bit propagation and stronger expression simplification;
- improve incremental, parallel, or portfolio solving and enumeration
  performance;
- add optional coverage policies for dynamic array indices, slices, shifts, and
  other data selectors;
- minimize active choices proven semantically irrelevant;
- add ahead-of-time specialization and caching for common concrete arguments;
- expand corpora, stress benchmarks, and long-running fuzz campaigns;
- improve function fingerprints and best-effort source correlation without a
  stability promise; and
- consider interpreting pure assertions as preconditions if a sound
  token/effect boundary is designed.

Whole-machine symbolic execution, proc execution, unbounded memory semantics,
and hardware timing remain downstream or separate-project concerns. They are
not post-v1 obligations for this repository.
