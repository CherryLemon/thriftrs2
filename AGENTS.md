# AGENTS.md

## Project snapshot
- `thriftrs2` is a Python package backed by a Rust `cdylib` built with PyO3 + maturin. The package/import name is `thriftrs2`, and the native module is built as `thriftrs2.thriftrs2` (`pyproject.toml`, `src/lib.rs`).
- The codebase is intentionally Python-first at the API layer, but most parsing, protocol, and RPC I/O logic lives in Rust.

## Architecture to understand before editing
- `src/parser/*` is the IDL front end: lexer + AST + parser. `Parser::parse_document()` currently recognizes `struct` and `service` definitions; other tokens are skipped, and `include_dirs` in `python/thriftrs2/loader.py` is currently accepted but not used.
- `src/python/parser.rs` converts parsed AST into Python-visible `ThriftStruct`, `PyThriftService`, and `PyThriftMethod` objects and builds the shared `struct_map()` used for nested struct resolution.
- `src/python/types.rs` is the core schema-aware Python boundary. `ThriftStruct` is callable from Python (for example `mod.User(id=1, ...)`) and produces `ThriftStructInstance`; serialization accepts either a plain `dict` or a `ThriftStructInstance`.
- `src/python/serde.rs` is the shared conversion layer between Python objects and wire values. If a type change touches nested structs, containers, or attribute coercion, it usually belongs here.
- `src/protocol/{binary,compact,json}.rs` holds protocol codecs; `src/protocol/types.rs` defines shared protocol traits/types.
- `src/python/client.rs` and `src/python/server.rs` implement RPC over Tokio TCP. The client serializes calls in Rust, and the server dispatches Python handlers after decoding the request. Optionally, `ThriftServer` can register an **HTTP probe handler**: peek at the start of each connection; if it looks like HTTP (e.g. `GET /health`, `GET /metrics` for Prometheus), hand the socket to Python so you can answer **same-port** HTTP probes (K8s/LB health, metrics scrapes, `curl` ops checks, scanners) while Thrift RPC stays on the same listen address; otherwise decode as Thrift. The callback is not a built-in HTTP stack—implement minimal responses or wrap the socket yourself.
- The high-level Python façade lives in `python/thriftrs2/{__init__.py,loader.py,protocol.py}` and intentionally mimics `thriftpy2` usage (`load`, `make_client`, `make_server`, `serialize`, `deserialize`).

## Codebase-specific conventions
- Keep the helper API and the low-level PyO3 exports in sync: changes in `src/lib.rs` usually require matching updates in `python/thriftrs2/__init__.py` and sometimes `python/thriftrs2/protocol.py` or `loader.py`.
- Preserve the module-like loader pattern from `python/thriftrs2/loader.py`: `load("examples/example.thrift")` returns an object whose structs/services are accessed as attributes (`mod.User`, `mod.UserService`).
- Preserve the current return-shape split: raw Rust deserializers produce `ThriftStructInstance`, while the convenience Python helpers in `python/thriftrs2/protocol.py` call `.to_dict()` and return plain dicts.
- Be careful with transport defaults: low-level `ThriftClient`/`ThriftServer` default to `TransportType::Framed` in Rust, but `make_client()`/`make_server()` default to buffered transport and the examples use `TBufferedTransport.transport_type`.
- Server handlers may be sync or `async def`; async support relies on per-worker Python event loops created in `src/python/server.rs` via `pyo3-async-runtimes`.
- Examples and tests use the installed/importable package, not a local path hack. After Rust edits, rebuild/reinstall instead of assuming the checked-in `.so` is current.

## Developer workflow used in this repo
- Install/reinstall the extension after Rust changes:
  - `maturin develop --release`
- Baseline verification:
  - `python -m pytest -q`
  - `cargo check`
- Useful smoke tests from the repo docs:
  - `python examples/test.py`
  - `python examples/test_protocols.py`
  - `python examples/server_example.py` and `python examples/client_example.py`
- Packaging workflow documented by the repo:
  - `maturin build --release`
  - `python -m twine check target/wheels/*`

## High-value edit paths
- Adding/parsing new IDL features usually requires coordinated changes across `src/parser/lexer.rs`, `src/parser/mod.rs`, `src/parser/ast.rs`, and then the PyO3 wrappers in `src/python/parser.rs` / `src/python/types.rs`.
- Changing wire compatibility or field encoding should be traced through `src/python/serde.rs` plus the affected protocol file in `src/protocol/`.
- RPC behavior changes should be checked against both helper entry points (`python/thriftrs2/loader.py`) and the runnable examples in `examples/server_example.py` / `examples/client_example.py`.

