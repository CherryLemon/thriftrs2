#!/usr/bin/env python3
"""Profile xlarge (~100 KB) struct deserialization for thriftrs2 vs thriftpy2."""

import cProfile
import pstats
import tempfile
import time
from pathlib import Path

import thriftpy2
from thriftpy2.protocol import TJSONProtocolFactory
from thriftpy2.utils import deserialize as tp2_deserialize
from thriftpy2.utils import serialize as tp2_serialize

from thriftrs2 import ProtocolType, deserialize as rs_deserialize, load, serialize

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


def main():
    with tempfile.TemporaryDirectory() as tmp:
        idl_path = Path(tmp) / "bench.thrift"
        idl_path.write_text(BENCH_IDL)

        rs_mod = load(str(idl_path))
        tp2_mod = thriftpy2.load(str(idl_path), module_name="prof_bench_thrift")

        rs_struct = rs_mod.BatchReport
        tp2_struct = tp2_mod.BatchReport
        data = make_xlarge()

        # Pre-build thriftpy2 object
        tp2_obj = tp2_mod.BatchReport(
            id=data["id"],
            title=data["title"],
            profiles=[
                tp2_mod.ComplexProfile(
                    user=tp2_mod.SimpleUser(**p["user"]),
                    addresses=[tp2_mod.Address(**a) for a in p["addresses"]],
                    scores=p["scores"],
                    tags=list(p["tags"]),
                    events=[tp2_mod.Event(**e) for e in p["events"]],
                    payload=p["payload"],
                    note=p["note"],
                )
                for p in data["profiles"]
            ],
            metrics=data["metrics"],
            extra=data["extra"],
        )

        # Pre-serialize blobs with JSON protocol
        rs_blob = serialize(rs_struct, data, proto=ProtocolType.JSON)
        tp2_blob = tp2_serialize(tp2_obj, proto_factory=TJSONProtocolFactory())

        print(f"Payload sizes:")
        print(f"  thriftrs2 blob: {len(rs_blob):,} bytes")
        print(f"  thriftpy2 blob: {len(tp2_blob):,} bytes")
        print()

        # Warmup
        for _ in range(50):
            rs_deserialize(rs_struct, rs_blob, proto=ProtocolType.JSON)
            tp2_deserialize(tp2_struct(), tp2_blob, proto_factory=TJSONProtocolFactory())

        iterations = 500

        # --- thriftrs2 deserialize ---
        print(f"thriftrs2 deserialize x {iterations}:")
        start = time.perf_counter()
        for _ in range(iterations):
            rs_deserialize(rs_struct, rs_blob, proto=ProtocolType.JSON)
        rs_elapsed = time.perf_counter() - start
        print(f"  total: {rs_elapsed:.3f}s  avg: {rs_elapsed/iterations*1e6:.1f} µs  ops/s: {iterations/rs_elapsed:.1f}")

        # --- thriftpy2 deserialize ---
        print(f"thriftpy2 deserialize x {iterations}:")
        start = time.perf_counter()
        for _ in range(iterations):
            tp2_deserialize(tp2_struct(), tp2_blob, proto_factory=TJSONProtocolFactory())
        tp2_elapsed = time.perf_counter() - start
        print(f"  total: {tp2_elapsed:.3f}s  avg: {tp2_elapsed/iterations*1e6:.1f} µs  ops/s: {iterations/tp2_elapsed:.1f}")

        print(f"\nRatio: {tp2_elapsed/rs_elapsed:.2f}x {'faster' if rs_elapsed < tp2_elapsed else 'slower'} than thriftpy2")

        # --- cProfile the thriftrs2 deser ---
        print("\n\n=== cProfile: thriftrs2 deserialize ===")
        prof = cProfile.Profile()
        prof.enable()
        for _ in range(200):
            rs_deserialize(rs_struct, rs_blob, proto=ProtocolType.JSON)
        prof.disable()
        stats = pstats.Stats(prof).sort_stats("cumtime")
        stats.print_stats(30)

        # --- cProfile the thriftpy2 deser ---
        print("\n\n=== cProfile: thriftpy2 deserialize ===")
        prof2 = cProfile.Profile()
        prof2.enable()
        for _ in range(200):
            tp2_deserialize(tp2_struct(), tp2_blob, proto_factory=TJSONProtocolFactory())
        prof2.disable()
        stats2 = pstats.Stats(prof2).sort_stats("cumtime")
        stats2.print_stats(30)


if __name__ == "__main__":
    main()
