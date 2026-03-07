# thriftrs2 User Guide

This guide covers installation, the core Python API, example workflows, and a practical release checklist for `thriftrs2`.

## 1. What is `thriftrs2`?

`thriftrs2` is a Python package backed by a Rust implementation using PyO3.
It provides:

- a Thrift IDL loader
- dynamic struct/service access from parsed `.thrift` files
- fast serialization/deserialization helpers
- RPC client and server utilities

## 2. Installation

### Install from PyPI

```bash
pip install thriftrs2
```

### Install from source for development

```bash
pip install maturin pytest thriftpy2 twine
maturin develop --release
```

If you change Rust code, rerun:

```bash
maturin develop --release
```

## 3. Loading a `.thrift` file

```python
from thriftrs2 import load

mod = load("examples/example.thrift")
print(mod.User)
print(mod.UserService)
```

`load()` returns a module-like object exposing parsed structs and services as attributes.

## 4. Struct serialization and deserialization

```python
from thriftrs2 import load, serialize, deserialize

mod = load("examples/example.thrift")

user_data = {
    "id": 123,
    "name": "John Doe",
    "email": "john@example.com",
    "age": 30,
}

blob = serialize(mod.User, user_data)
restored = deserialize(mod.User, blob)
assert restored == user_data
```

## 5. Protocol selection

You can choose a protocol explicitly with `ProtocolType`.

```python
from thriftrs2 import load, serialize, deserialize, ProtocolType

mod = load("examples/example.thrift")
user = {"id": 1, "name": "Compact User", "email": "compact@example.com"}

blob = serialize(mod.User, user, proto=ProtocolType.Compact)
restored = deserialize(mod.User, blob, proto=ProtocolType.Compact)
```

Available values:

- `ProtocolType.Binary`
- `ProtocolType.Compact`
- `ProtocolType.JSON`

For JSON helpers:

```python
from thriftrs2 import dumps, loads

text = dumps(mod.User, user)
round_trip = loads(mod.User, text)
```

## 6. Running a Thrift client

```python
from thriftrs2 import load, make_client, TBufferedTransport

mod = load("examples/example.thrift")

with make_client(
    mod.UserService,
    "127.0.0.1",
    9090,
    TBufferedTransport.transport_type,
) as client:
    result = client.call("get_user", user_id=1)
    print(result)
```

## 7. Running a Thrift server

```python
from thriftrs2 import load, make_server, TBufferedTransport

mod = load("examples/example.thrift")

class Handler:
    def get_user(self, user_id):
        return mod.User(id=user_id, name="Alice", email="alice@example.com", age=30)

    def create_user(self, user):
        return True

    def list_users(self):
        return []

server = make_server(
    mod.UserService,
    Handler(),
    transport=TBufferedTransport.transport_type,
    workers=4,
)
server.serve_forever("127.0.0.1", 9090)
```

## 8. Repository examples

After installing the package into your environment, these examples should work directly:

```bash
python examples/test.py
python examples/test_protocols.py
python examples/server_example.py
python examples/client_example.py
```

## 9. Development workflow

Recommended local workflow:

```bash
cargo check
maturin develop --release
python -m pytest -q
python examples/test.py
python examples/test_protocols.py
```

## 10. Release checklist

Before publishing to PyPI:

1. Confirm `Cargo.toml` and `pyproject.toml` versions match your intended release.
2. Rebuild and reinstall the extension:
   ```bash
   maturin develop --release
   ```
3. Run tests:
   ```bash
   python -m pytest -q
   cargo check
   ```
4. Build wheels:
   ```bash
   maturin build --release
   ```
5. Validate package metadata:
   ```bash
   python -m twine check target/wheels/*
   ```
6. Upload:
   ```bash
   python -m twine upload target/wheels/*
   ```

## 11. Notes for maintainers

- The Python distribution name is `thriftrs2`.
- The Python import name is also `thriftrs2`.
- The native extension is built as `thriftrs2.thriftrs2` via maturin.
- Examples are written to use the installed package rather than a checked-in `.so` file.

