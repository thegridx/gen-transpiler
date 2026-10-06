# Contributing

## Before you open a pull request

Run these from the repository root. All of them must pass:

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --release
(cd smoke-tests && uv run --locked transpiler_smoke_tests.py)
```

If you change the smoke-test script, also run these from `smoke-tests/`:

```bash
uv run --locked ruff check .
uv run --locked ruff format --check .
uv run --locked mypy
```

The [README](README.md) lists the tools these commands need.

## Code rules

The workspace `Cargo.toml` and `clippy.toml` enforce most rules. Some are not enforced by a lint:

- Do not use `unwrap()`, `expect()` or panics for error handling outside tests.
- Use checked or saturating arithmetic.
- Use little-endian byte order only.
- Put every `unsafe` block behind `#[expect(unsafe_code, reason = "...")]`, with a `SAFETY:`
  comment that states the invariants it relies on.

## Test fixtures

The prebuilt programs under `data/c_programs/artifacts/` and
`data/rust_programs/math-test/artifacts/` are checked in on purpose. Gas is metered per
RISC-V instruction, so a different compiler version can change the expected gas values. If you
rebuild a fixture, update the expected values in `smoke-tests/transpiler_smoke_tests.py` in the
same pull request.
