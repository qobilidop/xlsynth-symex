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

See:

- [Design](docs/design.md) for the current, authoritative design.
- [Research](docs/research.md) for related work and evaluation corpora.
- [Initial discussion notes](docs/notes/2026-08-01-initial-design-discussion.md)
  for the temporary historical record of the project's formative discussion.
