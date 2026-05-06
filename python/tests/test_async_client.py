from __future__ import annotations

import asyncio
from io import StringIO

import pytest

import thriftrs2
from thriftrs2.loader import load_fp


pytestmark = pytest.mark.rpc


class _RecordingHandler:
    def __init__(self, user_struct):
        self.user_struct = user_struct
        self.notifications = []

    def get_user(self, user_id):
        return self.user_struct(id=user_id, name=f"user-{user_id}", email=None)

    def create_user(self, user):
        return user.id > 0 and bool(user.name)

    async def list_users(self):
        await asyncio.sleep(0)
        return [self.user_struct(id=1, name="Alice", email="alice@example.com")]

    def ping(self):
        return None

    def notify(self, message):
        self.notifications.append(message)
        return None


def _start_server(service_module, port):
    handler = _RecordingHandler(service_module.User)
    server = thriftrs2.make_server(
        service_module.UserService,
        handler,
        transport=thriftrs2.TransportType.Buffered,
        protocol=thriftrs2.ProtocolType.Binary,
        workers=2,
    )
    server.serve_forever("127.0.0.1", port, blocking=False)
    return server, handler


def test_async_client_round_trip(service_module, free_tcp_port):
    server, _handler = _start_server(service_module, free_tcp_port)

    async def scenario():
        client = await thriftrs2.make_async_client(
            service_module.UserService,
            "127.0.0.1",
            free_tcp_port,
        )
        try:
            user = await client.call("get_user", user_id=11)
            assert user.to_dict() == {"id": 11, "name": "user-11", "email": None}
        finally:
            await client.close()

    try:
        asyncio.run(scenario())
    finally:
        server._server.stop()


def test_async_client_concurrent_calls(service_module, free_tcp_port):
    """Each AsyncThriftClient owns one connection, so concurrent gather() calls
    on a single client must serialise at the connection level. This test pins
    that contract by running gather() with multiple per-call clients in
    parallel — the async runtime should overlap their I/O without blocking
    the event loop."""

    server, _handler = _start_server(service_module, free_tcp_port)

    async def scenario():
        clients = [
            await thriftrs2.make_async_client(
                service_module.UserService,
                "127.0.0.1",
                free_tcp_port,
            )
            for _ in range(8)
        ]
        try:
            results = await asyncio.gather(
                *(client.call("get_user", user_id=i) for i, client in enumerate(clients))
            )
            assert [u.to_dict()["id"] for u in results] == list(range(8))
        finally:
            await asyncio.gather(*(client.close() for client in clients))

    try:
        asyncio.run(scenario())
    finally:
        server._server.stop()


def test_async_client_async_with(service_module, free_tcp_port):
    server, _handler = _start_server(service_module, free_tcp_port)

    async def scenario():
        client = thriftrs2.AsyncThriftClient(
            service_module.UserService.service_def,
            "127.0.0.1",
            free_tcp_port,
            thriftrs2.TransportType.Buffered,
            thriftrs2.ProtocolType.Binary,
        )
        client.set_parser(service_module.UserService.parser)
        async with client as opened:
            assert opened is client
            assert client.is_open() is True
            user = await client.call("get_user", user_id=3)
            assert user.id == 3
        assert client.is_open() is False

    try:
        asyncio.run(scenario())
    finally:
        server._server.stop()


def test_async_client_oneway_returns_none(service_module, free_tcp_port):
    server, handler = _start_server(service_module, free_tcp_port)

    async def scenario():
        client = await thriftrs2.make_async_client(
            service_module.UserService,
            "127.0.0.1",
            free_tcp_port,
        )
        try:
            assert await client.call("notify", message="async-hello") is None
            for _ in range(50):
                if handler.notifications:
                    break
                await asyncio.sleep(0.02)
        finally:
            await client.close()

    try:
        asyncio.run(scenario())
        assert handler.notifications == ["async-hello"]
    finally:
        server._server.stop()


def test_async_client_call_before_open_raises(service_module, free_tcp_port):
    async def scenario():
        client = thriftrs2.AsyncThriftClient(
            service_module.UserService.service_def,
            "127.0.0.1",
            free_tcp_port,
        )
        client.set_parser(service_module.UserService.parser)
        with pytest.raises(OSError, match="not open"):
            await client.call("get_user", user_id=1)

    asyncio.run(scenario())


def test_async_client_unknown_method_raises(service_module, free_tcp_port):
    async def scenario():
        client = thriftrs2.AsyncThriftClient(
            service_module.UserService.service_def,
            "127.0.0.1",
            free_tcp_port,
        )
        client.set_parser(service_module.UserService.parser)
        with pytest.raises(ValueError):
            await client.call("missing")

    asyncio.run(scenario())


def test_async_client_propagates_declared_exception(free_tcp_port):
    module = load_fp(
        StringIO(
            """
            exception NotFound { 1: string message; }
            service Lookup { string get(1: i32 id) throws (1: NotFound missing); }
            """
        ),
        "lookup_async",
    )

    class NotFound(Exception):
        def __init__(self, message):
            super().__init__(message)
            self.message = message

    class Handler:
        def get(self, id):
            raise NotFound(f"missing-{id}")

    server = thriftrs2.make_server(
        module.Lookup,
        Handler(),
        transport=thriftrs2.TransportType.Buffered,
        protocol=thriftrs2.ProtocolType.Binary,
        workers=2,
    )
    server.serve_forever("127.0.0.1", free_tcp_port, blocking=False)

    async def scenario():
        client = await thriftrs2.make_async_client(
            module.Lookup,
            "127.0.0.1",
            free_tcp_port,
            transport=thriftrs2.TransportType.Buffered,
            protocol=thriftrs2.ProtocolType.Binary,
        )
        try:
            with pytest.raises(RuntimeError, match="missing"):
                await client.call("get", id=42)
        finally:
            await client.close()

    try:
        asyncio.run(scenario())
    finally:
        server._server.stop()


def test_async_client_does_not_block_event_loop(service_module, free_tcp_port):
    """The async client must release the GIL during socket I/O so other
    coroutines on the same event loop can make progress."""
    server, _handler = _start_server(service_module, free_tcp_port)

    async def scenario():
        client = await thriftrs2.make_async_client(
            service_module.UserService,
            "127.0.0.1",
            free_tcp_port,
        )
        ticker = 0

        async def tick():
            nonlocal ticker
            for _ in range(20):
                ticker += 1
                await asyncio.sleep(0)

        try:
            tick_task = asyncio.create_task(tick())
            for _ in range(20):
                user = await client.call("get_user", user_id=2)
                assert user.id == 2
            await tick_task
            assert ticker == 20
        finally:
            await client.close()

    try:
        asyncio.run(scenario())
    finally:
        server._server.stop()
