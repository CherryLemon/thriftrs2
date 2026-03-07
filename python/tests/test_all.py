from pathlib import Path

import thriftrs2


EXAMPLE_THRIFT = Path(__file__).resolve().parents[2] / "examples" / "example.thrift"


def test_public_exports_include_json_protocol():
    assert thriftrs2.JSONProtocol is not None


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
