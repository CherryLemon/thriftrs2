#!/usr/bin/env python3
"""
Concurrent RPC server benchmark.

Starts two Thrift UserService servers on different ports:
  - Port 9191: thriftrs2 (Rust-backed)
  - Port 9192: thriftpy2 (pure-Python)

Both servers implement the same UserService from example.thrift.
Clients are thriftpy2 clients in all cases, measuring the server-side
performance difference under concurrent load.

Metrics reported:
  - Throughput (ops/s)
  - Mean / P50 / P90 / P99 / P99.9 latency

Usage:
    python examples/server_benchmark.py [options]

Options:
    -n, --requests    Total requests per server (default: 2000)
    -c, --concurrency Number of concurrent threads (default: 20)
    --warmup          Warmup requests per server (default: 200)
    --mix             Op mix, e.g. "get:5,list:3,create:2" (default: get:5,list:3,create:2)
    --rs-port         Port for thriftrs2 server (default: 9191)
    --tp2-port        Port for thriftpy2 server (default: 9192)
    --host            Bind/connect host (default: 127.0.0.1)
"""

import os
import sys
import time
import socket
import random
import string
import argparse
import threading
import statistics
import multiprocessing
from collections import defaultdict

# from tqdm import trange
trange = range
# ── make local python package importable ──────────────────────────────────────
EXAMPLES_DIR = os.path.dirname(os.path.abspath(__file__))

THRIFT_FILE = os.path.join(EXAMPLES_DIR, 'example.thrift')

# ── import backends ────────────────────────────────────────────────────────────

import thriftpy2
from thriftpy2.rpc import make_client as tp2_make_client, make_aio_server as tp2_make_server
from thriftpy2.transport import TCyBufferedTransportFactory, TBufferedTransportFactory
from thriftpy2.contrib.aio.transport import TAsyncBufferedTransportFactory

HAS_THRIFTPY2 = True

from thriftrs2 import load as rs_load, TBufferedTransport, make_client as rs_make_client, \
    make_server as rs_make_server

HAS_RS = True


# ══════════════════════════════════════════════════════════════════════════════
#  Server startup helpers
# ══════════════════════════════════════════════════════════════════════════════

def _wait_port(host: str, port: int, timeout: float = 10.0) -> None:
    """Block until TCP port accepts connections (or raise RuntimeError)."""
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        try:
            with socket.create_connection((host, port), timeout=0.2):
                return
        except OSError:
            time.sleep(0.05)
    raise RuntimeError(f"Server {host}:{port} did not come up within {timeout}s")


def init(thrift_mod):
    # Populate with 1000 random thrift_mod.User objects.
    _db: dict = {}
    _next_id = [1001]

    for i in range(1, 11):
        _db[i] = thrift_mod.User(
            id=i,
            name=f"{_rand_str()}",
            email=f"{_rand_str(32)}@example.com",
            age=random.randint(18, 80),
        )

    class Handler:
        async def get_user(self, user_id):
            return _db.get(user_id)

        async def create_user(self, user):
            uid = user.id if user.id is not None else _next_id[0]
            if uid in _db:
                return False
            user.id = uid
            _db[uid] = user
            _next_id[0] = max(_next_id[0], uid + 1)
            return True

        async def list_users(self):
            # Return snapshot without locking for the read-only benchmark
            return list(_db.values())

    return Handler()


def start_rs_server(host: str, port: int) -> None:
    """Start the thriftrs2 server in a background OS thread (serve_nonblocking)."""
    if not HAS_RS:
        raise RuntimeError("thriftrs2 is not importable")

    thrift_mod = rs_load(THRIFT_FILE)
    server = rs_make_server(
        thrift_mod.UserService,
        init(thrift_mod),
        workers=4
    )

    server.serve_forever(host, port, blocking=False)
    print(f"[rs   ] server ready on {host}:{port}")


def start_tp2_server(host: str, port: int) -> None:
    """Start the thriftpy2 server in a daemon thread."""
    if not HAS_THRIFTPY2:
        raise RuntimeError("thriftpy2 is not importable")

    thrift_mod = thriftpy2.load(THRIFT_FILE, module_name='bench_tp2_thrift')

    server = tp2_make_server(
        thrift_mod.UserService,
        init(thrift_mod),
        host=host,
        port=port,
        trans_factory=TAsyncBufferedTransportFactory(),
    )

    t = threading.Thread(target=server.serve, daemon=True)
    t.start()
    _wait_port(host, port)
    print(f"[tp2  ] server ready on {host}:{port}")


# ══════════════════════════════════════════════════════════════════════════════
#  Client factories
# ══════════════════════════════════════════════════════════════════════════════

def make_rs_client(host: str, port: int):
    """Create a connected thriftrs2 ThriftClient."""
    mod = rs_load(THRIFT_FILE)
    client = rs_make_client(
        mod.UserService,
        host,
        port,
        TBufferedTransport.transport_type,
    )
    return client, mod


def make_tp2_client(host: str, port: int):
    """Create a fresh thriftpy2 client connection."""
    thrift_mod = thriftpy2.load(THRIFT_FILE, module_name='bench_tp2_thrift')
    client = tp2_make_client(
        thrift_mod.UserService,
        host, port,
        trans_factory=TCyBufferedTransportFactory(),
    )
    return client, thrift_mod


# ══════════════════════════════════════════════════════════════════════════════
#  Benchmark worker
# ══════════════════════════════════════════════════════════════════════════════

def _rand_str(n: int = 8) -> str:
    return ''.join(random.choices(string.ascii_lowercase, k=n))


def _bench_worker(
        host: str,
        port: int,
        ops_mix: list,
        n_requests: int,
        result_queue: multiprocessing.Queue,  # will receive (latencies_list, errors_list)
        use_rs: bool = False,
) -> None:
    """Process worker: performs requests and puts (latencies, errors) into result_queue."""
    try:
        if use_rs:
            client, thrift_mod = make_rs_client(host, port)
        else:
            client, thrift_mod = make_tp2_client(host, port)
    except Exception as exc:
        # send empty latencies with connect error
        result_queue.put(([], [f"connect error: {exc}"]))
        return

    local_latencies = []
    local_errors = []

    try:
        for _ in trange(n_requests):
            op = random.choice(ops_mix)
            try:
                t0 = time.perf_counter()
                if use_rs:
                    if op == 'get':
                        uid = random.randint(1, 5)
                        client.call("get_user", user_id=uid)
                    elif op == 'list':
                        ids = [i.id for i in client.call("list_users")]
                    elif op == 'create':
                        uid = random.randint(1000, 99999)
                        user = thrift_mod.User(**{
                            "id": uid,
                            "name": _rand_str(),
                            "email": f"{_rand_str(1024)}@test.com",
                            "age": random.randint(18, 80),
                        })
                        client.call("create_user", user=user)
                else:
                    client, thrift_mod = make_tp2_client(host, port)
                    if op == 'get':
                        uid = random.randint(1, 5)
                        client.get_user(uid)
                    elif op == 'list':
                        ids = [i.id for i in client.list_users()]
                    elif op == 'create':
                        uid = random.randint(1000, 99999)
                        user = thrift_mod.User(
                            id=uid,
                            name=_rand_str(),
                            email=f"{_rand_str(1024)}@test.com",
                            age=random.randint(18, 80),
                        )
                        client.create_user(user)
                elapsed = time.perf_counter() - t0
                local_latencies.append((op, elapsed))
            except Exception as exc:
                local_errors.append(f"{op}: {exc}")
    finally:
        if use_rs:
            try:
                client.close()
            except Exception:
                pass

    # send results back to parent
    result_queue.put((local_latencies, local_errors))


# ══════════════════════════════════════════════════════════════════════════════
#  Stats helpers
# ══════════════════════════════════════════════════════════════════════════════

def _percentile(data: list, pct: float) -> float:
    if not data:
        return float('nan')
    s = sorted(data)
    idx = (pct / 100.0) * (len(s) - 1)
    lo, hi = int(idx), min(int(idx) + 1, len(s) - 1)
    return s[lo] + (idx - lo) * (s[hi] - s[lo])


def _run_bench(
        host: str,
        port: int,
        ops_mix: list,
        requests_per_thread: int,
        concurrency: int,
        use_rs: bool = False,
) -> tuple:
    """
    Spawn `concurrency` processes, each making `requests_per_thread` calls.
    Returns (all_latencies, all_errors, wall_seconds).
    """
    manager = multiprocessing.Manager()
    result_queue = manager.Queue()

    processes = [
        multiprocessing.Process(
            target=_bench_worker,
            args=(host, port, ops_mix, requests_per_thread, result_queue),
            kwargs={"use_rs": use_rs},
            daemon=False,
        )
        for _ in range(concurrency)
    ]

    # start processes
    t_start = time.perf_counter()
    for p in processes:
        p.start()

    # collect results
    all_latencies = []
    all_errors = []

    for _ in range(concurrency):
        lat_list, err_list = result_queue.get()
        all_latencies.append(lat_list)
        all_errors.extend(err_list)

    # ensure all processes exit
    for p in processes:
        p.join()

    wall = time.perf_counter() - t_start

    return all_latencies, all_errors, wall


# ══════════════════════════════════════════════════════════════════════════════
#  Report
# ══════════════════════════════════════════════════════════════════════════════

def _report(name: str, all_latencies: list, all_errors: list, wall: float) -> dict:
    flat = [lat for worker in all_latencies for (_, lat) in worker]
    per_op: dict = defaultdict(list)
    for worker in all_latencies:
        for op, lat in worker:
            per_op[op].append(lat)

    total = len(flat)
    errs = len(all_errors)

    print(f"\n{'═' * 62}")
    print(f"  {name}")
    print(f"{'═' * 62}")

    if total == 0:
        print("  ⚠  No successful requests recorded.")
        for e in all_errors[:5]:
            print(f"     {e}")
        return {}

    mean_ms = statistics.mean(flat) * 1e3
    p50_ms = _percentile(flat, 50) * 1e3
    p90_ms = _percentile(flat, 90) * 1e3
    p99_ms = _percentile(flat, 99) * 1e3
    p999_ms = _percentile(flat, 99.9) * 1e3
    tput = total / wall

    print(f"  Requests        : {total}  (errors: {errs})")
    print(f"  Wall time       : {wall:.3f} s")
    print(f"  Throughput      : {tput:>10.1f} ops/s")
    print(f"  Latency (ms)")
    print(f"    mean          : {mean_ms:>10.3f}")
    print(f"    p50           : {p50_ms:>10.3f}")
    print(f"    p90           : {p90_ms:>10.3f}")
    print(f"    p99           : {p99_ms:>10.3f}")
    print(f"    p99.9         : {p999_ms:>10.3f}")

    if len(per_op) > 1:
        print(f"  Per-op breakdown  (mean ms / p90 ms / count)")
        for op in sorted(per_op):
            lats = per_op[op]
            pmean = statistics.mean(lats) * 1e3
            pp90 = _percentile(lats, 90) * 1e3
            print(f"    {op:<10}: mean={pmean:7.3f}  p90={pp90:7.3f}  n={len(lats)}")

    if all_errors:
        print(f"  Sample errors (first 5):")
        for e in all_errors[:5]:
            print(f"    {e}")

    return {
        'tput': tput, 'mean': mean_ms, 'p50': p50_ms,
        'p90': p90_ms, 'p99': p99_ms, 'p999': p999_ms,
    }


def _comparison(rs_stats: dict, tp2_stats: dict) -> None:
    if not rs_stats or not tp2_stats:
        return

    print(f"\n{'═' * 62}")
    print(f"  Comparison  (thriftrs2  vs  thriftpy2)")
    print(f"{'═' * 62}")
    print(f"  {'Metric':<22}  {'thriftrs2':>16}  {'thriftpy2':>12}  {'rs/tp2':>8}")
    print(f"  {'-' * 22}  {'-' * 16}  {'-' * 12}  {'-' * 8}")

    rows = [
        ("Throughput (ops/s)", 'tput', True, '.1f'),
        ("Mean latency (ms)", 'mean', False, '.3f'),
        ("P50 latency (ms)", 'p50', False, '.3f'),
        ("P90 latency (ms)", 'p90', False, '.3f'),
        ("P99 latency (ms)", 'p99', False, '.3f'),
        ("P99.9 latency (ms)", 'p999', False, '.3f'),
    ]

    for label, key, higher_better, fmt in rows:
        rs_v = rs_stats[key]
        tp2_v = tp2_stats[key]
        ratio = rs_v / tp2_v if tp2_v else float('inf')
        if higher_better:
            mark = ' ✓' if ratio > 1 else (' ✗' if ratio < 1 else '')
        else:
            mark = ' ✓' if ratio < 1 else (' ✗' if ratio > 1 else '')
        print(f"  {label:<22}  {rs_v:>15{fmt}}  {tp2_v:>12{fmt}}  {ratio:>7.3f}x{mark}")

    print()
    print("  ✓ = thriftrs2 wins   ✗ = thriftpy2 wins")
    print()


# ══════════════════════════════════════════════════════════════════════════════
#  CLI entry-point
# ══════════════════════════════════════════════════════════════════════════════

def parse_mix(mix_str: str) -> list:
    """'get:5,list:3,create:2'  ->  ['get'×5, 'list'×3, 'create'×2]"""
    expanded = []
    for part in mix_str.split(','):
        op, _, weight = part.partition(':')
        op = op.strip()
        w = int(weight.strip()) if weight.strip() else 1
        expanded.extend([op] * w)
    if not expanded:
        raise ValueError(f"Invalid mix string: {mix_str!r}")
    return expanded


def main() -> None:
    ap = argparse.ArgumentParser(
        description='Concurrent Thrift RPC server benchmark',
        formatter_class=argparse.RawDescriptionHelpFormatter,
    )
    ap.add_argument('-n', '--requests', type=int, default=15000,
                    help='Total requests per server (default: 2000)')
    ap.add_argument('-c', '--concurrency', type=int, default=30,
                    help='Concurrent client threads (default: 20)')
    ap.add_argument('--warmup', type=int, default=200,
                    help='Warmup requests per server (default: 200)')
    ap.add_argument('--mix', default='get:5,list:3,create:2',
                    help='Op mix (default: get:5,list:3,create:2)')
    ap.add_argument('--rs-port', type=int, default=9191)
    ap.add_argument('--tp2-port', type=int, default=9192)
    ap.add_argument('--host', default='127.0.0.1')
    args = ap.parse_args()

    if not HAS_THRIFTPY2:
        sys.exit("thriftpy2 is required for the client side of this benchmark.")

    # Force the benchmark to exercise only the list_users path.
    ops_mix = ['list']
    host = args.host
    rs_port = args.rs_port
    tp2_port = args.tp2_port

    rpt = max(1, args.requests // args.concurrency)  # requests per thread
    wpt = max(1, args.warmup // args.concurrency)  # warmup per thread

    print('═' * 62)
    print('  Thrift RPC Concurrent Server Benchmark')
    print('═' * 62)
    print(f'  Concurrency      : {args.concurrency} processes')
    print(f'  Requests/server  : {rpt * args.concurrency}  ({rpt} × {args.concurrency})')
    print(f'  Warmup/server    : {wpt * args.concurrency}')
    print(f'  Op mix           : {args.mix}')
    print('  NOTE             : Running list-only benchmark (1000 pre-populated users)')
    print(f'  thriftrs2        : {host}:{rs_port}')
    print(f'  thriftpy2        : {host}:{tp2_port}')
    print()

    # ── start servers ─────────────────────────────────────────────────────────
    rs_ok = tp2_ok = False

    if HAS_RS:
        try:
            start_rs_server(host, rs_port)
            rs_ok = True
        except Exception as exc:
            print(f"[WARN] rs server failed to start: {exc}")
    else:
        print("[SKIP] thriftrs2 not available")

    if HAS_THRIFTPY2:
        try:
            start_tp2_server(host, tp2_port)
            tp2_ok = True
        except Exception as exc:
            print(f"[WARN] tp2 server failed to start: {exc}")

    if not rs_ok and not tp2_ok:
        sys.exit("No servers could be started; aborting.")

    time.sleep(0.3)  # brief settle

    # ── warmup ────────────────────────────────────────────────────────────────
    wc = min(args.concurrency, 5)
    print('\nWarming up...')
    if rs_ok:
        try:
            _run_bench(host, rs_port, ops_mix, wpt, wc, use_rs=True)
            print('  [rs   ] warmup done')
        except Exception as exc:
            print(f'  [rs   ] warmup error: {exc}')
    if tp2_ok:
        try:
            _run_bench(host, tp2_port, ops_mix, wpt, wc)
            print('  [tp2  ] warmup done')
        except Exception as exc:
            print(f'  [tp2  ] warmup error: {exc}')

    # ── benchmark ─────────────────────────────────────────────────────────────
    print('\nRunning benchmark...')

    rs_stats = {}
    tp2_stats = {}

    if rs_ok:
        print(f'\n  → thriftrs2      ({host}:{rs_port}) …')
        lats, errs, wall = _run_bench(host, rs_port, ops_mix, rpt, args.concurrency, use_rs=True)
        rs_stats = _report('thriftrs2', lats, errs, wall)

    if tp2_ok:
        print(f'\n  → thriftpy2       ({host}:{tp2_port}) …')
        lats, errs, wall = _run_bench(host, tp2_port, ops_mix, rpt, args.concurrency, use_rs=False)
        tp2_stats = _report('thriftpy2', lats, errs, wall)

    if rs_ok and tp2_ok:
        _comparison(rs_stats, tp2_stats)


if __name__ == '__main__':
    main()
