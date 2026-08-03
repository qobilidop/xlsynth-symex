# xlsynth-symex

Symbolic evaluation and exhaustive path generation for pure
[XLS](https://github.com/google/xls) functions in Rust, built around the
[xlsynth](https://github.com/xlsynth/xlsynth-crate) ecosystem.

```text
pure XLS function + concrete/symbolic arguments
    -> every feasible canonical path
       + constraints + symbolic results + selection traces
```

The library is scoped to finite pure functions over bits, tuples, and fixed-size
arrays. Stateful XLS constructs, hardware timing, unbounded memory, and
whole-machine symbolic execution are outside the project boundary.

## Development

Run project commands in the checked-in development container:

```text
./dev.sh cargo test --workspace
./dev.sh cargo fmt --all -- --check
./dev.sh cargo clippy --workspace --all-targets -- -D warnings
```

Run `./dev.sh` without a command for an interactive shell. The wrapper uses the
checked-in development environment shared with CI.

## Documentation

- [V1 design](docs/design.md): the normative end state and project boundary.
- [Verification](docs/verification.md): the evidence required to call v1 done.
- [Status](docs/status.md): the current implementation and validation snapshot.
- [Roadmap](docs/roadmap.md): the path to v1 and optional work beyond it.
- [Research](docs/research.md): prior art and evaluation sources.
- [Historical notes](docs/notes/): superseded design discussion and prototype
  records.
