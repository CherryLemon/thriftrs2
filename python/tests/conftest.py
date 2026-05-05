from __future__ import annotations

from dataclasses import dataclass
from pathlib import Path
import socket
from typing import Any

import pytest

import thriftrs2


@dataclass(frozen=True)
class ThriftTestFiles:
    primitives: Path
    containers: Path
    all_types: Path
    service: Path
    empty: Path


@pytest.fixture(scope="session")
def thrift_files(tmp_path_factory: pytest.TempPathFactory) -> ThriftTestFiles:
    root = tmp_path_factory.mktemp("thrift_idl")

    primitives = root / "primitives.thrift"
    primitives.write_text(
        """
struct PrimitiveValues {
    1: required bool flag;
    2: required byte tiny;
    3: required i16 short_value;
    4: required i32 int_value;
    5: required i64 long_value;
    6: required double ratio;
    7: required string label;
    8: required binary payload;
}
""",
        encoding="utf-8",
    )

    containers = root / "containers.thrift"
    containers.write_text(
        """
struct Address {
    1: required string city;
    2: optional string street;
}

struct ContainerValues {
    1: required list<i32> numbers;
    2: required set<string> tags;
    3: required map<string, i64> counters;
    4: required list<Address> addresses;
    5: required map<i32, string> numeric_names;
    6: optional map<string, list<i32>> grouped;
}
""",
        encoding="utf-8",
    )

    all_types = root / "all_types.thrift"
    all_types.write_text(
        """
# leading comment
struct Empty {
}

struct Child {
    1: required string name = "default";
}

struct AllTypes {
    1: required bool flag;
    2: required byte tiny;
    3: required i16 short_value;
    4: required i32 int_value;
    5: required i64 long_value;
    6: required double ratio;
    7: required string label;
    8: required binary payload;
    9: required list<i32> numbers;
    10: required set<string> tags;
    11: required map<string, i64> counters;
    12: required Child child;
    13: optional string note;
}

service FixtureService {
    AllTypes echo(1: AllTypes value);
    list<AllTypes> list_values();
    bool save(1: AllTypes value);
    void ping();
    oneway void notify(1: string message)
}
""",
        encoding="utf-8",
    )

    service = root / "service.thrift"
    service.write_text(
        """
struct User {
    1: required i32 id;
    2: required string name;
    3: optional string email;
}

service UserService {
    User get_user(1: i32 user_id);
    bool create_user(1: User user);
    list<User> list_users();
    void ping();
    oneway void notify(1: string message)
}
""",
        encoding="utf-8",
    )

    empty = root / "empty.thrift"
    empty.write_text("struct Empty {\n}\n", encoding="utf-8")

    return ThriftTestFiles(
        primitives=primitives,
        containers=containers,
        all_types=all_types,
        service=service,
        empty=empty,
    )


@pytest.fixture(scope="session")
def primitives_module(thrift_files: ThriftTestFiles):
    return thriftrs2.load(str(thrift_files.primitives))


@pytest.fixture(scope="session")
def containers_module(thrift_files: ThriftTestFiles):
    return thriftrs2.load(str(thrift_files.containers))


@pytest.fixture(scope="session")
def all_types_module(thrift_files: ThriftTestFiles):
    return thriftrs2.load(str(thrift_files.all_types))


@pytest.fixture(scope="session")
def service_module(thrift_files: ThriftTestFiles):
    return thriftrs2.load(str(thrift_files.service))


@pytest.fixture
def primitive_data() -> dict[str, Any]:
    return {
        "flag": True,
        "tiny": -12,
        "short_value": -1234,
        "int_value": 123456,
        "long_value": 9_223_372_036_854_775_000,
        "ratio": 3.5,
        "label": "hello",
        "payload": b"abc\x00xyz",
    }


@pytest.fixture
def container_data() -> dict[str, Any]:
    return {
        "numbers": [1, 2, 3],
        "tags": ["red", "blue"],
        "counters": {"large": 9_223_372_036_854_775_000, "small": 7},
        "addresses": [{"city": "Hangzhou", "street": "Wenyi"}],
        "numeric_names": {1: "one", -2: "minus two"},
        "grouped": {"a": [1, 2], "b": [3]},
    }


@pytest.fixture
def all_types_data() -> dict[str, Any]:
    return {
        "flag": False,
        "tiny": 42,
        "short_value": 1234,
        "int_value": -123456,
        "long_value": -9_000_000_000_000,
        "ratio": 1.25,
        "label": "all",
        "payload": b"payload",
        "numbers": [5, 8, 13],
        "tags": ["alpha", "beta"],
        "counters": {"x": 1, "y": 2},
        "child": {"name": "nested"},
        "note": None,
    }


@pytest.fixture
def free_tcp_port() -> int:
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as sock:
        sock.bind(("127.0.0.1", 0))
        return sock.getsockname()[1]
