from typing import Any, Dict, IO
from .thrift_rs_pyo3 import BinaryProtocol, ThriftStruct


class TBinaryProtocol:
    """Binary protocol implementation compatible with thriftpy2"""

    def __init__(self, trans=None):
        self.trans = trans
        self._protocol = BinaryProtocol()

    def write_struct(self, struct_def: ThriftStruct, data: Dict[str, Any]) -> bytes:
        """Serialize a struct to binary format"""
        return self._protocol.serialize_struct(struct_def, data)

    def read_struct(self, struct_def: ThriftStruct, data: bytes) -> Dict[str, Any]:
        """Deserialize a struct from binary format"""
        return self._protocol.deserialize_struct(struct_def, data)


class TBinaryProtocolFactory:
    """Factory for creating binary protocol instances"""

    def get_protocol(self, trans):
        return TBinaryProtocol(trans)


# Convenience functions similar to thriftpy2
def serialize(struct_def: ThriftStruct, data: Dict[str, Any]) -> bytes:
    """Serialize struct data to binary format"""
    protocol = TBinaryProtocol()
    return protocol.write_struct(struct_def, data)


def deserialize(struct_def: ThriftStruct, data: bytes) -> Dict[str, Any]:
    """Deserialize binary data to struct"""
    protocol = TBinaryProtocol()
    return protocol.read_struct(struct_def, data)
