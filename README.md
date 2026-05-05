<div align="center">

# thriftrs2

[![PyPI](https://img.shields.io/pypi/v/thriftrs2?color=blue)](https://pypi.org/project/thriftrs2/)
[![Python](https://img.shields.io/pypi/pyversions/thriftrs2)](https://pypi.org/project/thriftrs2/)
[![License](https://img.shields.io/github/license/CherryLemon/thriftrs2)](LICENSE)
[![Rust](https://img.shields.io/badge/Rust-1.80+-orange)](https://www.rust-lang.org/)

**A fast, Rust-powered Apache Thrift toolkit for Python.**

[Features](#features) · [Quick Start](#quick-start) · [Documentation](docs/USER_GUIDE.md) · [Changelog](CHANGELOG.md)

</div>

---

## About

`thriftrs2` brings native Rust performance to Python Thrift workflows through PyO3. It provides an end-to-end toolkit: parse `.thrift` IDL files, serialize and deserialize structs, and run RPC clients and servers — all with a Python-first API that feels idiomatic.

> Status: alpha. The core serialization, IDL parsing, and RPC paths are stable enough for evaluation; the API may still shift before 1.0.

### Why another Thrift library?

Existing Python Thrift libraries are pure-Python and carry a serialization bottleneck. `thriftrs2` replaces the hot path (parsing, ser/de, RPC framing) with compiled Rust, while keeping the user-facing API in Python where flexibility matters.

## Features

| Category | What's included |
|----------|----------------|
| **IDL parser** | `struct`, `service`, `enum`, `union`, `exception`, `const`, `typedef`, `include`, `namespace`, `throws`, annotations, field defaults, `extends` inheritance |
| **Protocols** | Binary, Compact, JSON (TJSON field-id format) |
| **Serialization** | `serialize` / `deserialize` for structs; `dumps` / `loads` for JSON text |
| **Transports** | `TBufferedTransport`, `TFramedTransport` |
| **RPC client** | Context-manager based, sync `call()` with automatic request/response framing |
| **RPC server** | Multi-threaded (configurable workers), sync handler dispatch, oneway support |
| **Compatibility** | Reads thriftpy2 JSON envelopes; runs structured benchmarks against thriftpy2 |

## Quick Start

### Installation

```bash
pip install thriftrs2
```

Requires Python ≥ 3.9.

### 1. Load a Thrift file

```python
from thriftrs2 import load

mod = load("example.thrift")
# mod.User       → struct type
# mod.UserService → service type
```

### 2. Serialize and deserialize

```python
from thriftrs2 import serialize, deserialize

user = {"id": 1, "name": "Alice", "email": "alice@example.com", "age": 30}

blob = serialize(mod.User, user)
restored = deserialize(mod.User, blob)
assert restored == user
```

Protocol selection:

```python
from thriftrs2 import ProtocolType

blob = serialize(mod.User, user, proto=ProtocolType.Compact)
restored = deserialize(mod.User, blob, proto=ProtocolType.Compact)

# JSON helpers
from thriftrs2 import dumps, loads
text = dumps(mod.User, user)
restored = loads(mod.User, text)
```

### 3. RPC client

```python
from thriftrs2 import make_client, TBufferedTransport, ProtocolType

with make_client(
    mod.UserService,
    "127.0.0.1", 9090,
    TBufferedTransport.transport_type,
    protocol=ProtocolType.Binary,
) as client:
    user = client.call("get_user", user_id=1)
```

### 4. RPC server

```python
from thriftrs2 import make_server

class Handler:
    def get_user(self, user_id):
        return mod.User(id=user_id, name="Alice", email="alice@example.com", age=30)

server = make_server(
    mod.UserService, Handler(),
    transport=TBufferedTransport.transport_type,
    protocol=ProtocolType.Binary,
    workers=4,
)
server.serve_forever("127.0.0.1", 9090)
```

## Architecture

```
 ┌──────────────────────────────────────────┐
 │              Python API                   │
 │  load()  serialize()  make_client()  ...  │
 └──────────────┬───────────────────────────┘
                │ PyO3
 ┌──────────────┴───────────────────────────┐
 │              Rust Core                    │
 │  ┌──────────┐ ┌──────────┐ ┌───────────┐ │
 │  │  Parser  │ │ Protocol │ │   Python   │ │
 │  │  (nom)   │ │ (bin/cmp │ │  bindings  │ │
 │  │          │ │  /json)  │ │            │ │
 │  └──────────┘ └──────────┘ └───────────┘ │
 │  ┌──────────────────────────────────────┐ │
 │  │     Client / Server (tokio)          │ │
 │  └──────────────────────────────────────┘ │
 └──────────────────────────────────────────┘
```

- **Parser** — Nom-based `.thrift` IDL parser producing an AST
- **Protocol** — Binary, Compact, and JSON read/write with correct framing
- **Client/Server** — Tokio-powered async I/O behind a sync Python API

## Project Structure

```
thriftrs2/
├── src/
│   ├── lib.rs                  # PyO3 module entry point
│   ├── parser/                 # IDL parser (lexer, AST, grammar)
│   ├── protocol/               # Binary, Compact, JSON ser/de
│   └── python/                 # PyO3 bindings (client, server, types, parser wrappers)
├── python/
│   └── thriftrs2/              # Python package layer
│       ├── __init__.py         # Public API re-exports
│       ├── loader.py           # load(), make_client(), make_server()
│       └── protocol.py         # Python-side protocol helpers
├── examples/                   # Runnable examples & benchmarks
│   ├── example.thrift          # Sample IDL
│   ├── test.py                 # Struct round-trip
│   ├── test_protocols.py       # Protocol comparison
│   ├── client_example.py       # RPC client
│   ├── server_example.py       # RPC server
│   ├── ocr_client.py           # Larger service client
│   ├── ocr_server.py           # Larger service server
│   ├── benchmark.py            # Serialization micro-benchmark
│   └── benchmark_all.py        # Full matrix: ser/de + RPC vs thriftpy2
├── python/tests/               # pytest + cargo test suites
├── docs/USER_GUIDE.md          # Detailed user guide
├── Cargo.toml                  # Rust crate manifest (version source of truth)
├── pyproject.toml              # Python build config (maturin)
└── CHANGELOG.md                # Keep a Changelog
```

## Performance

Results from `benchmark_all.py` (500 ser/de, 1K RPC iterations, 50 warmup, 3 runs, on AMD Ryzen 9950X3D). All comparisons vs thriftpy2.

### Struct deserialization — all protocols

Deserialize + `to_dict()`, ops/s (higher = better):

| Shape | Wire bytes (Bin/Cmp/JSON) | Binary | Compact | JSON | JSON vs tp2 |
|-------|--------------------------:|-------:|--------:|-----:|------------:|
| simple | 21 / 11 / 36 B | 1,610,845 | 1,501,299 | 1,155,703 | **3.3×** |
| complex | 641 / 460 / 986 B | 233,622 | 227,657 | 113,934 | **2.4×** |
| large | 8.0 / 6.0 / 11.5 KB | 18,576 | 18,619 | 8,199 | **3.0×** |
| xlarge | 65.8 / 47.6 / 100.5 KB | 2,072 | 2,111 | 796 | **2.2×** |

Binary and Compact are neck-and-neck; Compact payloads are ~30% smaller. JSON deserialization uses a direct `serde_json::Value → Python` conversion path that skips the intermediate `ThriftValue` tree.

### JSON serialize / deserialize — vs thriftpy2

| Shape | Payload | Serialize | vs tp2 | Deserialize | vs tp2 |
|-------|--------:|------------------:|-------:|--------------------:|-------:|
| simple | 36 B | 3,108,918 ops/s | **10.1×** | 1,143,568 ops/s | **3.3×** |
| complex | ~1 KB | 131,788 ops/s | **5.6×** | 76,620 ops/s | **2.4×** |
| large | ~11 KB | 7,155 ops/s | **2.9×** | 8,566 ops/s | **3.0×** |
| xlarge | ~100 KB | 776 ops/s | **2.9×** | 646 ops/s | **2.2×** |

Serialization runs 2.9–10.1× faster. Deserialization leads 2.2–3.3× across all payload sizes, reversing the pre-optimization gap at large payloads.

### RPC: `get_batch` (~11 KB) — throughput (req/s) and speedup

| Protocol | Transport | Conc=1 | Conc=4 | Conc=16 | Conc=64 |
|----------|-----------|-------:|-------:|--------:|--------:|
| Binary | Buffered | 13,233 (**547×**) | 15,009 (**155×**) | 11,739 (**31×**) | 10,393 (**7.1×**) |
| Binary | Framed | 15,480 (**1.36×**) | 13,548 (**1.59×**) | 12,221 (**1.69×**) | 10,750 (**1.40×**) |
| JSON | Buffered | 1,335 (**1.15×**) | 1,457 (**1.33×**) | 1,147 (**1.22×**) | 1,161 (**1.33×**) |
| JSON | Framed | 1,454 (**1.25×**) | 1,442 (**1.35×**) | 1,209 (**1.26×**) | 1,158 (**1.36×**) |

Values are throughput (requests / second) with speedup vs thriftpy2. All rows include thriftpy2 comparison (both Buffered and Framed transports). Binary Framed achieves the highest single-connection throughput for large payloads. Under concurrency, both Buffered and Framed Binary deliver ~10K+ req/s sustained. See the full matrix including `get_simple`, `get_complex`, `save_complex`, and `save_batch` by running:

```bash
# CI smoke (fast)
python examples/benchmark_all.py --ci-smoke

# Full matrix
python examples/benchmark_all.py \
    --ser-iterations 500 \
    --rpc-iterations 1000 \
    --warmup 50 \
    --rpc-concurrency 1 4 16 64 \
    --runs 3
```

## Known Limitations

These are tracked gaps, not permanent design decisions:

- **JSON output envelope** — Reads thriftpy2 JSON envelopes but does not yet emit them
- **Exception types** — Declared Thrift exceptions are decoded and raised as `RuntimeError` rather than dedicated Python exception classes
- **Multi-file namespaces** — `include` resolves types across files but does not yet enforce a full scoped namespace model for same-name types
- **Benchmarks** — Smoke mode is suitable for CI; production-grade numbers should use `--runs 3` with adequate warmup

See [open issues](https://github.com/CherryLemon/thriftrs2/issues) for the current backlog.

## Development

```bash
# Setup
pip install maturin pytest
maturin develop --release

# Run tests
python -m pytest -q
cargo test
cargo check

# Rebuild after Rust changes
maturin develop --release
```

## Contributing

Contributions are welcome. The project is early-stage, so starting with an issue to discuss scope is recommended before investing in large changes.

1. Fork the repository
2. Create a feature branch
3. Make your changes and add tests
4. Run `python -m pytest -q && cargo test`
5. Open a pull request

## License

MIT — see [LICENSE](LICENSE).

---

<div align="center">

**[PyPI](https://pypi.org/project/thriftrs2/)** · **[Issues](https://github.com/CherryLemon/thriftrs2/issues)** · **[Changelog](CHANGELOG.md)**

</div>
