from __future__ import annotations

from io import StringIO

import pytest

import thriftrs2
from thriftrs2.loader import load_fp


def test_public_all_contains_core_exports():
    for name in ["load", "make_client", "make_server", "ProtocolType", "JSONProtocol", "dumps", "loads"]:
        assert name in thriftrs2.__all__


def test_thrift_module_exposes_struct_attributes(all_types_module):
    assert all_types_module.AllTypes.name == "AllTypes"
    assert all_types_module.Child.name == "Child"


def test_thrift_module_exposes_service_attributes(all_types_module):
    assert all_types_module.FixtureService.service_def.name == "FixtureService"


def test_thrift_struct_repr_includes_name(all_types_module):
    assert "AllTypes" in repr(all_types_module.AllTypes)


def test_thrift_field_repr_includes_name(all_types_module):
    assert "flag" in repr(all_types_module.AllTypes.fields[0])


def test_new_instance_defaults_all_fields_to_none(all_types_module):
    instance = all_types_module.Child.new_instance()
    assert instance.to_dict() == {"name": None}


def test_struct_call_builds_instance(all_types_module):
    instance = all_types_module.Child(name="Ada")
    assert instance.struct_name == "Child"
    assert instance.name == "Ada"


def test_new_instance_from_dict_builds_instance(all_types_module):
    instance = all_types_module.Child.new_instance_from_dict({"name": "Lin"})
    assert instance.to_dict() == {"name": "Lin"}


def test_instance_attribute_setter_updates_value(all_types_module):
    instance = all_types_module.Child(name="old")
    instance.name = "new"
    assert instance.to_dict() == {"name": "new"}


def test_instance_unknown_getattr_raises(all_types_module):
    instance = all_types_module.Child(name="Ada")
    with pytest.raises(AttributeError):
        _ = instance.missing


def test_instance_unknown_setattr_raises(all_types_module):
    instance = all_types_module.Child(name="Ada")
    with pytest.raises(AttributeError):
        instance.missing = 1


def test_instance_repr_is_stable(all_types_module):
    assert repr(all_types_module.Child(name="Ada")) == "Child(name='Ada')"


def test_to_dict_preserves_none_optional_fields(all_types_module, all_types_data):
    data = dict(all_types_data)
    data.pop("note")
    instance = all_types_module.AllTypes(**data)
    assert instance.to_dict()["note"] is None


def test_load_fp_returns_module():
    module = load_fp(StringIO("struct Inline { 1: required i32 id; }"), "inline_mod")
    assert module.name == "inline_mod"
    assert module.Inline(id=5).to_dict() == {"id": 5}


def test_protocol_wrappers_store_transport_object():
    marker = object()
    assert thriftrs2.TBinaryProtocol(marker).trans is marker
    assert thriftrs2.TCompactProtocol(marker).trans is marker


def test_make_server_registers_object_handlers(service_module):
    class Handler:
        def get_user(self, user_id):
            return service_module.User(id=user_id, name="n", email=None)

    server = thriftrs2.make_server(service_module.UserService, Handler(), workers=1)
    assert server._server.is_running() is False
    assert server._server.protocol == thriftrs2.ProtocolType.Binary
    server._server.stop()


def test_make_server_registers_dict_handlers(service_module):
    def list_users():
        return []

    server = thriftrs2.make_server(service_module.UserService, {"list_users": list_users})
    assert server._server.transport == thriftrs2.TransportType.Buffered
    server._server.stop()


def test_raw_client_exposes_transport_and_protocol(service_module):
    client = thriftrs2.ThriftClient(
        service_module.UserService.service_def,
        "127.0.0.1",
        1,
        thriftrs2.TransportType.Framed,
        thriftrs2.ProtocolType.JSON,
    )
    assert client.transport == thriftrs2.TransportType.Framed
    assert client.protocol == thriftrs2.ProtocolType.JSON
    client.close()


def test_raw_client_setters_update_transport_and_protocol(service_module):
    client = thriftrs2.ThriftClient(service_module.UserService.service_def, "127.0.0.1", 1)
    client.transport = thriftrs2.TransportType.Buffered
    client.protocol = thriftrs2.ProtocolType.Compact
    assert client.transport == thriftrs2.TransportType.Buffered
    assert client.protocol == thriftrs2.ProtocolType.Compact
    client.close()


def test_unopened_client_call_raises(service_module):
    client = thriftrs2.ThriftClient(service_module.UserService.service_def, "127.0.0.1", 1)
    with pytest.raises(OSError):
        client.call("list_users")


def test_unknown_client_method_raises(service_module):
    client = thriftrs2.ThriftClient(service_module.UserService.service_def, "127.0.0.1", 1)
    with pytest.raises(ValueError):
        client.call("missing")


def test_server_setters_update_transport_protocol_workers(service_module):
    server = thriftrs2.ThriftServer(service_module.UserService.service_def)
    server.transport = thriftrs2.TransportType.Framed
    server.protocol = thriftrs2.ProtocolType.JSON
    server.workers = 2
    assert server.transport == thriftrs2.TransportType.Framed
    assert server.protocol == thriftrs2.ProtocolType.JSON
    assert server.workers == 2
    server.stop()
