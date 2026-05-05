from __future__ import annotations

import json

import pytest

import thriftrs2


PROTOCOLS = [
    thriftrs2.ProtocolType.Binary,
    thriftrs2.ProtocolType.Compact,
    thriftrs2.ProtocolType.JSON,
]


def _normalize(value):
    if hasattr(value, "to_dict"):
        return _normalize(value.to_dict())
    if isinstance(value, dict):
        return {key: _normalize(item) for key, item in value.items()}
    if isinstance(value, list):
        return [_normalize(item) for item in value]
    return value


@pytest.mark.parametrize("proto", PROTOCOLS)
def test_primitives_round_trip_all_protocols(primitives_module, primitive_data, proto):
    encoded = thriftrs2.serialize(primitives_module.PrimitiveValues, primitive_data, proto=proto)
    decoded = thriftrs2.deserialize(primitives_module.PrimitiveValues, encoded, proto=proto)
    assert _normalize(decoded) == primitive_data


@pytest.mark.parametrize("proto", PROTOCOLS)
def test_containers_round_trip_all_protocols(containers_module, container_data, proto):
    encoded = thriftrs2.serialize(containers_module.ContainerValues, container_data, proto=proto)
    decoded = thriftrs2.deserialize(containers_module.ContainerValues, encoded, proto=proto)
    assert _normalize(decoded) == container_data


@pytest.mark.parametrize("proto", PROTOCOLS)
def test_all_types_round_trip_all_protocols(all_types_module, all_types_data, proto):
    encoded = thriftrs2.serialize(all_types_module.AllTypes, all_types_data, proto=proto)
    decoded = thriftrs2.deserialize(all_types_module.AllTypes, encoded, proto=proto)
    assert _normalize(decoded) == all_types_data


def test_binary_static_protocol_round_trip(primitives_module, primitive_data):
    encoded = thriftrs2.BinaryProtocol.serialize_struct(primitives_module.PrimitiveValues, primitive_data)
    decoded = thriftrs2.BinaryProtocol.deserialize_struct(primitives_module.PrimitiveValues, encoded)
    assert decoded.to_dict() == primitive_data


def test_compact_static_protocol_round_trip(primitives_module, primitive_data):
    encoded = thriftrs2.CompactProtocol.serialize_struct(primitives_module.PrimitiveValues, primitive_data)
    decoded = thriftrs2.CompactProtocol.deserialize_struct(primitives_module.PrimitiveValues, encoded)
    assert decoded.to_dict() == primitive_data


def test_json_static_protocol_round_trip(primitives_module, primitive_data):
    encoded = thriftrs2.JSONProtocol.serialize_struct(primitives_module.PrimitiveValues, primitive_data)
    decoded = thriftrs2.JSONProtocol.deserialize_struct(primitives_module.PrimitiveValues, encoded)
    assert decoded.to_dict() == primitive_data


def test_struct_instance_can_be_serialized(all_types_module, all_types_data):
    instance = all_types_module.AllTypes(**all_types_data)
    encoded = thriftrs2.serialize(all_types_module.AllTypes, instance, proto=thriftrs2.ProtocolType.JSON)
    decoded = thriftrs2.deserialize(all_types_module.AllTypes, encoded, proto=thriftrs2.ProtocolType.JSON)
    assert _normalize(decoded) == all_types_data


def test_empty_struct_round_trip(thrift_files):
    module = thriftrs2.load(str(thrift_files.empty))
    encoded = thriftrs2.serialize(module.Empty, {}, proto=thriftrs2.ProtocolType.JSON)
    assert json.loads(encoded.decode("utf-8")) == {}
    assert thriftrs2.deserialize(module.Empty, encoded, proto=thriftrs2.ProtocolType.JSON) == {}


def test_json_dumps_returns_text(all_types_module, all_types_data):
    text = thriftrs2.dumps(all_types_module.AllTypes, all_types_data)
    assert isinstance(text, str)
    assert json.loads(text)["5"] == ["i64", str(all_types_data["long_value"])]


def test_json_loads_accepts_text(all_types_module, all_types_data):
    text = thriftrs2.dumps(all_types_module.AllTypes, all_types_data)
    decoded = thriftrs2.loads(all_types_module.AllTypes, text)
    assert _normalize(decoded) == all_types_data


def test_json_loads_accepts_bytes(all_types_module, all_types_data):
    text = thriftrs2.dumps(all_types_module.AllTypes, all_types_data)
    decoded = thriftrs2.loads(all_types_module.AllTypes, text.encode("utf-8"))
    assert _normalize(decoded) == all_types_data


def test_json_binary_payload_is_base64(primitives_module, primitive_data):
    text = thriftrs2.dumps(primitives_module.PrimitiveValues, primitive_data)
    parsed = json.loads(text)
    assert parsed["8"] == ["str", "YWJjAHh5eg=="]


def test_json_numeric_map_keys_restore_as_ints(containers_module, container_data):
    decoded = thriftrs2.loads(
        containers_module.ContainerValues,
        thriftrs2.dumps(containers_module.ContainerValues, container_data),
    )
    assert decoded["numeric_names"] == container_data["numeric_names"]


def test_protocol_outputs_are_bytes(primitives_module, primitive_data):
    for proto in PROTOCOLS:
        encoded = thriftrs2.serialize(primitives_module.PrimitiveValues, primitive_data, proto=proto)
        assert isinstance(encoded, bytes)
        assert encoded


def test_optional_fields_default_to_none(primitives_module, primitive_data):
    data = dict(primitive_data)
    data.pop("payload")
    decoded = thriftrs2.loads(primitives_module.PrimitiveValues, thriftrs2.dumps(primitives_module.PrimitiveValues, data))
    assert decoded["payload"] is None


@pytest.mark.parametrize("proto", PROTOCOLS)
def test_optional_none_is_omitted_for_dict(all_types_module, all_types_data, proto):
    data = dict(all_types_data)
    data["note"] = None
    encoded = thriftrs2.serialize(all_types_module.AllTypes, data, proto=proto)
    decoded = thriftrs2.deserialize(all_types_module.AllTypes, encoded, proto=proto)
    assert decoded["note"] is None


def test_required_none_raises_type_error(primitives_module, primitive_data):
    data = dict(primitive_data)
    data["label"] = None
    with pytest.raises(TypeError, match="Required field 'label' cannot be None"):
        thriftrs2.dumps(primitives_module.PrimitiveValues, data)
