#!/usr/bin/env python3
"""Profile-only thriftrs2 xlarge deserialization to isolate Rust hot path."""

import time
import tempfile
from pathlib import Path

from thriftrs2 import ProtocolType, deserialize, load, serialize

BENCH_IDL = """
struct SimpleUser {
    1: required i32 id;
    2: required string name;
    3: optional string email;
}
struct Address {
    1: required string city;
    2: optional string street;
}
struct Event {
    1: required i64 ts;
    2: required string kind;
    3: required map<string, string> attrs;
}
struct ComplexProfile {
    1: required SimpleUser user;
    2: required list<Address> addresses;
    3: required map<string, list<i32>> scores;
    4: required set<string> tags;
    5: required list<Event> events;
    6: required binary payload;
    7: optional string note;
}
struct BatchReport {
    1: required i64 id;
    2: required string title;
    3: required list<ComplexProfile> profiles;
    4: required map<string, double> metrics;
    5: required binary extra;
}
"""


def make_complex(index: int):
    return {
        "user": {"id": index, "name": f"user-{index}", "email": None},
        "addresses": [
            {"city": "Hangzhou", "street": "Wenyi"},
            {"city": "Shanghai", "street": None},
            {"city": "Beijing", "street": "Zhongguancun"},
        ],
        "scores": {"search": [95, 97, 99, 100], "rpc": [80, 85, 90], "json": [88, 89, 91]},
        "tags": ["json", "rpc", "nested", "benchmark"],
        "events": [
            {"ts": 1_700_000_000_001, "kind": "created", "attrs": {"source": "fixture"}},
            {"ts": 1_700_000_000_123, "kind": "updated", "attrs": {"field": "email"}},
            {"ts": 1_700_000_001_999, "kind": "viewed", "attrs": {"device": "desktop"}},
        ],
        "payload": b"thriftrs2-benchmark-payload" * 8,
        "note": None,
    }


def make_xlarge():
    return {
        "id": 1,
        "title": "xlarge-batch",
        "profiles": [make_complex(i + 1) for i in range(100)],
        "metrics": {f"metric_{k}": float(k) * 1.5 for k in range(50)},
        "extra": b"x" * 512,
    }


with tempfile.TemporaryDirectory() as tmp:
    idl_path = Path(tmp) / "bench.thrift"
    idl_path.write_text(BENCH_IDL)
    mod = load(str(idl_path))
    struct = mod.BatchReport
    data = make_xlarge()
    blob = serialize(struct, data, proto=ProtocolType.JSON)
    print(f"Payload: {len(blob):,} bytes")

    # Warmup
    for _ in range(50):
        deserialize(struct, blob, proto=ProtocolType.JSON)

    # Timed loop — run long enough for py-spy to sample
    ITER = 2000
    print(f"Running {ITER} thriftrs2-only deserializations...")
    start = time.perf_counter()
    for _ in range(ITER):
        deserialize(struct, blob, proto=ProtocolType.JSON)
    elapsed = time.perf_counter() - start
    print(f"Total: {elapsed:.3f}s  avg: {elapsed/ITER*1e6:.1f} µs  ops/s: {ITER/elapsed:.1f}")
