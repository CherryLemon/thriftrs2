# thriftrs2 User Guide

This guide covers installation, the core Python API, example workflows, and a practical release checklist for `thriftrs2`.

## 1. What is `thriftrs2`?

`thriftrs2` is a Python package backed by a Rust implementation using PyO3.
It provides:

- a Thrift IDL loader
- dynamic struct/service access from parsed `.thrift` files
- fast serialization/deserialization helpers
- RPC client and server utilities
- practical thriftpy2 interop checks and benchmark tooling

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

The loader supports `include_dirs` and common IDL features used by thriftpy2-style schemas:

- `include` and qualified references such as `common.User`
- `namespace`
- `typedef`
- `enum` constants exposed as module attributes
- `const` declarations
- `exception` and service `throws`
- `union`
- annotations, which are parsed and ignored
- field defaults
- `struct extends` and `service extends` inheritance when the parent is parsed first

Example:

```python
mod = load("service.thrift", include_dirs=["idl"])
assert mod.Status.OK == 1
```

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

JSON deserialization accepts two input dialects:

- thriftrs2's TJSON field-id object format
- thriftpy2 `utils` TJSON envelopes with a 4-byte length prefix and `metadata`/`payload`

The reverse direction is intentionally tracked as an interop gap: `thriftrs2`
does not yet emit thriftpy2's metadata envelope.

RPC helpers also accept `protocol=...`. The client and server must agree on
the same protocol:

```python
from thriftrs2 import ProtocolType, TBufferedTransport, make_client, make_server

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

## 6. Running a Thrift client

```python
from thriftrs2 import load, make_client, TBufferedTransport, ProtocolType

mod = load("examples/example.thrift")

with make_client(
    mod.UserService,
    "127.0.0.1",
    9090,
    TBufferedTransport.transport_type,
    protocol=ProtocolType.Binary,
) as client:
    result = client.call("get_user", user_id=1)
    print(result)
```

## 7. Running a Thrift server

```python
from thriftrs2 import load, make_server, TBufferedTransport, ProtocolType

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
    protocol=ProtocolType.Binary,
    workers=4,
)
server.serve_forever("127.0.0.1", 9090)
```

Declared exceptions can be returned over RPC when a handler raises a Python
exception whose class name matches a `throws` exception struct. The client
decodes the declared exception payload using the service schema and raises a
Python `RuntimeError` containing the decoded value. A dedicated exception type
is planned but not part of the current API.

## 8. Repository examples

After installing the package into your environment, these examples should work directly:

```bash
python examples/test.py
python examples/test_protocols.py
python examples/server_example.py
python examples/client_example.py
python examples/benchmark_all.py --ci-smoke
```

## 9. Development workflow

Recommended local workflow:

```bash
cargo check
maturin develop --release
python -m pytest -q
cargo test
python examples/test.py
python examples/test_protocols.py
python examples/benchmark_all.py --ci-smoke
```

For a fuller local performance run against thriftpy2:

```bash
python examples/benchmark_all.py \
    --ser-iterations 10000 \
    --rpc-iterations 1000 \
    --warmup 300 \
    --rpc-concurrency 1 4 \
    --runs 3 \
    --output-json target/benchmark_all_results.json
```

Benchmark ratio columns use thriftpy2 as `1.00x`; values greater than `1.00x`
mean `thriftrs2` is faster for that row.

## 10. Release checklist

The **release version** is defined only in `Cargo.toml` (`[package].version`). Maturin copies that into wheel/sdist metadata; `thriftrs2.__version__` reads it via `importlib.metadata` after install.

Before publishing to PyPI:

1. Bump **`Cargo.toml`** only (no second copy in Python source).
2. Update **`CHANGELOG.md`** and prepare GitHub Release notes.
3. Rebuild and reinstall locally:
   ```bash
   maturin develop --release
   ```
4. Run tests:

```bash
python -m pytest -q
cargo test
cargo check
```

The current suite includes one expected JSON interop xfail for the direction
where thriftpy2 reads thriftrs2 JSON output.
5. Tag and push (triggers CI wheels + optional PyPI upload), e.g. `v0.1.0`.

Manual build (without CI):

```bash
maturin build --release
python -m twine check target/wheels/*
```

## 11. PyPI Trusted Publishing (maintainers)

CI publishes with **OIDC** (no long-lived PyPI password in secrets) when configured on PyPI:

1. On [pypi.org](https://pypi.org/manage/account/publishing/), add a **pending publisher** for this repo: owner `CherryLemon`, repo `thriftrs2`, workflow `release.yml`, environment `pypi` (if the workflow uses that environment name).
2. Optionally repeat for [TestPyPI](https://test.pypi.org/manage/account/publishing/) and use the `workflow_dispatch` input on `release.yml` to upload to TestPyPI first.
3. After the first successful upload, finalize the project on PyPI if prompted.

## 12. Notes for maintainers

- The Python distribution name is `thriftrs2`.
- The Python import name is also `thriftrs2`.
- The native extension is built as `thriftrs2.thriftrs2` via maturin.
- Examples are written to use the installed package rather than a checked-in `.so` file.

### Post-release smoke (clean venv)

```bash
python -m venv .smoke-venv
. .smoke-venv/bin/activate
pip install dist/thriftrs2-*.whl   # or the wheel path from CI artifacts
python examples/test.py
deactivate
rm -rf .smoke-venv
```

