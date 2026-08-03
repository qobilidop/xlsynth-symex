# xlsynth-symex documentation

`xlsynth-symex` symbolically evaluates finite, pure XLS functions and enumerates
every feasible canonical IR selection path with its condition, residual result,
selection trace, and concrete witness.

```text
pure XLS function + concrete/symbolic inputs
    -> every feasible canonical path
       + condition + result + selection trace + concrete witness
```

Start with the [user guide](user/guide.md) to understand the public API, path
semantics, completeness contract, supported domain, and performance limits. The
[operation support matrix](user/support-matrix.md) is the checked inventory for
the pinned XLS toolchain.

Contributors and reviewers should continue with the
[design](developer/design.md), [contribution guide](developer/contributing.md),
[verification argument](developer/verification.md), and
[upstreaming notes](developer/upstreaming.md).

The [API reference](https://qobilidop.github.io/xlsynth-symex/api/xlsynth_symex/)
is generated from the current `main` branch. Released crate versions use their
versioned API documentation on [docs.rs](https://docs.rs/xlsynth-symex).
