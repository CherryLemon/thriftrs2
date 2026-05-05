# thriftrs2

`thriftrs2` is a Rust-powered Apache Thrift toolkit for Python built with PyO3.
It focuses on fast struct serialization/deserialization plus practical client/server helpers with a Python-first API.

> Status: early-stage project / alpha. Expect API changes while the package stabilizes.

## Features

- Parse `.thrift` IDL files from Python
- Create Python-accessible struct and service definitions dynamically
- Serialize and deserialize Thrift structs
- Support for Binary, Compact, and JSON protocols
- Simple client and server helpers for RPC workflows
- Rust implementation for performance-sensitive paths

## Installation

### From PyPI

```bash
pip install thriftrs2
```

### From source

For local development, install the extension into your current virtual environment:

```bash
pip install maturin
maturin develop --release
```

## Quick start

Given a Thrift file like:

```thrift
struct User {
    1: required i32 id;
    2: required string name;
    3: optional string email;
    4: optional i32 age;
}
```

You can load it and round-trip a struct:

```python
from thriftrs2 import load, serialize, deserialize

thrift_module = load("example.thrift")
User = thrift_module.User

payload = {
    "id": 1,
    "name": "Alice",
    "email": "alice@example.com",
    "age": 30,
}

blob = serialize(User, payload)
restored = deserialize(User, blob)
print(restored)
```

## Protocols

`thriftrs2` exposes an enum named `ProtocolType`:

- `ProtocolType.Binary`
- `ProtocolType.Compact`
- `ProtocolType.JSON`

Example:

```python
from thriftrs2 import load, serialize, deserialize, dumps, loads, ProtocolType

mod = load("example.thrift")
user = {"id": 1, "name": "Compact User", "email": "compact@example.com"}

compact_blob = serialize(mod.User, user, proto=ProtocolType.Compact)
restored = deserialize(mod.User, compact_blob, proto=ProtocolType.Compact)

json_text = dumps(mod.User, user)
json_restored = loads(mod.User, json_text)
```

The same protocol enum can be used by the RPC helpers. Both client and server
must use the same protocol:

```python
server = make_server(
    mod.UserService,
    Handler(),
    transport=TBufferedTransport.transport_type,
    protocol=ProtocolType.JSON,
)

client = make_client(
    mod.UserService,
    "127.0.0.1",
    9090,
    TBufferedTransport.transport_type,
    protocol=ProtocolType.JSON,
)
```

## RPC helpers

### Client

```python
from thriftrs2 import load, make_client, TBufferedTransport, ProtocolType

mod = load("example.thrift")

with make_client(
    mod.UserService,
    "127.0.0.1",
    9090,
    TBufferedTransport.transport_type,
    protocol=ProtocolType.Binary,
) as client:
    user = client.call("get_user", user_id=1)
    print(user)
```

### Server

```python
from thriftrs2 import load, make_server, TBufferedTransport, ProtocolType

mod = load("example.thrift")

class Handler:
    def get_user(self, user_id):
        return mod.User(id=user_id, name="Alice", email="alice@example.com", age=30)

server = make_server(
    mod.UserService,
    Handler(),
    transport=TBufferedTransport.transport_type,
    protocol=ProtocolType.Binary,
    workers=4,
)
server.serve_forever("127.0.0.1", 9090)
```

## Examples

The repository includes runnable examples in [`examples/`](examples/):

- `examples/test.py` — struct round-trip
- `examples/test_protocols.py` — protocol comparison
- `examples/client_example.py` / `examples/server_example.py` — RPC usage
- `examples/ocr_client.py` / `examples/ocr_server.py` — larger service example
- `examples/benchmark.py` — serialization benchmark

After installing from source with `maturin develop`, you can run:

```bash
python examples/test.py
python examples/test_protocols.py
```

## Documentation

See [`docs/USER_GUIDE.md`](docs/USER_GUIDE.md) for a more complete user guide, development workflow, and publishing checklist.

## Development

Run the test suite:

```bash
python -m pytest -q
cargo check
```

Reinstall the extension after Rust changes:

```bash
maturin develop --release
```

## Publishing to PyPI

Releases are built in CI (`.github/workflows/release.yml`). Push an annotated or lightweight tag `vX.Y.Z` after bumping `Cargo.toml` to trigger wheel builds, `twine check`, upload to PyPI via **Trusted Publishing (OIDC)**, and GitHub Release assets.

Manual build:

```bash
maturin build --release
python -m twine check target/wheels/*
```

See [`docs/USER_GUIDE.md`](docs/USER_GUIDE.md) for the maintainer checklist and PyPI publisher setup.

## Repository

- Homepage: <https://github.com/CherryLemon/thriftrs2>
- Issues: <https://github.com/CherryLemon/thriftrs2/issues>

