# Curated XLS corpus

This directory contains pure DSLX functions copied from the upstream XLS
examples corpus. `manifest.tsv` is the machine-readable source of provenance
and test configuration. Its revision is the commit tagged `v0.54.3` in
`xlsynth/xlsynth`, matching the XLS runtime release selected by the pinned
`xlsynth-crate` dependency.

The fixtures retain their upstream Apache-2.0 notices. To refresh or add a
fixture:

1. select a pure function that lowers to XLS IR;
2. copy it without modification from a commit compatible with the pinned XLS
   runtime;
3. add its provenance and feature requirements to `manifest.tsv`;
4. add deterministic input samples to `tests/curated_corpus.rs`; and
5. run the full development checks documented in the repository README.

Tests do not access the network. Both optimized and unoptimized IR are checked,
and failures identify the corpus entry, optimization mode, and concrete sample.
