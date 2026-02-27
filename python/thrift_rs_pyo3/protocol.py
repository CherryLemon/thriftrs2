from typing import Any, Dict, IO
from .thrift_rs_pyo3 import BinaryProtocol, ThriftStruct, TransportType


class TBinaryProtocol:
    """Binary protocol implementation compatible with thriftpy2"""

    def __init__(self, trans=None):
        self.trans = trans

    def write_struct(self, struct_def: ThriftStruct, data: Dict[str, Any]) -> bytes:
        """Serialize a struct to binary format"""
        return BinaryProtocol.serialize_struct(struct_def, data)

    def read_struct(self, struct_def: ThriftStruct, data: bytes) -> Dict[str, Any]:
        """Deserialize a struct from binary format"""
        return BinaryProtocol.deserialize_struct(struct_def, data)


class TBinaryProtocolFactory:
    """Factory for creating binary protocol instances"""

    def get_protocol(self, trans):
        return TBinaryProtocol(trans)


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
# to avoid constructing any Python wrapper objects per call.
def serialize(struct_def: ThriftStruct, data: Dict[str, Any]) -> bytes:
    """Serialize struct data to binary format"""
    return BinaryProtocol.serialize_struct(struct_def, data)


def deserialize(struct_def: ThriftStruct, data: bytes) -> Dict[str, Any]:
    """Deserialize binary data to struct"""
    return BinaryProtocol.deserialize_struct(struct_def, data)
