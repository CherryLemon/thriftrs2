from typing import Any, Dict, IO
from .thrift_rs_pyo3 import BinaryProtocol, CompactProtocol, ThriftStruct, TransportType, ProtocolType


class TBinaryProtocol:
    """Binary protocol implementation compatible with thriftpy2"""

    def __init__(self, trans=None):
        self.trans = trans

    def write_struct(self, struct_def: ThriftStruct, data: Dict[str, Any]) -> bytes:
        """Serialize a struct to binary format"""
        return BinaryProtocol.serialize_struct(struct_def, data)

    def read_struct(self, struct_def: ThriftStruct, data: bytes) -> Dict[str, Any]:
        """Deserialize a struct from binary format"""
        return BinaryProtocol.deserialize_struct(struct_def, data).to_dict()


class TBinaryProtocolFactory:
    """Factory for creating binary protocol instances"""

    def get_protocol(self, trans):
        return TBinaryProtocol(trans)


class TCompactProtocol:
    """Compact protocol implementation compatible with thriftpy2"""

    def __init__(self, trans=None):
        self.trans = trans

    def write_struct(self, struct_def: ThriftStruct, data: Dict[str, Any]) -> bytes:
        """Serialize a struct to compact format"""
        return CompactProtocol.serialize_struct(struct_def, data)

    def read_struct(self, struct_def: ThriftStruct, data: bytes) -> Dict[str, Any]:
        """Deserialize a struct from compact format"""
        return CompactProtocol.deserialize_struct(struct_def, data).to_dict()


class TCompactProtocolFactory:
    """Factory for creating compact protocol instances"""

    def get_protocol(self, trans):
        return TCompactProtocol(trans)


# ---------------------------------------------------------------------------
# Transport helpers
# ---------------------------------------------------------------------------

class TFramedTransport:
    """
    Thin wrapper that tags a host/port pair as using framed transport
    (TFramedTransport in the official Thrift SDKs).  Pass an instance of this
    to ThriftServer instead of a raw (host, port) tuple to select the framed
    transport mode explicitly.
    """
    transport_type = TransportType.Framed


class TBufferedTransport:
    """
    Thin wrapper that tags a host/port pair as using buffered transport
    (TBufferedTransport / TSocket in the official Thrift SDKs).  Pass an
    instance of this to ThriftServer to select buffered (non-framed) mode.
    """
    transport_type = TransportType.Buffered


# Convenience functions similar to thriftpy2 — call the Rust static methods directly


def serialize(struct_def: ThriftStruct, data: Dict[str, Any], proto: ProtocolType = ProtocolType.Binary) -> bytes:
    """Serialize struct data to target protocol format"""
    return struct_def.serialize_with_protocol(data, proto)


def deserialize(struct_def: ThriftStruct, data: bytes, proto: ProtocolType = ProtocolType.Binary) -> Dict[str, Any]:
    """Deserialize target protocol data to struct"""
    return struct_def.deserialize_with_protocol(data, proto).to_dict()

def dumps(struct_def: ThriftStruct, data: Dict[str, Any]) -> str:
    """Serialize struct data to JSON format"""
    return struct_def.serialize_with_protocol(data, ProtocolType.JSON)

def loads(struct_def: ThriftStruct, data: bytes, proto: ProtocolType = ProtocolType.Binary) -> Dict[str, Any]:
    """Deserialize target protocol data to struct"""
    return struct_def.deserialize_with_protocol(data, ProtocolType.JSON).to_dict()
