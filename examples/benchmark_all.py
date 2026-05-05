#!/usr/bin/env python3
"""Comprehensive thriftrs2 vs thriftpy2 benchmark.

Covers:
- JSON serialization/deserialization for simple and nested structs.
- RPC performance across protocol, transport, method, payload shape, and client concurrency.

The script generates a temporary Thrift IDL so both libraries run against the
same schema. It prints matrix-style tables and can optionally emit JSON results.
"""

from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import socket
import statistics
import tempfile
import threading
import time
from dataclasses import asdict, dataclass
from typing import Any, Callable, Iterable

import thriftpy2
from thriftpy2.protocol import TJSONProtocolFactory
from thriftpy2.rpc import make_client as tp2_make_client
from thriftpy2.rpc import make_server as tp2_make_server
from thriftpy2.transport import TCyBufferedTransportFactory, TFramedTransportFactory
from thriftpy2.utils import deserialize as tp2_deserialize
from thriftpy2.utils import serialize as tp2_serialize

from thriftrs2 import (
    ProtocolType,
    TBufferedTransport,
    TFramedTransport,
    deserialize as rs_deserialize,
    load as rs_load,
    make_client as rs_make_client,
    make_server as rs_make_server,
    serialize as rs_serialize,
)


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

service BenchService {
    SimpleUser get_simple(1: i32 user_id);
    ComplexProfile get_complex(1: i32 user_id);
    bool save_complex(1: ComplexProfile profile);
    BatchReport get_batch(1: i32 batch_id);
    bool save_batch(1: BatchReport report);
}
"""


@dataclass(frozen=True)
class RpcConfig:
    protocol: str
    transport: str
    rs_protocol: ProtocolType
    rs_transport: Any
    compare_thriftpy2: bool
    tp2_proto_factory: Callable[[], Any] | None = None
    tp2_transport: Any = TCyBufferedTransportFactory


@dataclass
class LoopStats:
    library: str
    shape: str
    operation: str
    protocol: str
    iterations: int
    total_s: float
    avg_us: float
    ops_per_s: float
    payload_bytes: int | None = None


@dataclass
class RpcStats:
    library: str
    method: str
    protocol: str
    transport: str
    concurrency: int
    iterations: int
    total_s: float
    avg_ms: float
    p50_ms: float
    p90_ms: float
    p99_ms: float
    ops_per_s: float


def make_simple_data(index: int = 1) -> dict[str, Any]:
    return {
        "id": index,
        "name": f"user-{index}",
        "email": None,
    }


def make_complex_data(index: int = 1) -> dict[str, Any]:
    return {
        "user": make_simple_data(index),
        "addresses": [
            {"city": "Hangzhou", "street": "Wenyi"},
            {"city": "Shanghai", "street": None},
            {"city": "Beijing", "street": "Zhongguancun"},
        ],
        "scores": {
            "search": [95, 97, 99, 100],
            "rpc": [80, 85, 90],
            "json": [88, 89, 91],
        },
        "tags": ["json", "rpc", "nested", "benchmark"],
        "events": [
            {"ts": 1_700_000_000_001, "kind": "created", "attrs": {"source": "fixture"}},
            {"ts": 1_700_000_000_123, "kind": "updated", "attrs": {"field": "email"}},
            {"ts": 1_700_000_001_999, "kind": "viewed", "attrs": {"device": "desktop"}},
        ],
        "payload": (b"thriftrs2-benchmark-payload" * 8),
        "note": None,
    }


def make_large_data(index: int = 1, profile_count: int = 10) -> dict[str, Any]:
    return {
        "id": index,
        "title": f"batch-report-{index}",
        "profiles": [make_complex_data(i + 1) for i in range(profile_count)],
        "metrics": {f"metric_{k}": float(k) * 1.5 for k in range(50)},
        "extra": b"x" * 512,
    }


def make_xlarge_data(index: int = 1, profile_count: int = 100) -> dict[str, Any]:
    return make_large_data(index, profile_count)


def tp2_simple(mod: Any, data: dict[str, Any]) -> Any:
    return mod.SimpleUser(**data)


def tp2_complex(mod: Any, data: dict[str, Any]) -> Any:
    return mod.ComplexProfile(
        user=tp2_simple(mod, data["user"]),
        addresses=[mod.Address(**item) for item in data["addresses"]],
        scores=data["scores"],
        tags=list(data["tags"]),
        events=[mod.Event(**item) for item in data["events"]],
        payload=data["payload"],
        note=data["note"],
    )


def tp2_batch(mod: Any, data: dict[str, Any]) -> Any:
    return mod.BatchReport(
        id=data["id"],
        title=data["title"],
        profiles=[tp2_complex(mod, p) for p in data["profiles"]],
        metrics=data["metrics"],
        extra=data["extra"],
    )


def free_tcp_port() -> int:
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as sock:
        sock.bind(("127.0.0.1", 0))
        return sock.getsockname()[1]


def wait_port(host: str, port: int, timeout_s: float = 5.0) -> None:
    deadline = time.monotonic() + timeout_s
    while time.monotonic() < deadline:
        try:
            with socket.create_connection((host, port), timeout=0.2):
                return
        except OSError:
            time.sleep(0.02)
    raise RuntimeError(f"server did not open {host}:{port} within {timeout_s}s")


def measure_loop(fn: Callable[[], Any], iterations: int, warmup: int) -> tuple[float, Any]:
    last_result = None
    for _ in range(warmup):
        last_result = fn()
    start = time.perf_counter()
    for _ in range(iterations):
        last_result = fn()
    total = time.perf_counter() - start
    return total, last_result


def percentile(values: list[float], pct: float) -> float:
    if not values:
        return float("nan")
    ordered = sorted(values)
    pos = (len(ordered) - 1) * pct / 100.0
    lo = int(pos)
    hi = min(lo + 1, len(ordered) - 1)
    return ordered[lo] + (pos - lo) * (ordered[hi] - ordered[lo])


def split_counts(total: int, parts: int) -> list[int]:
    base, extra = divmod(total, parts)
    return [base + (1 if idx < extra else 0) for idx in range(parts)]


def measure_rpc_concurrent(
    client_factory: Callable[[], Any],
    call_factory: Callable[[Any], Callable[[], Any]],
    close_client: Callable[[Any], None],
    iterations: int,
    warmup: int,
    concurrency: int,
) -> tuple[float, list[float]]:
    clients = [client_factory() for _ in range(concurrency)]
    try:
        for index in range(warmup):
            call_factory(clients[index % concurrency])()

        barrier = threading.Barrier(concurrency + 1)
        errors: list[BaseException] = []
        latencies: list[float] = []
        latencies_lock = threading.Lock()

        def worker(client: Any, count: int) -> None:
            fn = call_factory(client)
            local_latencies: list[float] = []
            try:
                barrier.wait()
                for _ in range(count):
                    t0 = time.perf_counter()
                    fn()
                    local_latencies.append(time.perf_counter() - t0)
            except BaseException as exc:
                errors.append(exc)
            finally:
                with latencies_lock:
                    latencies.extend(local_latencies)

        threads = [
            threading.Thread(target=worker, args=(client, count), daemon=True)
            for client, count in zip(clients, split_counts(iterations, concurrency))
        ]
        for thread in threads:
            thread.start()
        start = time.perf_counter()
        barrier.wait()
        for thread in threads:
            thread.join()
        total = time.perf_counter() - start
        if errors:
            raise errors[0]
        return total, latencies
    finally:
        for client in clients:
            close_client(client)


def write_bench_idl(root: Path) -> Path:
    thrift_file = root / "benchmark_all.thrift"
    thrift_file.write_text(BENCH_IDL, encoding="utf-8")
    return thrift_file


def load_modules(thrift_file: Path) -> tuple[Any, Any]:
    rs_mod = rs_load(str(thrift_file))
    module_name = f"benchmark_all_{os.getpid()}_{time.time_ns()}_thrift"
    tp2_mod = thriftpy2.load(str(thrift_file), module_name=module_name)
    return rs_mod, tp2_mod


def run_json_matrix(rs_mod: Any, tp2_mod: Any, iterations: int, warmup: int) -> list[LoopStats]:
    cases = [
        ("simple", rs_mod.SimpleUser, make_simple_data(), tp2_mod.SimpleUser, tp2_simple(tp2_mod, make_simple_data())),
        (
            "complex",
            rs_mod.ComplexProfile,
            make_complex_data(),
            tp2_mod.ComplexProfile,
            tp2_complex(tp2_mod, make_complex_data()),
        ),
        (
            "large",
            rs_mod.BatchReport,
            make_large_data(),
            tp2_mod.BatchReport,
            tp2_batch(tp2_mod, make_large_data()),
        ),
        (
            "xlarge",
            rs_mod.BatchReport,
            make_xlarge_data(),
            tp2_mod.BatchReport,
            tp2_batch(tp2_mod, make_xlarge_data()),
        ),
    ]
    tp2_json = TJSONProtocolFactory()
    rows: list[LoopStats] = []

    for shape, rs_struct, rs_data, tp2_struct, tp2_obj in cases:
        rs_blob = rs_serialize(rs_struct, rs_data, proto=ProtocolType.JSON)
        tp2_blob = tp2_serialize(tp2_obj, proto_factory=tp2_json)

        total, _ = measure_loop(
            lambda: rs_serialize(rs_struct, rs_data, proto=ProtocolType.JSON),
            iterations,
            warmup,
        )
        rows.append(loop_stats("thriftrs2", shape, "serialize", "json", iterations, total, len(rs_blob)))

        total, _ = measure_loop(
            lambda: rs_deserialize(rs_struct, rs_blob, proto=ProtocolType.JSON),
            iterations,
            warmup,
        )
        rows.append(loop_stats("thriftrs2", shape, "deserialize", "json", iterations, total, len(rs_blob)))

        total, _ = measure_loop(
            lambda: tp2_serialize(tp2_obj, proto_factory=tp2_json),
            iterations,
            warmup,
        )
        rows.append(loop_stats("thriftpy2", shape, "serialize", "json", iterations, total, len(tp2_blob)))

        total, _ = measure_loop(
            lambda: tp2_deserialize(tp2_struct(), tp2_blob, proto_factory=tp2_json),
            iterations,
            warmup,
        )
        rows.append(loop_stats("thriftpy2", shape, "deserialize", "json", iterations, total, len(tp2_blob)))

    return rows


def loop_stats(
    library: str,
    shape: str,
    operation: str,
    protocol: str,
    iterations: int,
    total_s: float,
    payload_bytes: int | None = None,
) -> LoopStats:
    return LoopStats(
        library=library,
        shape=shape,
        operation=operation,
        protocol=protocol,
        iterations=iterations,
        total_s=total_s,
        avg_us=(total_s / iterations) * 1_000_000,
        ops_per_s=iterations / total_s if total_s else float("inf"),
        payload_bytes=payload_bytes,
    )


def make_rs_handler(rs_mod: Any) -> Any:
    complex_profile = rs_mod.ComplexProfile(**make_complex_data(7))
    large_report = rs_mod.BatchReport(**make_large_data(7))

    class Handler:
        def get_simple(self, user_id: int):
            return rs_mod.SimpleUser(id=user_id, name=f"user-{user_id}", email=None)

        def get_complex(self, user_id: int):
            _ = user_id
            return complex_profile

        def save_complex(self, profile: Any):
            return profile.user.id > 0 and len(profile.addresses) > 0

        def get_batch(self, batch_id: int):
            _ = batch_id
            return large_report

        def save_batch(self, report: Any):
            return report.id > 0 and len(report.profiles) > 0

    return Handler()


def make_tp2_handler(tp2_mod: Any) -> Any:
    complex_profile = tp2_complex(tp2_mod, make_complex_data(7))
    large_report = tp2_batch(tp2_mod, make_large_data(7))

    class Handler:
        def get_simple(self, user_id: int):
            return tp2_mod.SimpleUser(id=user_id, name=f"user-{user_id}", email=None)

        def get_complex(self, user_id: int):
            _ = user_id
            return complex_profile

        def save_complex(self, profile: Any):
            return profile.user.id > 0 and len(profile.addresses) > 0

        def get_batch(self, batch_id: int):
            _ = batch_id
            return large_report

        def save_batch(self, report: Any):
            return report.id > 0 and len(report.profiles) > 0

    return Handler()


def rpc_configs(include_framed: bool) -> list[RpcConfig]:
    configs = [
        RpcConfig("binary", "buffered", ProtocolType.Binary, TBufferedTransport.transport_type, True),
        RpcConfig("json", "buffered", ProtocolType.JSON, TBufferedTransport.transport_type, True, TJSONProtocolFactory),
    ]
    if include_framed:
        configs.extend(
            [
                RpcConfig("binary", "framed", ProtocolType.Binary, TFramedTransport.transport_type, True, tp2_transport=TFramedTransportFactory),
                RpcConfig("json", "framed", ProtocolType.JSON, TFramedTransport.transport_type, True, TJSONProtocolFactory, tp2_transport=TFramedTransportFactory),
            ]
        )
    return configs


def run_rpc_matrix(
    rs_mod: Any,
    tp2_mod: Any,
    iterations: int,
    warmup: int,
    host: str,
    concurrencies: Iterable[int],
    include_framed: bool,
) -> list[RpcStats]:
    rows: list[RpcStats] = []
    for config in rpc_configs(include_framed):
        for concurrency in concurrencies:
            rows.extend(run_rpc_config(rs_mod, tp2_mod, config, iterations, warmup, host, concurrency))
    return rows


def run_rpc_config(
    rs_mod: Any,
    tp2_mod: Any,
    config: RpcConfig,
    iterations: int,
    warmup: int,
    host: str,
    concurrency: int,
) -> list[RpcStats]:
    rs_port = free_tcp_port()
    rs_server = rs_make_server(
        rs_mod.BenchService,
        make_rs_handler(rs_mod),
        transport=config.rs_transport,
        protocol=config.rs_protocol,
        workers=max(4, concurrency),
    )
    rs_server.serve_forever(host, rs_port, blocking=False)

    tp2_port = free_tcp_port()
    tp2_server = None
    tp2_thread = None
    if config.compare_thriftpy2:
        tp2_kwargs: dict[str, Any] = {
            "host": host,
            "port": tp2_port,
            "trans_factory": config.tp2_transport(),
        }
        if config.tp2_proto_factory is not None:
            tp2_kwargs["proto_factory"] = config.tp2_proto_factory()
        tp2_server = tp2_make_server(tp2_mod.BenchService, make_tp2_handler(tp2_mod), **tp2_kwargs)
        tp2_thread = threading.Thread(target=tp2_server.serve, daemon=True)
        tp2_thread.start()
        wait_port(host, tp2_port)

    rows: list[RpcStats] = []
    try:
        rs_methods = [
            (
                "get_simple",
                lambda client: (lambda: client.call("get_simple", user_id=11)),
            ),
            (
                "get_complex",
                lambda client: (lambda: client.call("get_complex", user_id=11)),
            ),
            (
                "get_batch",
                lambda client: (lambda: client.call("get_batch", batch_id=13)),
            ),
            (
                "save_complex",
                lambda client: make_rs_save_complex_call(rs_mod, client),
            ),
            (
                "save_batch",
                lambda client: make_rs_save_batch_call(rs_mod, client),
            ),
        ]
        for method, call_factory in rs_methods:
            total, latencies = measure_rpc_concurrent(
                lambda: rs_make_client(rs_mod.BenchService, host, rs_port, config.rs_transport, protocol=config.rs_protocol),
                call_factory,
                lambda client: client.close(),
                iterations,
                warmup,
                concurrency,
            )
            rows.append(rpc_stats("thriftrs2", method, config, concurrency, iterations, total, latencies))

        if config.compare_thriftpy2:
            tp2_methods = [
                ("get_simple", lambda client: (lambda: client.get_simple(11))),
                ("get_complex", lambda client: (lambda: client.get_complex(11))),
                ("get_batch", lambda client: (lambda: client.get_batch(13))),
                ("save_complex", lambda client: make_tp2_save_complex_call(tp2_mod, client)),
                ("save_batch", lambda client: make_tp2_save_batch_call(tp2_mod, client)),
            ]
            for method, call_factory in tp2_methods:
                total, latencies = measure_rpc_concurrent(
                    lambda: make_tp2_client(tp2_mod, host, tp2_port, config),
                    call_factory,
                    close_tp2_client,
                    iterations,
                    warmup,
                    concurrency,
                )
                rows.append(rpc_stats("thriftpy2", method, config, concurrency, iterations, total, latencies))
    finally:
        rs_server._server.stop()
        if tp2_server is not None:
            close_tp2_server(tp2_server)
        if tp2_thread is not None:
            tp2_thread.join(timeout=0.1)

    return rows


def make_tp2_client(tp2_mod: Any, host: str, port: int, config: RpcConfig) -> Any:
    kwargs: dict[str, Any] = {
        "trans_factory": config.tp2_transport(),
    }
    if config.tp2_proto_factory is not None:
        kwargs["proto_factory"] = config.tp2_proto_factory()
    return tp2_make_client(tp2_mod.BenchService, host, port, **kwargs)


def make_rs_save_complex_call(rs_mod: Any, client: Any) -> Callable[[], Any]:
    profile = rs_mod.ComplexProfile(**make_complex_data(11))
    return lambda: client.call("save_complex", profile=profile)


def make_rs_save_batch_call(rs_mod: Any, client: Any) -> Callable[[], Any]:
    report = rs_mod.BatchReport(**make_large_data(11))
    return lambda: client.call("save_batch", report=report)


def make_tp2_save_complex_call(tp2_mod: Any, client: Any) -> Callable[[], Any]:
    profile = tp2_complex(tp2_mod, make_complex_data(11))
    return lambda: client.save_complex(profile)


def make_tp2_save_batch_call(tp2_mod: Any, client: Any) -> Callable[[], Any]:
    report = tp2_batch(tp2_mod, make_large_data(11))
    return lambda: client.save_batch(report)


def close_tp2_client(client: Any) -> None:
    try:
        client.close()
    except Exception:
        pass


def close_tp2_server(server: Any) -> None:
    for method_name in ("close", "shutdown", "stop"):
        method = getattr(server, method_name, None)
        if method is None:
            continue
        try:
            method()
            return
        except Exception:
            return


def rpc_stats(
    library: str,
    method: str,
    config: RpcConfig,
    concurrency: int,
    iterations: int,
    total_s: float,
    latencies: list[float],
) -> RpcStats:
    return RpcStats(
        library=library,
        method=method,
        protocol=config.protocol,
        transport=config.transport,
        concurrency=concurrency,
        iterations=iterations,
        total_s=total_s,
        avg_ms=statistics.mean(latencies) * 1_000,
        p50_ms=percentile(latencies, 50) * 1_000,
        p90_ms=percentile(latencies, 90) * 1_000,
        p99_ms=percentile(latencies, 99) * 1_000,
        ops_per_s=iterations / total_s if total_s else float("inf"),
    )


def aggregate_loop_rows(rows: list[LoopStats]) -> list[LoopStats]:
    groups: dict[tuple[str, str, str, str, int | None], list[LoopStats]] = {}
    for row in rows:
        groups.setdefault(
            (row.library, row.shape, row.operation, row.protocol, row.payload_bytes), []
        ).append(row)

    aggregated = []
    for (library, shape, operation, protocol, payload_bytes), group in groups.items():
        iterations = group[0].iterations
        avg_total_s = statistics.mean(row.total_s for row in group)
        aggregated.append(
            loop_stats(library, shape, operation, protocol, iterations, avg_total_s, payload_bytes)
        )
    return aggregated


def aggregate_rpc_rows(rows: list[RpcStats]) -> list[RpcStats]:
    groups: dict[tuple[str, str, str, str, int], list[RpcStats]] = {}
    for row in rows:
        groups.setdefault(
            (row.library, row.method, row.protocol, row.transport, row.concurrency), []
        ).append(row)

    aggregated = []
    for (library, method, protocol, transport, concurrency), group in groups.items():
        iterations = group[0].iterations
        avg_total_s = statistics.mean(row.total_s for row in group)
        avg_ms = statistics.mean(row.avg_ms for row in group)
        p50_ms = statistics.mean(row.p50_ms for row in group)
        p90_ms = statistics.mean(row.p90_ms for row in group)
        p99_ms = statistics.mean(row.p99_ms for row in group)
        aggregated.append(
            RpcStats(
                library=library,
                method=method,
                protocol=protocol,
                transport=transport,
                concurrency=concurrency,
                iterations=iterations,
                total_s=avg_total_s,
                avg_ms=avg_ms,
                p50_ms=p50_ms,
                p90_ms=p90_ms,
                p99_ms=p99_ms,
                ops_per_s=iterations / avg_total_s if avg_total_s else float("inf"),
            )
        )
    return aggregated


def print_json_matrix(rows: list[LoopStats]) -> None:
    print("\nJSON serialization/deserialization matrix")
    print("-" * 104)
    print(f"{'shape':<9} {'op':<12} {'library':<10} {'payload':>8} {'avg us':>12} {'ops/s':>14} {'ratio vs tp2':>14}")
    print("-" * 104)
    by_key = {(row.shape, row.operation, row.library): row for row in rows}
    for shape in ("simple", "complex", "large", "xlarge"):
        for op in ("serialize", "deserialize"):
            tp2 = by_key[(shape, op, "thriftpy2")]
            for library in ("thriftrs2", "thriftpy2"):
                row = by_key[(shape, op, library)]
                ratio = tp2.avg_us / row.avg_us if row.avg_us else float("inf")
                print(
                    f"{shape:<9} {op:<12} {library:<10} {row.payload_bytes:>8} "
                    f"{row.avg_us:>12.2f} {row.ops_per_s:>14.1f} {ratio:>13.2f}x"
                )
    print("ratio vs tp2 > 1.00 means faster than thriftpy2 for that row.")


def print_rpc_matrix(rows: list[RpcStats]) -> None:
    print("\nRPC performance matrix")
    print("-" * 142)
    print(
        f"{'proto':<7} {'transport':<9} {'conc':>4} {'method':<14} {'library':<10} "
        f"{'avg ms':>10} {'p50':>10} {'p90':>10} {'p99':>10} {'ops/s':>12} {'ratio vs tp2':>14}"
    )
    print("-" * 142)
    by_key = {
        (row.protocol, row.transport, row.concurrency, row.method, row.library): row
        for row in rows
    }
    config_keys = sorted({(row.protocol, row.transport, row.concurrency) for row in rows})
    for protocol, transport, concurrency in config_keys:
        for method in ("get_simple", "get_complex", "get_batch", "save_complex", "save_batch"):
            tp2 = by_key.get((protocol, transport, concurrency, method, "thriftpy2"))
            for library in ("thriftrs2", "thriftpy2"):
                row = by_key.get((protocol, transport, concurrency, method, library))
                if row is None:
                    continue
                ratio = "-"
                if tp2 is not None:
                    ratio = f"{(tp2.avg_ms / row.avg_ms if row.avg_ms else float('inf')):.2f}x"
                print(
                    f"{protocol:<7} {transport:<9} {concurrency:>4} {method:<14} {library:<10} "
                    f"{row.avg_ms:>10.3f} {row.p50_ms:>10.3f} {row.p90_ms:>10.3f} "
                    f"{row.p99_ms:>10.3f} {row.ops_per_s:>12.1f} {ratio:>14}"
                )
    print("ratio vs tp2 > 1.00 means lower average latency than thriftpy2 for that row.")
    print("A dash means no thriftpy2 row was run for that exact protocol/transport/concurrency.")


def parse_concurrency(values: list[int]) -> list[int]:
    unique = sorted({value for value in values if value > 0})
    if not unique:
        raise ValueError("at least one positive --rpc-concurrency value is required")
    return unique


def main() -> None:
    parser = argparse.ArgumentParser(description="Run thriftrs2 vs thriftpy2 benchmark matrix")
    parser.add_argument("--ser-iterations", type=int, default=500, help="iterations per JSON serialize/deserialize row")
    parser.add_argument("--rpc-iterations", type=int, default=1_000, help="iterations per RPC row")
    parser.add_argument("--warmup", type=int, default=50, help="warmup iterations per row")
    parser.add_argument("--host", default="127.0.0.1", help="RPC bind/connect host")
    parser.add_argument("--rpc-concurrency", type=int, nargs="+", default=[1, 4, 16, 64, 128], help="client concurrency levels for RPC rows")
    parser.add_argument("--runs", type=int, default=1, help="repeat the benchmark and report averaged rows")
    parser.add_argument("--ci-smoke", action="store_true", help="small stable matrix intended for CI smoke checks")
    parser.add_argument("--skip-framed-rpc", action="store_true", help="skip thriftrs2 framed RPC rows")
    parser.add_argument("--skip-rpc", action="store_true", help="skip RPC matrix")
    parser.add_argument("--output-json", type=Path, help="optional path to write machine-readable results")
    args = parser.parse_args()

    if args.ci_smoke:
        args.ser_iterations = 200
        args.rpc_iterations = 50
        args.warmup = 10
        args.rpc_concurrency = [1]
        args.skip_framed_rpc = True

    args.runs = max(1, args.runs)
    concurrencies = parse_concurrency(args.rpc_concurrency)

    with tempfile.TemporaryDirectory(prefix="thriftrs2-bench-") as tmp:
        thrift_file = write_bench_idl(Path(tmp))
        rs_mod, tp2_mod = load_modules(thrift_file)

        print("Comprehensive thriftrs2 benchmark")
        print(f"IDL: {thrift_file}")
        print(f"JSON iterations per row: {args.ser_iterations}")
        print(f"RPC iterations per row: {args.rpc_iterations if not args.skip_rpc else 0}")
        print(f"Warmup per row: {args.warmup}")
        print(f"RPC concurrency levels: {concurrencies if not args.skip_rpc else []}")
        print(f"Runs: {args.runs}")
        if args.ci_smoke:
            print("Mode: CI smoke")

        json_rows_raw: list[LoopStats] = []
        rpc_rows_raw: list[RpcStats] = []
        for run_index in range(args.runs):
            if args.runs > 1:
                print(f"\nRun {run_index + 1}/{args.runs}")
            json_rows_raw.extend(run_json_matrix(rs_mod, tp2_mod, args.ser_iterations, args.warmup))
            if not args.skip_rpc:
                rpc_rows_raw.extend(
                    run_rpc_matrix(
                        rs_mod,
                        tp2_mod,
                        args.rpc_iterations,
                        args.warmup,
                        args.host,
                        concurrencies,
                        include_framed=not args.skip_framed_rpc,
                    )
                )

        json_rows = aggregate_loop_rows(json_rows_raw)
        print_json_matrix(json_rows)

        rpc_rows = aggregate_rpc_rows(rpc_rows_raw)
        if rpc_rows:
            print_rpc_matrix(rpc_rows)

        if args.output_json:
            payload = {
                "metadata": {
                    "ser_iterations": args.ser_iterations,
                    "rpc_iterations": args.rpc_iterations if not args.skip_rpc else 0,
                    "warmup": args.warmup,
                    "rpc_concurrency": concurrencies if not args.skip_rpc else [],
                    "runs": args.runs,
                    "ci_smoke": args.ci_smoke,
                    "framed_rpc": not args.skip_framed_rpc and not args.skip_rpc,
                },
                "json": [asdict(row) for row in json_rows],
                "rpc": [asdict(row) for row in rpc_rows],
            }
            args.output_json.write_text(json.dumps(payload, indent=2), encoding="utf-8")
            print(f"\nWrote JSON results to {args.output_json}")


if __name__ == "__main__":
    main()
