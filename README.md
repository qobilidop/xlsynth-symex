# xlsynth-symex

Symbolic evaluation of pure [XLS](https://github.com/google/xls) functions in
Rust, built around the [xlsynth](https://github.com/xlsynth/xlsynth-crate)
ecosystem.

This project is in its initial design and prototyping phase. Its intended core
operation is:

```text
XLS function + concrete/symbolic arguments
    -> symbolic results + path conditions + selection traces
```

The initial scope is deliberately narrow: pure XLS IR functions over bits,
tuples, and fixed-size arrays. Stateful XLS constructs and whole-machine
symbolic execution are not part of the library.

## Development

Run development commands in the Ubuntu-based development container:

```text
./dev.sh cargo test --workspace
./dev.sh cargo fmt --all -- --check
./dev.sh cargo clippy --all-targets -- -D warnings
./dev.sh
```

`dev.sh` pulls `ghcr.io/qobilidop/xlsynth-symex/dev:main` on first use and
falls back to a local build if the image has not been published yet. Use
`./dev.sh --pull` to refresh it or `./dev.sh --build` to rebuild it locally.
Set `XLSYNTH_SYMEX_DEV_IMAGE` to select another registry or immutable SHA tag.

Editors supporting the Development Containers specification can use
`.devcontainer/devcontainer.json` directly. GitHub Actions validates the image
on pull requests and publishes it after relevant changes land on `main`.

## Current implementation

The first symbolic-evaluation milestone is a deliberately minimal vertical
slice. `xlsynth_symex::evaluate` returns one unconditional path and delegates
the merged SMT-LIB result to XLS's Z3 translator. Tests compare this adapter
with the upstream SMT output and run deterministic generated IR programs
against both Z3 and the XLS interpreter. An offline curated corpus also compiles
pinned upstream XLS examples and checks selected pure functions in optimized
and unoptimized forms. See `tests/corpus/curated/README.md` for its provenance
and extension workflow. The corpus uses curated-vector differential testing and
deterministic differential fuzz testing today, while explicitly tracking the
native-evaluator prerequisites for symbolic equivalence checking and
path-witness replay.

See:

- [Design](docs/design.md) for the current, authoritative design.
- [Research](docs/research.md) for related work and evaluation corpora.
- [Initial discussion notes](docs/notes/2026-08-01-initial-design-discussion.md)
  for the temporary historical record of the project's formative discussion.
