from __future__ import annotations

import json

import pytest

import thriftrs2


PROTOCOLS = [
    thriftrs2.ProtocolType.Binary,
    thriftrs2.ProtocolType.Compact,
    thriftrs2.ProtocolType.JSON,
]


def _minimal_primitives(**overrides):
    data = {
        "flag": False,
        "tiny": 0,
        "short_value": 0,
        "int_value": 0,
        "long_value": 0,
        "ratio": 0.0,
        "label": "",
        "payload": b"",
    }
    data.update(overrides)
    return data


@pytest.mark.parametrize("tiny", [-128, -1, 0, 1, 127])
def test_byte_boundaries_json(primitives_module, tiny):
    data = _minimal_primitives(tiny=tiny)
    assert thriftrs2.loads(primitives_module.PrimitiveValues, thriftrs2.dumps(primitives_module.PrimitiveValues, data))["tiny"] == tiny


@pytest.mark.parametrize("short_value", [-32768, -1, 0, 1, 32767])
def test_i16_boundaries_json(primitives_module, short_value):
    data = _minimal_primitives(short_value=short_value)
    assert thriftrs2.loads(primitives_module.PrimitiveValues, thriftrs2.dumps(primitives_module.PrimitiveValues, data))["short_value"] == short_value


@pytest.mark.parametrize("int_value", [-(2**31), -1, 0, 1, 2**31 - 1])
def test_i32_boundaries_json(primitives_module, int_value):
    data = _minimal_primitives(int_value=int_value)
    assert thriftrs2.loads(primitives_module.PrimitiveValues, thriftrs2.dumps(primitives_module.PrimitiveValues, data))["int_value"] == int_value


@pytest.mark.parametrize("long_value", [-(2**63) + 1, -1, 0, 1, 2**63 - 1])
def test_i64_boundaries_json(primitives_module, long_value):
    data = _minimal_primitives(long_value=long_value)
    assert thriftrs2.loads(primitives_module.PrimitiveValues, thriftrs2.dumps(primitives_module.PrimitiveValues, data))["long_value"] == long_value


@pytest.mark.parametrize("ratio", [-1.5, -0.0, 0.0, 1.5, 1.7976931348623157e308])
def test_double_boundaries_json(primitives_module, ratio):
    data = _minimal_primitives(ratio=ratio)
    decoded = thriftrs2.loads(primitives_module.PrimitiveValues, thriftrs2.dumps(primitives_module.PrimitiveValues, data))
    assert decoded["ratio"] == ratio


def test_unicode_string_round_trip(primitives_module):
    data = _minimal_primitives(label="hello 世界")
    assert thriftrs2.loads(primitives_module.PrimitiveValues, thriftrs2.dumps(primitives_module.PrimitiveValues, data))["label"] == "hello 世界"


def test_binary_payload_round_trip_all_byte_values(primitives_module):
    payload = bytes(range(256))
    data = _minimal_primitives(payload=payload)
    decoded = thriftrs2.loads(primitives_module.PrimitiveValues, thriftrs2.dumps(primitives_module.PrimitiveValues, data))
    assert decoded["payload"] == payload


def test_empty_containers_round_trip(containers_module):
    data = {
        "numbers": [],
        "tags": [],
        "counters": {},
        "addresses": [],
        "numeric_names": {},
        "grouped": {},
    }
    decoded = thriftrs2.loads(containers_module.ContainerValues, thriftrs2.dumps(containers_module.ContainerValues, data))
    assert decoded == data


def test_unknown_json_field_is_skipped(primitives_module, primitive_data):
    parsed = json.loads(thriftrs2.dumps(primitives_module.PrimitiveValues, primitive_data))
    parsed["99"] = ["lst", ["i32", 3, 1, 2, 3]]
    decoded = thriftrs2.loads(primitives_module.PrimitiveValues, json.dumps(parsed))
    assert decoded == primitive_data


def test_unknown_json_nested_map_field_is_skipped(primitives_module, primitive_data):
    parsed = json.loads(thriftrs2.dumps(primitives_module.PrimitiveValues, primitive_data))
    parsed["99"] = ["map", ["str", "lst", 1, {"ignored": ["i32", 2, 10, 11]}]]
    decoded = thriftrs2.loads(primitives_module.PrimitiveValues, json.dumps(parsed))
    assert decoded == primitive_data


def test_invalid_json_raises_value_error(primitives_module):
    with pytest.raises(Exception):
        thriftrs2.loads(primitives_module.PrimitiveValues, "not json")


@pytest.mark.parametrize("proto", PROTOCOLS)
def test_truncated_payload_raises(primitives_module, primitive_data, proto):
    encoded = thriftrs2.serialize(primitives_module.PrimitiveValues, primitive_data, proto=proto)
    with pytest.raises(Exception):
        thriftrs2.deserialize(primitives_module.PrimitiveValues, encoded[:-1], proto=proto)


def test_i8_overflow_raises(primitives_module):
    with pytest.raises((OverflowError, TypeError, ValueError)):
        thriftrs2.dumps(primitives_module.PrimitiveValues, _minimal_primitives(tiny=128))


def test_i16_overflow_raises(primitives_module):
    with pytest.raises((OverflowError, TypeError, ValueError)):
        thriftrs2.dumps(primitives_module.PrimitiveValues, _minimal_primitives(short_value=32768))


def test_i32_overflow_raises(primitives_module):
    with pytest.raises((OverflowError, TypeError, ValueError)):
        thriftrs2.dumps(primitives_module.PrimitiveValues, _minimal_primitives(int_value=2**31))


def test_wrong_container_type_raises(containers_module, container_data):
    data = dict(container_data)
    data["numbers"] = {"not": "a list"}
    with pytest.raises(Exception):
        thriftrs2.dumps(containers_module.ContainerValues, data)


def test_wrong_struct_type_raises(containers_module, container_data):
    data = dict(container_data)
    data["addresses"] = [123]
    with pytest.raises(Exception):
        thriftrs2.dumps(containers_module.ContainerValues, data)


def test_deserialize_with_wrong_protocol_raises(primitives_module, primitive_data):
    encoded = thriftrs2.serialize(primitives_module.PrimitiveValues, primitive_data, proto=thriftrs2.ProtocolType.Binary)
    with pytest.raises(Exception):
        thriftrs2.deserialize(primitives_module.PrimitiveValues, encoded, proto=thriftrs2.ProtocolType.JSON)
