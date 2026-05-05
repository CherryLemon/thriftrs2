from __future__ import annotations

from io import StringIO

import pytest

import thriftrs2
from thriftrs2.loader import load_fp


pytestmark = pytest.mark.rpc


SUPPORTED_RPC_COMBINATIONS = [
    (thriftrs2.TransportType.Buffered, thriftrs2.ProtocolType.Binary),
    (thriftrs2.TransportType.Buffered, thriftrs2.ProtocolType.JSON),
    (thriftrs2.TransportType.Framed, thriftrs2.ProtocolType.Binary),
    (thriftrs2.TransportType.Framed, thriftrs2.ProtocolType.Compact),
    (thriftrs2.TransportType.Framed, thriftrs2.ProtocolType.JSON),
]


class RecordingHandler:
    def __init__(self, user_struct):
        self.user_struct = user_struct
        self.notifications = []
        self.ping_count = 0

    def get_user(self, user_id):
        return self.user_struct(id=user_id, name=f"user-{user_id}", email=None)

    def create_user(self, user):
        return user.id > 0 and bool(user.name)

    async def list_users(self):
        return [self.user_struct(id=1, name="Alice", email="alice@example.com")]

    def ping(self):
        self.ping_count += 1
        return None

    def notify(self, message):
        self.notifications.append(message)
        return None


def _start_server(service_module, transport, protocol, port, workers=2):
    handler = RecordingHandler(service_module.User)
    server = thriftrs2.make_server(
        service_module.UserService,
        handler,
        transport=transport,
        protocol=protocol,
        workers=workers,
    )
    server.serve_forever("127.0.0.1", port, blocking=False)
    return server, handler


@pytest.mark.parametrize(("transport", "protocol"), SUPPORTED_RPC_COMBINATIONS)
def test_rpc_get_user_round_trip(service_module, free_tcp_port, transport, protocol):
    server, _handler = _start_server(service_module, transport, protocol, free_tcp_port)
    client = None
    try:
        client = thriftrs2.make_client(
            service_module.UserService,
            "127.0.0.1",
            free_tcp_port,
            transport=transport,
            protocol=protocol,
        )
        user = client.call("get_user", user_id=7)
        assert user.to_dict() == {"id": 7, "name": "user-7", "email": None}
    finally:
        if client is not None:
            client.close()
        server._server.stop()


@pytest.mark.parametrize(("transport", "protocol"), SUPPORTED_RPC_COMBINATIONS)
def test_rpc_struct_argument_and_bool_return(service_module, free_tcp_port, transport, protocol):
    server, _handler = _start_server(service_module, transport, protocol, free_tcp_port)
    client = None
    try:
        client = thriftrs2.make_client(
            service_module.UserService,
            "127.0.0.1",
            free_tcp_port,
            transport=transport,
            protocol=protocol,
        )
        assert client.call("create_user", user=service_module.User(id=3, name="Carol", email=None)) is True
        assert client.call("create_user", user=service_module.User(id=0, name="", email=None)) is False
    finally:
        if client is not None:
            client.close()
        server._server.stop()


@pytest.mark.parametrize(("transport", "protocol"), SUPPORTED_RPC_COMBINATIONS)
def test_rpc_async_list_return(service_module, free_tcp_port, transport, protocol):
    server, _handler = _start_server(service_module, transport, protocol, free_tcp_port)
    client = None
    try:
        client = thriftrs2.make_client(
            service_module.UserService,
            "127.0.0.1",
            free_tcp_port,
            transport=transport,
            protocol=protocol,
        )
        users = client.call("list_users")
        assert [user.to_dict() for user in users] == [
            {"id": 1, "name": "Alice", "email": "alice@example.com"}
        ]
    finally:
        if client is not None:
            client.close()
        server._server.stop()


def test_rpc_void_method_returns_none(service_module, free_tcp_port):
    server, handler = _start_server(
        service_module,
        thriftrs2.TransportType.Buffered,
        thriftrs2.ProtocolType.JSON,
        free_tcp_port,
    )
    client = None
    try:
        client = thriftrs2.make_client(
            service_module.UserService,
            "127.0.0.1",
            free_tcp_port,
            protocol=thriftrs2.ProtocolType.JSON,
        )
        assert client.call("ping") is None
        assert handler.ping_count == 1
    finally:
        if client is not None:
            client.close()
        server._server.stop()


def test_rpc_oneway_method_returns_none(service_module, free_tcp_port):
    server, _handler = _start_server(
        service_module,
        thriftrs2.TransportType.Buffered,
        thriftrs2.ProtocolType.Binary,
        free_tcp_port,
    )
    client = None
    try:
        client = thriftrs2.make_client(service_module.UserService, "127.0.0.1", free_tcp_port)
        assert client.call("notify", message="sent") is None
    finally:
        if client is not None:
            client.close()
        server._server.stop()


def test_rpc_context_manager_closes_client(service_module, free_tcp_port):
    server, _handler = _start_server(
        service_module,
        thriftrs2.TransportType.Buffered,
        thriftrs2.ProtocolType.Binary,
        free_tcp_port,
    )
    try:
        with thriftrs2.make_client(service_module.UserService, "127.0.0.1", free_tcp_port) as client:
            assert client.call("get_user", user_id=4).id == 4
    finally:
        server._server.stop()


def test_rpc_unknown_method_fails_before_network(service_module, free_tcp_port):
    server, _handler = _start_server(
        service_module,
        thriftrs2.TransportType.Buffered,
        thriftrs2.ProtocolType.Binary,
        free_tcp_port,
    )
    client = None
    try:
        client = thriftrs2.make_client(service_module.UserService, "127.0.0.1", free_tcp_port)
        with pytest.raises(ValueError):
            client.call("missing")
    finally:
        if client is not None:
            client.close()
        server._server.stop()


def test_rpc_connect_failure_raises(service_module, free_tcp_port):
    with pytest.raises(OSError):
        thriftrs2.make_client(service_module.UserService, "127.0.0.1", free_tcp_port)


def test_rpc_buffered_compact_reports_unsupported(service_module, free_tcp_port):
    server, _handler = _start_server(
        service_module,
        thriftrs2.TransportType.Buffered,
        thriftrs2.ProtocolType.Compact,
        free_tcp_port,
    )
    client = None
    try:
        client = thriftrs2.make_client(
            service_module.UserService,
            "127.0.0.1",
            free_tcp_port,
            transport=thriftrs2.TransportType.Buffered,
            protocol=thriftrs2.ProtocolType.Compact,
        )
        with pytest.raises(OSError):
            client.call("get_user", user_id=1)
    finally:
        if client is not None:
            client.close()
        server._server.stop()


def test_rpc_server_running_flag_changes(service_module, free_tcp_port):
    server, _handler = _start_server(
        service_module,
        thriftrs2.TransportType.Buffered,
        thriftrs2.ProtocolType.Binary,
        free_tcp_port,
    )
    try:
        assert server._server.is_running() is True
    finally:
        server._server.stop()


@pytest.mark.parametrize("protocol", [thriftrs2.ProtocolType.Binary, thriftrs2.ProtocolType.JSON])
def test_rpc_declared_exception_round_trip(free_tcp_port, protocol):
    module = load_fp(
        StringIO(
            """
            exception NotFound { 1: string message; }
            service Lookup { string get(1: i32 id) throws (1: NotFound missing); }
            """
        ),
        "lookup",
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
        protocol=protocol,
        workers=2,
    )
    server.serve_forever("127.0.0.1", free_tcp_port, blocking=False)
    client = None
    try:
        client = thriftrs2.make_client(
            module.Lookup,
            "127.0.0.1",
            free_tcp_port,
            transport=thriftrs2.TransportType.Buffered,
            protocol=protocol,
        )
        with pytest.raises(RuntimeError, match="missing") as exc_info:
            client.call("get", id=7)
        assert "missing-7" in str(exc_info.value)
    finally:
        if client is not None:
            client.close()
        server._server.stop()
