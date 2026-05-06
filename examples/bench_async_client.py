#!/usr/bin/env python3
"""Benchmark: sync ThriftClient vs AsyncThriftClient.

Spins up a single thriftrs2 UserService server on localhost and measures:

  1. sync_sequential   - ThriftClient.call(...) in a tight loop.
  2. async_sequential  - one AsyncThriftClient, awaited one call at a time.
  3. async_concurrent  - N AsyncThriftClients sharing the event loop with
     asyncio.gather (each client owns its own connection, so calls overlap).

The async-concurrent number is the only one that actually exercises pipelining
of independent connections; the sequential numbers show per-call overhead.

Usage:
    python examples/bench_async_client.py
    python examples/bench_async_client.py --requests 5000 --concurrency 32
"""

from __future__ import annotations

import argparse
import asyncio
import os
import socket
import statistics
import time

import thriftrs2

EXAMPLES_DIR = os.path.dirname(os.path.abspath(__file__))
THRIFT_FILE = os.path.join(EXAMPLES_DIR, "example.thrift")


class Handler:
    def get_user(self, user_id):
        return User(id=user_id, name=f"user-{user_id}", email=None, age=None)

    def create_user(self, user):
        return user.id > 0

    def list_users(self):
        return [User(id=i, name=f"user-{i}", email=None, age=None) for i in range(4)]


User = None  # populated after loading the module


def _free_port() -> int:
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as sock:
        sock.bind(("127.0.0.1", 0))
        return sock.getsockname()[1]


def _wait_port(host: str, port: int, timeout: float = 5.0) -> None:
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        try:
            with socket.create_connection((host, port), timeout=0.2):
                return
        except OSError:
            time.sleep(0.02)
    raise RuntimeError(f"server {host}:{port} did not come up within {timeout}s")


def _summary(name: str, latencies_us: list[float], duration_s: float, total: int) -> None:
    qps = total / duration_s if duration_s > 0 else float("inf")
    p50 = statistics.median(latencies_us)
    p99 = statistics.quantiles(latencies_us, n=100)[98] if len(latencies_us) >= 100 else max(latencies_us)
    mean = statistics.mean(latencies_us)
    print(
        f"{name:<22}  total={total:>5}  duration={duration_s*1000:>7.1f} ms  "
        f"qps={qps:>9.1f}  mean={mean:>7.1f} us  p50={p50:>7.1f}  p99={p99:>7.1f}"
    )


def bench_sync_sequential(service_module, host: str, port: int, total: int) -> None:
    client = thriftrs2.make_client(service_module.UserService, host, port)
    latencies = []
    try:
        # warmup
        for _ in range(min(50, total)):
            client.call("get_user", user_id=0)
        start = time.perf_counter()
        for i in range(total):
            t0 = time.perf_counter()
            client.call("get_user", user_id=i)
            latencies.append((time.perf_counter() - t0) * 1e6)
        duration = time.perf_counter() - start
    finally:
        client.close()
    _summary("sync_sequential", latencies, duration, total)


def bench_async_sequential(service_module, host: str, port: int, total: int) -> None:
    async def run():
        client = await thriftrs2.make_async_client(service_module.UserService, host, port)
        latencies = []
        try:
            for _ in range(min(50, total)):
                await client.call("get_user", user_id=0)
            start = time.perf_counter()
            for i in range(total):
                t0 = time.perf_counter()
                await client.call("get_user", user_id=i)
                latencies.append((time.perf_counter() - t0) * 1e6)
            duration = time.perf_counter() - start
        finally:
            await client.close()
        return latencies, duration

    latencies, duration = asyncio.run(run())
    _summary("async_sequential", latencies, duration, total)


def bench_async_concurrent(
    service_module, host: str, port: int, total: int, concurrency: int
) -> None:
    async def run():
        clients = [
            await thriftrs2.make_async_client(service_module.UserService, host, port)
            for _ in range(concurrency)
        ]
        latencies = []
        try:
            # warmup one round per client
            await asyncio.gather(*(c.call("get_user", user_id=0) for c in clients))

            sem = asyncio.Semaphore(concurrency)

            async def one(i: int):
                async with sem:
                    client = clients[i % concurrency]
                    t0 = time.perf_counter()
                    await client.call("get_user", user_id=i)
                    latencies.append((time.perf_counter() - t0) * 1e6)

            start = time.perf_counter()
            await asyncio.gather(*(one(i) for i in range(total)))
            duration = time.perf_counter() - start
        finally:
            await asyncio.gather(*(c.close() for c in clients))
        return latencies, duration

    latencies, duration = asyncio.run(run())
    _summary(f"async_concurrent_{concurrency}", latencies, duration, total)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("-n", "--requests", type=int, default=2000)
    parser.add_argument("-c", "--concurrency", type=int, default=16)
    args = parser.parse_args()

    service_module = thriftrs2.load(THRIFT_FILE)
    global User
    User = service_module.User

    port = _free_port()
    server = thriftrs2.make_server(
        service_module.UserService,
        Handler(),
        transport=thriftrs2.TransportType.Buffered,
        protocol=thriftrs2.ProtocolType.Binary,
        workers=max(2, args.concurrency // 4),
    )
    server.serve_forever("127.0.0.1", port, blocking=False)
    _wait_port("127.0.0.1", port)

    print(
        f"thriftrs2 UserService on 127.0.0.1:{port}  "
        f"(requests={args.requests}, concurrency={args.concurrency})\n"
    )

    try:
        bench_sync_sequential(service_module, "127.0.0.1", port, args.requests)
        bench_async_sequential(service_module, "127.0.0.1", port, args.requests)
        bench_async_concurrent(
            service_module, "127.0.0.1", port, args.requests, args.concurrency
        )
    finally:
        server._server.stop()


if __name__ == "__main__":
    main()
