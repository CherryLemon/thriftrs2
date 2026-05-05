from __future__ import annotations

import json
import os
import time

import thriftpy2
import pytest
from thriftpy2.protocol import TBinaryProtocolFactory, TJSONProtocolFactory
from thriftpy2.utils import deserialize as tp2_deserialize
from thriftpy2.utils import serialize as tp2_serialize

import thriftrs2


def _normalize(value):
    if hasattr(value, "to_dict"):
        return _normalize(value.to_dict())
    if isinstance(value, dict):
        return {key: _normalize(item) for key, item in value.items()}
    if isinstance(value, list):
        return [_normalize(item) for item in value]
    return value


def _load_tp2(path):
    module_name = f"golden_{os.getpid()}_{time.time_ns()}_thrift"
    return thriftpy2.load(str(path), module_name=module_name)


def _tp2_primitive(mod, data):
    return mod.PrimitiveValues(**data)


def test_binary_helper_and_static_api_match(primitives_module, primitive_data):
    helper = thriftrs2.serialize(primitives_module.PrimitiveValues, primitive_data)
    direct = thriftrs2.BinaryProtocol.serialize_struct(primitives_module.PrimitiveValues, primitive_data)
    assert helper == direct


def test_compact_helper_and_static_api_match(primitives_module, primitive_data):
    helper = thriftrs2.serialize(
        primitives_module.PrimitiveValues,
        primitive_data,
        proto=thriftrs2.ProtocolType.Compact,
    )
    direct = thriftrs2.CompactProtocol.serialize_struct(primitives_module.PrimitiveValues, primitive_data)
    assert helper == direct


def test_json_helper_and_static_api_match(primitives_module, primitive_data):
    helper = thriftrs2.serialize(
        primitives_module.PrimitiveValues,
        primitive_data,
        proto=thriftrs2.ProtocolType.JSON,
    )
    direct = thriftrs2.JSONProtocol.serialize_struct(primitives_module.PrimitiveValues, primitive_data)
    assert json.loads(helper.decode("utf-8")) == json.loads(direct.decode("utf-8"))


def test_binary_protocol_wrapper_matches_helper(primitives_module, primitive_data):
    protocol = thriftrs2.TBinaryProtocol()
    encoded = protocol.write_struct(primitives_module.PrimitiveValues, primitive_data)
    assert encoded == thriftrs2.serialize(primitives_module.PrimitiveValues, primitive_data)
    assert protocol.read_struct(primitives_module.PrimitiveValues, encoded) == primitive_data


def test_compact_protocol_wrapper_matches_helper(primitives_module, primitive_data):
    protocol = thriftrs2.TCompactProtocol()
    encoded = protocol.write_struct(primitives_module.PrimitiveValues, primitive_data)
    helper = thriftrs2.serialize(
        primitives_module.PrimitiveValues,
        primitive_data,
        proto=thriftrs2.ProtocolType.Compact,
    )
    assert encoded == helper
    assert protocol.read_struct(primitives_module.PrimitiveValues, encoded) == primitive_data


def test_json_dumps_loads_match_json_static_api(containers_module, container_data):
    text = thriftrs2.dumps(containers_module.ContainerValues, container_data)
    direct = thriftrs2.JSONProtocol.serialize_struct(containers_module.ContainerValues, container_data)
    assert json.loads(text) == json.loads(direct.decode("utf-8"))
    decoded = thriftrs2.JSONProtocol.deserialize_struct(containers_module.ContainerValues, direct)
    assert _normalize(decoded) == _normalize(thriftrs2.loads(containers_module.ContainerValues, text))


def test_round_tripped_values_are_protocol_equivalent(all_types_module, all_types_data):
    expected = None
    for proto in (thriftrs2.ProtocolType.Binary, thriftrs2.ProtocolType.Compact, thriftrs2.ProtocolType.JSON):
        encoded = thriftrs2.serialize(all_types_module.AllTypes, all_types_data, proto=proto)
        decoded = _normalize(thriftrs2.deserialize(all_types_module.AllTypes, encoded, proto=proto))
        if expected is None:
            expected = decoded
        assert decoded == expected


def test_protocol_enum_members_are_distinct():
    assert thriftrs2.ProtocolType.Binary != thriftrs2.ProtocolType.Compact
    assert thriftrs2.ProtocolType.Compact != thriftrs2.ProtocolType.JSON
    assert repr(thriftrs2.ProtocolType.JSON)


def test_transport_enum_members_are_distinct():
    assert thriftrs2.TransportType.Framed != thriftrs2.TransportType.Buffered
    assert thriftrs2.TFramedTransport.transport_type == thriftrs2.TransportType.Framed
    assert thriftrs2.TBufferedTransport.transport_type == thriftrs2.TransportType.Buffered


def test_binary_golden_thriftpy2_to_thriftrs2(thrift_files, primitives_module, primitive_data):
    tp2_mod = _load_tp2(thrift_files.primitives)
    encoded = tp2_serialize(_tp2_primitive(tp2_mod, primitive_data), proto_factory=TBinaryProtocolFactory())

    decoded = thriftrs2.deserialize(primitives_module.PrimitiveValues, encoded)

    assert _normalize(decoded) == primitive_data


def test_binary_golden_thriftrs2_to_thriftpy2(thrift_files, primitives_module, primitive_data):
    tp2_mod = _load_tp2(thrift_files.primitives)
    encoded = thriftrs2.serialize(primitives_module.PrimitiveValues, primitive_data)

    decoded = tp2_deserialize(tp2_mod.PrimitiveValues(), encoded, proto_factory=TBinaryProtocolFactory())

    assert _normalize(decoded.__dict__) == primitive_data


def test_json_golden_thriftpy2_to_thriftrs2(thrift_files, primitives_module, primitive_data):
    tp2_mod = _load_tp2(thrift_files.primitives)
    encoded = tp2_serialize(_tp2_primitive(tp2_mod, primitive_data), proto_factory=TJSONProtocolFactory())

    decoded = thriftrs2.deserialize(primitives_module.PrimitiveValues, encoded, proto=thriftrs2.ProtocolType.JSON)

    assert _normalize(decoded) == primitive_data


@pytest.mark.xfail(reason="thriftrs2 JSON uses TJSON field-id objects, while thriftpy2 utils TJSON expects its metadata envelope")
def test_json_golden_thriftrs2_to_thriftpy2(thrift_files, primitives_module, primitive_data):
    tp2_mod = _load_tp2(thrift_files.primitives)
    encoded = thriftrs2.serialize(
        primitives_module.PrimitiveValues,
        primitive_data,
        proto=thriftrs2.ProtocolType.JSON,
    )

    decoded = tp2_deserialize(tp2_mod.PrimitiveValues(), encoded, proto_factory=TJSONProtocolFactory())

    assert _normalize(decoded.__dict__) == primitive_data
