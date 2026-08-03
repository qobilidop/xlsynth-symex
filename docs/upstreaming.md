# Upstreaming readiness

This repository is developed as a standalone proving ground for a prospective
`xlsynth-crate` library component. The engineering baseline is
`xlsynth/xlsynth-crate` commit
`92bc9b932981c776bb4bb197cd6b6726f17ec090`, which is also the exact revision
used by the crate dependencies.

## Conventions adopted here

- Reusable behavior lives in a library with typed public APIs; the example is a
  thin consumer.
- Library code does not print. Solver output is captured and converted into
  results or errors.
- Observable maps and path ordering are deterministic.
- Symbolic bit operations use arbitrary-width `IrBits` and `IrValue` values.
- Maintained source files carry Apache-2.0 SPDX identifiers, enforced by tests.
- Public failure contracts and nontrivial evaluation algorithms are documented;
  comments explain invariants and policy rather than restating expressions.
- Rust warnings, missing public documentation, and unsafe code are rejected at
  the manifest level.
- Formatting, Clippy, tests, and rustdoc are required by one checked-in script.
  Pre-commit hooks provide the corresponding local checks.
- CI actions and the development toolchain are pinned for reproducibility.

## Intentional standalone differences

These are integration choices, not permanent forks from upstream practice:

- Dependencies use one Git revision. In the upstream workspace they should
  become versioned path dependencies and inherit workspace lints.
- `./dev.sh` and CI use the AMD64 development image because the pinned
  `xlsynth-sys` artifacts do not support the local ARM64 host path. Upstream's
  workspace CI and supported host matrix should replace this wrapper after
  integration.
- The nightly toolchain is date-pinned here; the upstream workspace toolchain
  becomes authoritative after integration.
- Z3 is currently an external process. Upstream review should decide whether to
  retain that adapter, inject a solver interface, or use an existing workspace
  facility before fixing the final crate boundary.
- Release versioning and publication metadata remain under upstream's release
  process. This repository must not pre-empt that process with an integration
  version bump or release tag.

## Integration sequence

1. Choose the destination crate or module and its public ownership boundary.
2. Move the library and focused tests without importing the standalone
   container or workflow machinery.
3. Replace Git dependencies with workspace path-and-version dependencies and
   inherit `[workspace.lints]`.
4. Adapt the checks to upstream pre-commit, nextest, SPDX, and CI targets.
5. Run the full upstream workspace suite and obtain API and solver-boundary
   review before any upstream release decision.
