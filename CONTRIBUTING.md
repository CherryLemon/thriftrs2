# Contributing

## Setup

- Rust stable toolchain (`rustup`)
- Python 3.9+ and a virtual environment

```bash
pip install "maturin>=1.8,<2.0" pytest thriftpy2
maturin develop --release
```

## Checks

```bash
cargo clippy --all-targets -- -D warnings
cargo test
python -m pytest -q
```

Optional local smoke:

```bash
python examples/test.py
python examples/test_protocols.py
```

## Pull requests

- Keep changes focused; match existing style and patterns described in [`AGENTS.md`](AGENTS.md).
- Ensure CI would pass: Rust clippy/tests and Python tests after `maturin develop --release`.

## Releases (maintainers)

See [`docs/USER_GUIDE.md`](docs/USER_GUIDE.md) (release checklist and PyPI Trusted Publishing).
