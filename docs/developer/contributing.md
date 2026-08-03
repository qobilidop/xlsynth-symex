# Contributing

This document is for people changing `xlsynth-symex`. It owns the development
workflow, repository conventions, and evidence expected with each kind of
change. Architecture belongs in [`design.md`](design.md), while the complete
validation argument belongs in [`verification.md`](verification.md).

## Development environment

Run every repository command through `./dev.sh`. The wrapper does one thing: it
runs the supplied command and arguments in the AMD64 development container with
the repository, Cargo cache, and target directory mounted. Use `./dev.sh bash`
for an interactive shell.

The complete local check is:

```text
./dev.sh ./scripts/check.sh
```

It checks formatting, Clippy with warnings denied, all tests, and rustdoc. Useful
shorter loops include:

```text
./dev.sh cargo fmt --all -- --check
./dev.sh cargo clippy --workspace --all-targets -- -D warnings
./dev.sh cargo test --workspace
./dev.sh cargo test --test selection_enumeration
```

Build the complete documentation site independently with:

```text
./dev.sh ./scripts/check-docs.sh
```

The command validates repository-relative Markdown links, builds mdBook and
rustdoc with their checked configuration, and writes the same `target/site`
tree deployed by CI. Open `target/site/index.html` for a local preview. mdBook
is version-pinned in the development image; do not install an unrelated host
copy for repository checks.

The default image is `ghcr.io/qobilidop/xlsynth-symex/dev:main`; Docker pulls it
automatically when it is absent. To test a Dockerfile change, build an image
directly and select it through the wrapper's single configuration variable:

```text
docker build --file .devcontainer/Dockerfile --platform linux/amd64 \
  --tag xlsynth-symex-dev:local .
XLSYNTH_SYMEX_DEV_IMAGE=xlsynth-symex-dev:local ./dev.sh ./scripts/check.sh
```

Image building, pulling, and publishing deliberately remain outside `dev.sh`.

Run the complete check before committing. Use `git diff --check` for prose-only
changes as well.

## Repository map

- `src/evaluator.rs`: merged symbolic evaluation and pure operation semantics.
- `src/enumerator.rs`: selection enumeration, trace policy, solving, and witnesses.
- `src/expr.rs`: typed interned bit-vector expression DAG and SMT rendering.
- `src/solver.rs`: expression lowering and the persistent upstream-solver session.
- `src/lib.rs`: public types, options, results, and entry points.
- `tests/`: semantic, differential, selection, mutation, corpus, and release checks.
- `tests/corpus/curated/`: pinned upstream fixtures and executable manifests.
- `docs/user/`: public behavior and the checked support inventory.
- `docs/developer/`: architecture, contribution, verification, and integration
  guidance.

## Engineering conventions

The code is intended for eventual review in `xlsynth-crate`:

- reusable behavior belongs in the library; examples remain thin consumers;
- library code does not print or depend on ambient logging for correctness;
- public ordering and serialization are deterministic;
- bits behavior supports arbitrary XLS widths rather than host-integer widths;
- maintained source files carry Apache-2.0 SPDX identifiers;
- unsafe code, compiler warnings, and missing public documentation are denied;
- public failure and completeness contracts are explicit; and
- comments explain invariants, semantics, or non-obvious policy rather than
  translating the code into prose.

Preserve the distinction between a hard error and incomplete enumeration. Bad
IR, invalid API inputs, and inconsistent internal state are errors. Resource
limits and solver indeterminacy produce an explicit incomplete result when
useful guarded results can still be returned.

## Changing operation semantics

When adding or modifying an XLS value operation:

1. Confirm its exact semantics in the pinned XLS version.
2. Implement it through the common typed value/expression layer, including
   concrete folding where useful.
3. Exercise concrete, symbolic, and mixed inputs at representative widths and
   structural shapes.
4. Compare with the XLS interpreter or JIT as the independent value oracle.
5. Add or update the operation row and executable coverage name in
   [`../user/support-matrix.md`](../user/support-matrix.md).
6. Promote any minimized failure to a permanent regression.

Do not classify an in-scope operation as unsupported merely because
`xlsynth-pir` cannot currently represent it. Extend or explicitly bridge the IR
layer, or document a genuine pure-function scope exclusion.

## Changing selection semantics

Changes to selection handling must preserve more than value equality. Add tests
for exact guards, canonical trace identities, structural inactivity,
feasibility pruning, deterministic order, and witness replay. When applicable,
extend exhaustive concrete trace-set comparison and the mutation harness so an
omitted, duplicated, relabeled, or incorrectly activated selection is detected.

Update the public contract in [`../user/guide.md`](../user/guide.md) and the
internal mechanism in [`design.md`](design.md) in the same change. A new limit
or approximation must be visible through completeness; it must never silently
weaken full-coverage semantics.

## Changing the curated corpus

Follow the directory-local
[`tests/corpus/curated/README.md`](https://github.com/qobilidop/xlsynth-symex/blob/main/tests/corpus/curated/README.md).
Fixtures retain upstream notices and provenance, tests remain offline, and
manifests pin the source revision, function, features, stable seeds, and
budgets. Both optimized and unoptimized IR forms are tracked independently.

External reference-translator failures are recorded as named blockers, not as
passes or candidate defects. Retain independent interpreter differential tests
and other applicable evidence.

## Documentation ownership

Keep each fact in one place:

- public semantics and limitations: `docs/user/guide.md`;
- supported operations and executable targets: `docs/user/support-matrix.md`;
- internal architecture and rationale: `docs/developer/design.md`;
- validation claims and release evidence: `docs/developer/verification.md`;
- upstream integration: `docs/developer/upstreaming.md`.

The README is a landing page, not a status report. Temporary implementation
state belongs in issues and pull requests. Once a decision is reflected in its
owning document, rely on Git history rather than adding permanent working notes
or an archive directory.

`docs/SUMMARY.md` owns the published sidebar and must include every narrative
chapter. Files linked from published chapters must either live under `docs/` or
use an explicit web URL; repository-relative links that escape the mdBook source
tree would be broken on GitHub Pages. CI publishes the combined mdBook and
rustdoc site only after the complete check succeeds on `main`.

After moving or deleting documentation, search the whole repository for stale
links and run the complete project check; tests consume the support and
validation manifests directly.
