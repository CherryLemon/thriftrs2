from importlib.metadata import PackageNotFoundError, version

from .thriftrs2 import (
    ThriftParser,
    BinaryProtocol,
    CompactProtocol,
    JSONProtocol,
    ProtocolType,
    ThriftStruct,
    ThriftField,
    PyThriftService,
    PyThriftMethod,
    ThriftServer,
    TransportType,
    ThriftStructInstance,
    ThriftClient,
    ThriftApplicationException,
)
from .loader import load, make_client, make_server
from .protocol import TBinaryProtocol, TCompactProtocol, TFramedTransport, TBufferedTransport, serialize, deserialize, loads, dumps

try:
    __version__ = version("thriftrs2")
except PackageNotFoundError:
    __version__ = "0.0.0+local"

__all__ = [
    "__version__",
    "ThriftParser",
    "BinaryProtocol",
    "CompactProtocol",
    "JSONProtocol",
    "ProtocolType",
    "ThriftStruct",
    "ThriftField",
    "PyThriftService",
    "PyThriftMethod",
    "ThriftServer",
    "TransportType",
    "ThriftStructInstance",
    "ThriftClient",
    "ThriftApplicationException",
    "TFramedTransport",
    "TBufferedTransport",
    "load",
    "make_client",
    "make_server",
    "deserialize",
    "serialize",
    "loads",
    "dumps",
    "TBinaryProtocol",
    "TCompactProtocol",
]
