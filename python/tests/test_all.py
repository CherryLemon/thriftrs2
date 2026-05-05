import json
import socket
from pathlib import Path

import thriftrs2


EXAMPLE_THRIFT = Path(__file__).resolve().parents[2] / "examples" / "example.thrift"


def _free_tcp_port():
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as sock:
        sock.bind(("127.0.0.1", 0))
        return sock.getsockname()[1]


def test_public_exports_include_json_protocol():
    assert thriftrs2.JSONProtocol is not None


def test_version_string_is_set():
    assert isinstance(thriftrs2.__version__, str)
    assert len(thriftrs2.__version__) > 0


def test_round_trip_binary_serialization_from_example_schema():
    thrift_module = thriftrs2.load(str(EXAMPLE_THRIFT))
    user_data = {
        "id": 123,
        "name": "John Doe",
        "email": "john@example.com",
        "age": 30,
    }

    encoded = thriftrs2.serialize(thrift_module.User, user_data)
    decoded = thriftrs2.deserialize(thrift_module.User, encoded)

    assert decoded == user_data


def test_round_trip_json_serialization_from_example_schema():
    thrift_module = thriftrs2.load(str(EXAMPLE_THRIFT))
    user_data = {
        "id": 123,
        "name": "John Doe",
        "email": "john@example.com",
        "age": 30,
    }

    encoded = thriftrs2.serialize(thrift_module.User, user_data, proto=thriftrs2.ProtocolType.JSON)
    parsed = json.loads(encoded.decode("utf-8"))
    assert parsed["1"] == ["i32", 123]

    decoded = thriftrs2.deserialize(thrift_module.User, encoded, proto=thriftrs2.ProtocolType.JSON)
    assert decoded == user_data

    dumped = thriftrs2.dumps(thrift_module.User, user_data)
    assert isinstance(dumped, str)
    assert thriftrs2.loads(thrift_module.User, dumped) == user_data
    assert thriftrs2.loads(thrift_module.User, dumped.encode("utf-8")) == user_data


def test_round_trip_json_serialization_with_nested_containers(tmp_path):
    thrift_file = tmp_path / "complex.thrift"
    thrift_file.write_text(
        """
struct Address {
    1: required string city;
}

struct Profile {
    1: required list<i32> scores;
    2: required map<string, i64> counters;
    3: required set<string> tags;
    4: required binary payload;
    5: required Address address;
    6: required bool active;
    7: required double ratio;
}
""",
        encoding="utf-8",
    )
    thrift_module = thriftrs2.load(str(thrift_file))
    profile_data = {
        "scores": [1, 2, 3],
        "counters": {"large": 9223372036854775807, "small": 7},
        "tags": ["blue", "fast"],
        "payload": b"abc",
        "address": {"city": "Hangzhou"},
        "active": True,
        "ratio": 1.25,
    }

    dumped = thriftrs2.dumps(thrift_module.Profile, profile_data)
    parsed = json.loads(dumped)
    assert parsed["4"] == ["str", "YWJj"]

    decoded = thriftrs2.loads(thrift_module.Profile, dumped)
    assert decoded["scores"] == profile_data["scores"]
    assert decoded["counters"] == profile_data["counters"]
    assert decoded["tags"] == profile_data["tags"]
    assert decoded["payload"] == profile_data["payload"]
    assert decoded["address"].to_dict() == profile_data["address"]
    assert decoded["active"] is True
    assert decoded["ratio"] == profile_data["ratio"]

    decoded_instance = thriftrs2.JSONProtocol.deserialize_struct(
        thrift_module.Profile, dumped.encode("utf-8")
    )
    redumped = thriftrs2.dumps(thrift_module.Profile, decoded_instance)
    assert thriftrs2.loads(thrift_module.Profile, redumped)["address"].to_dict() == profile_data[
        "address"
    ]


def test_json_deserializes_numeric_map_keys_and_skips_unknown_fields(tmp_path):
    thrift_file = tmp_path / "maps.thrift"
    thrift_file.write_text(
        """
struct NumericMaps {
    1: required map<i32, string> names;
    2: required map<i64, string> large_names;
}
""",
        encoding="utf-8",
    )
    thrift_module = thriftrs2.load(str(thrift_file))
    data = {
        "names": {1: "one", -2: "minus two"},
        "large_names": {9223372036854775807: "max"},
    }

    dumped = thriftrs2.dumps(thrift_module.NumericMaps, data)
    decoded = thriftrs2.loads(thrift_module.NumericMaps, dumped)
    assert decoded == data

    parsed = json.loads(dumped)
    parsed["99"] = ["map", ["str", "lst", 1, {"ignored": ["i32", 2, 10, 11]}]]
    decoded_with_unknown = thriftrs2.loads(thrift_module.NumericMaps, json.dumps(parsed))
    assert decoded_with_unknown == data


def test_json_rpc_round_trip_over_buffered_transport():
    thrift_module = thriftrs2.load(str(EXAMPLE_THRIFT))
    User = thrift_module.User
    port = _free_tcp_port()

    class Handler:
        def get_user(self, user_id):
            return User(id=user_id, name="JSON User", email="json@example.com", age=41)

        def create_user(self, user):
            return user.id == 99 and user.name == "Charlie"

        async def list_users(self):
            return [User(id=1, name="Alice", email="alice@example.com", age=30)]

    server = thriftrs2.make_server(
        thrift_module.UserService,
        Handler(),
        protocol=thriftrs2.ProtocolType.JSON,
        workers=2,
    )
    server.serve_forever("127.0.0.1", port, blocking=False)

    client = None
    try:
        client = thriftrs2.make_client(
            thrift_module.UserService,
            "127.0.0.1",
            port,
            protocol=thriftrs2.ProtocolType.JSON,
        )

        user = client.call("get_user", user_id=7)
        assert user.to_dict() == {
            "id": 7,
            "name": "JSON User",
            "email": "json@example.com",
            "age": 41,
        }

        created = client.call(
            "create_user",
            user=User(id=99, name="Charlie", email="charlie@example.com", age=22),
        )
        assert created is True

        users = client.call("list_users")
        assert [item.to_dict() for item in users] == [
            {"id": 1, "name": "Alice", "email": "alice@example.com", "age": 30}
        ]
    finally:
        if client is not None:
            client.close()
        server._server.stop()
