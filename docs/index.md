# xlsynth-symex documentation

`xlsynth-symex` symbolically evaluates finite, pure XLS functions and can
produce a complete partition of every feasible canonical IR selection trace,
with its guard, residual result, and concrete witness. Explicit completeness
status distinguishes full coverage from a useful partial result.

```text
pure XLS function + concrete/symbolic inputs
    -> selection partition or explicit incomplete status
       + canonical trace + guard + residual result + concrete witness
```

Start with the [user guide](user/guide.md) to understand the public API, selection
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
