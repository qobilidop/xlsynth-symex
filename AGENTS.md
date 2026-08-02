# Project continuity notes

## Repository identity

- The project is named `xlsynth-symex`.
- On 2026-08-01, it was renamed from `xls-symex`.
- GitHub repository: `qobilidop/xlsynth-symex`.
- Expected local checkout: `/Users/qobilidop/i/xlsynth-symex`.
- The old name may appear when documenting the rename itself; do not treat every
  historical occurrence as stale.

## Where to recover context

- `docs/design.md` is the living, authoritative design document.
- `docs/research.md` records relevant prior art and candidate test corpora.
- `docs/notes/2026-08-01-initial-design-discussion.md` is a temporary historical
  record of the initial rapid-prototyping discussion.
- Keep current conclusions in `docs/design.md`; do not make this file a second
  design document.

## Current project boundary

`xlsynth-symex` is intended to be a Rust symbolic evaluator for the pure-function
subset of XLS, built around the `xlsynth` ecosystem. Whole-processor symbolic
execution is a motivating downstream application, not part of this repository's
scope.

## Collaboration convention

For commits materially co-authored with Codex, add this trailer unless the user
requests a different attribution:

```text
Co-Authored-By: Codex GPT-5.6 Sol <codex@openai.com>
```

