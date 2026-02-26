from typing import Any, Dict, IO
from .thrift_rs_pyo3 import BinaryProtocol, ThriftStruct


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


# Convenience functions similar to thriftpy2 — call the Rust static methods directly
# to avoid constructing any Python wrapper objects per call.
def serialize(struct_def: ThriftStruct, data: Dict[str, Any]) -> bytes:
    """Serialize struct data to binary format"""
    return BinaryProtocol.serialize_struct(struct_def, data)


def deserialize(struct_def: ThriftStruct, data: bytes) -> Dict[str, Any]:
    """Deserialize binary data to struct"""
    return BinaryProtocol.deserialize_struct(struct_def, data)
