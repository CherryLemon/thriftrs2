#!/usr/bin/env python3
"""Compare JSON vs Binary vs Compact deserialization for xlarge struct."""
import time
import tempfile
from pathlib import Path
from thriftrs2 import ProtocolType, deserialize, load, serialize

BENCH_IDL = """
struct SimpleUser { 1: required i32 id; 2: required string name; 3: optional string email; }
struct Address { 1: required string city; 2: optional string street; }
struct Event { 1: required i64 ts; 2: required string kind; 3: required map<string, string> attrs; }
struct ComplexProfile {
    1: required SimpleUser user; 2: required list<Address> addresses;
    3: required map<string, list<i32>> scores; 4: required set<string> tags;
    5: required list<Event> events; 6: required binary payload; 7: optional string note;
}
struct BatchReport {
    1: required i64 id; 2: required string title;
    3: required list<ComplexProfile> profiles; 4: required map<string, double> metrics;
    5: required binary extra;
}
"""

def make_complex(i):
    return {
        "user": {"id": i, "name": f"user-{i}", "email": None},
        "addresses": [{"city": "Hangzhou", "street": "Wenyi"}, {"city": "Shanghai", "street": None}, {"city": "Beijing", "street": "Zhongguancun"}],
        "scores": {"search": [95,97,99,100], "rpc": [80,85,90], "json": [88,89,91]},
        "tags": ["json","rpc","nested","benchmark"],
        "events": [{"ts": 1700000000001+i, "kind": "created", "attrs": {"source":"f"}} for i in range(3)],
        "payload": b"thriftrs2-benchmark-payload" * 8,
        "note": None,
    }

def make_xlarge():
    return {"id": 1, "title": "xlarge-batch", "profiles": [make_complex(i+1) for i in range(100)], "metrics": {f"m_{k}": float(k)*1.5 for k in range(50)}, "extra": b"x" * 512}

with tempfile.TemporaryDirectory() as tmp:
    idl = Path(tmp) / "b.thrift"
    idl.write_text(BENCH_IDL)
    mod = load(str(idl))
    s = mod.BatchReport
    d = make_xlarge()

    for proto, pname in [(ProtocolType.Binary, "Binary"), (ProtocolType.Compact, "Compact"), (ProtocolType.JSON, "JSON")]:
        blob = serialize(s, d, proto=proto)
        print(f"{pname:>8} payload: {len(blob):>8,} bytes", end="")

        for _ in range(20):
            deserialize(s, blob, proto=proto)

        N = 200
        start = time.perf_counter()
        for _ in range(N):
            deserialize(s, blob, proto=proto)
        elapsed = time.perf_counter() - start
        print(f"  deser+to_dict x{N}: {elapsed:.3f}s avg={elapsed/N*1e6:.0f}us")
