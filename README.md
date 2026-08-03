# xlsynth-symex

Symbolic evaluation and exhaustive selection enumeration for finite, pure
[XLS](https://github.com/google/xls) functions in Rust, built around the
[xlsynth](https://github.com/xlsynth/xlsynth-crate) ecosystem.

```text
pure XLS function + concrete/symbolic inputs
    -> complete selection partition
       + canonical trace + guard + residual result + concrete witness
```

The library supports bits, tuples, fixed-size arrays, pure calls, and bounded
iteration. Stateful XLS constructs, hardware timing, unbounded memory, and
whole-machine symbolic execution are outside its scope.

Run the complete example in the checked-in development environment:

```text
./dev.sh cargo run --example enumerate
```

Always inspect `EnumerationResult::completeness` before treating returned
guarded results as full coverage.

## Documentation

The [documentation site](https://qobilidop.github.io/xlsynth-symex/) combines
the narrative guides with API documentation generated from the current `main`
branch. Its source is organized by audience below.

For library users:

- [User guide](docs/user/guide.md): usage, selection semantics, completeness,
  constraints, limitations, and performance characteristics.
- [Support matrix](docs/user/support-matrix.md): the checked operation inventory
  for the pinned XLS toolchain.
- [API reference](https://qobilidop.github.io/xlsynth-symex/api/xlsynth_symex/):
  item-level documentation for the current `main` branch.

For contributors and reviewers:

- [Design](docs/developer/design.md): architecture, invariants, and rationale.
- [Contributing](docs/developer/contributing.md): development workflow and
  change requirements.
- [Verification](docs/developer/verification.md): correctness, completeness,
  coverage, and release evidence.
- [Upstreaming](docs/developer/upstreaming.md): `xlsynth-crate` fit and
  integration considerations.

Build the complete site locally with `./dev.sh ./scripts/check-docs.sh`; output
is written to `target/site`.
