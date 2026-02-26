#!/usr/bin/env python3
"""
Benchmark script to compare serialization/deserialization speed between:
- thriftpy2 (examples/test_thriftpy2.py)
- this project's Python bindings (examples/test.py)

Usage: python3 examples/benchmark.py

The script runs multiple iterations and prints ops/sec and average time per op.
"""
import sys
import os
import time
import argparse

# Ensure local python package is importable
sys.path.insert(0, os.path.join(os.path.dirname(__file__), '..', 'python'))

# Import thriftpy2 path (we assume thriftpy2 is installed in the environment)
try:
    import thriftpy2
    from thriftpy2.utils import serialize as tp2_serialize, deserialize as tp2_deserialize
except Exception:
    thriftpy2 = None

# Import local bindings
from thrift_rs_pyo3 import load as rs_load, serialize as rs_serialize, deserialize as rs_deserialize

THRIFT_FILE = os.path.join(os.path.dirname(__file__), 'example.thrift')

SAMPLE = {
    'id': 123,
    'name': 'John Doe',
    'email': 'john@example.com' * 1000,
    'age': 30,
}

DEFAULT_ITER = 20000


def bench_thriftpy2(iterations):
    if thriftpy2 is None:
        print('thriftpy2 not available in this environment; skipping thriftpy2 benchmarks')
        return None

    mod = thriftpy2.load(THRIFT_FILE, module_name='example_thrift')
    User = mod.User

    # prepare one serialized blob for deserialize benchmark
    blob = tp2_serialize(User(**SAMPLE))

    # serialize benchmark
    t0 = time.perf_counter()
    for _ in range(iterations):
        tp2_serialize(User(**SAMPLE))
    t1 = time.perf_counter()

    # deserialize benchmark
    t2 = time.perf_counter()
    for _ in range(iterations):
        _ = tp2_deserialize(User(), blob)
    t3 = time.perf_counter()

    return {
        'serialize_total': t1 - t0,
        'deserialize_total': t3 - t2,
        'payload_size': len(blob),
    }


def bench_rs_binding(iterations):
    mod = rs_load(THRIFT_FILE)
    User = mod.User

    # The project's API accepts either (User, dict) for serialize or the struct type; examples/test.py uses serialize(User, user_data)
    # Create one serialized blob for deserialize benchmark
    blob = rs_serialize(User, SAMPLE)

    # serialize benchmark
    t0 = time.perf_counter()
    for _ in range(iterations):
        rs_serialize(User, SAMPLE)
    t1 = time.perf_counter()

    # deserialize benchmark
    t2 = time.perf_counter()
    for _ in range(iterations):
        _ = rs_deserialize(User, blob)
    t3 = time.perf_counter()

    return {
        'serialize_total': t1 - t0,
        'deserialize_total': t3 - t2,
        'payload_size': len(blob),
    }


def pretty_report(name, results, iterations):
    if results is None:
        return
    s_total = results['serialize_total']
    d_total = results['deserialize_total']
    size = results['payload_size']
    print(f"\n== {name} ==")
    print(f"payload size: {size} bytes")
    print(f"serialize: total {s_total:.6f}s, avg {s_total/iterations*1e6:.2f}us, ops/s {iterations/s_total:.2f}")
    print(f"deserialize: total {d_total:.6f}s, avg {d_total/iterations*1e6:.2f}us, ops/s {iterations/d_total:.2f}")


def main():
    parser = argparse.ArgumentParser(description='Benchmark thrift serialization libs')
    parser.add_argument('-n', '--iterations', type=int, default=DEFAULT_ITER, help='iterations per op (serialize/deserialize)')
    args = parser.parse_args()

    print(f"Running {args.iterations} iterations per operation")

    # Warmup runs
    print('Warming up...')
    try:
        # warmup thriftpy2 if available
        if thriftpy2 is not None:
            mod = thriftpy2.load(THRIFT_FILE, module_name='example_thrift')
            User = mod.User
            for _ in range(100):
                tp2_serialize(User(**SAMPLE))
                _ = tp2_deserialize(User(), tp2_serialize(User(**SAMPLE)))
    except Exception:
        pass

    try:
        mod = rs_load(THRIFT_FILE)
        User = mod.User
        for _ in range(100):
            rs_serialize(User, SAMPLE)
            _ = rs_deserialize(User, rs_serialize(User, SAMPLE))
    except Exception:
        pass

    # Run benchmarks
    tp2_res = bench_thriftpy2(args.iterations)
    rs_res = bench_rs_binding(args.iterations)

    pretty_report('thriftpy2', tp2_res, args.iterations)
    pretty_report('thrift_rs_pyo3', rs_res, args.iterations)

    if tp2_res is not None and rs_res is not None:
        print('\nSummary:')
        for op in ('serialize', 'deserialize'):
            t_tp2 = tp2_res[f'{op}_total']
            t_rs = rs_res[f'{op}_total']
            ratio = t_tp2 / t_rs if t_rs > 0 else float('inf')
            faster = 'thriftpy2' if ratio < 1 else 'thrift_rs_pyo3'
            print(f"{op}: {faster} is {ratio:.2f}x faster (tp2: {t_tp2:.6f}s, rs: {t_rs:.6f}s)")


if __name__ == '__main__':
    main()

