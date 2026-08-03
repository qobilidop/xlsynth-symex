# xlsynth-symex

Symbolic evaluation and exhaustive path generation for finite, pure
[XLS](https://github.com/google/xls) functions in Rust, built around the
[xlsynth](https://github.com/xlsynth/xlsynth-crate) ecosystem.

```text
pure XLS function + concrete/symbolic inputs
    -> every feasible canonical path
       + condition + result + selection trace + concrete witness
```

The library supports bits, tuples, fixed-size arrays, pure calls, and bounded
iteration. Stateful XLS constructs, hardware timing, unbounded memory, and
whole-machine symbolic execution are outside its scope.

Run the complete example in the checked-in development environment:

```text
./dev.sh cargo run --example enumerate
```

Always inspect `EnumerationResult::completeness` before treating returned paths
as full coverage.

## Documentation

For library users:

- [User guide](docs/user/guide.md): usage, path semantics, completeness,
  constraints, limitations, and performance characteristics.
- [Support matrix](docs/user/support-matrix.md): the checked operation inventory
  for the pinned XLS toolchain.
- [Rust API documentation](https://docs.rs/xlsynth-symex): item-level API
  reference once the crate is published. Until then, run
  `./dev.sh cargo doc --no-deps --open`.

For contributors and reviewers:

- [Design](docs/developer/design.md): architecture, invariants, and rationale.
- [Contributing](docs/developer/contributing.md): development workflow and
  change requirements.
- [Verification](docs/developer/verification.md): correctness, completeness,
  coverage, and release evidence.
- [Upstreaming](docs/developer/upstreaming.md): `xlsynth-crate` fit and
  integration considerations.
