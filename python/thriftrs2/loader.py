import asyncio
import os
import re
import threading
from typing import Any, Optional

from .thriftrs2 import ProtocolType, ThriftParser, ThriftClient, ThriftServer, TransportType


_INCLUDE_RE = re.compile(r'^\s*include\s+"([^"]+)"', re.MULTILINE)


class ThriftModule:
    """Similar to thriftpy2's thrift module representation"""

    def __init__(self, name: str, parser: ThriftParser):
        self.name = name
        self._parser = parser
        self._structs = {}
        self._services = {}
        self._enums = {}
        self._load_definitions()

    def _load_definitions(self):
        """Load struct and service definitions from parser"""
        struct_names = self._parser.list_structs()
        for struct_name in struct_names:
            struct_def = self._parser.get_struct(struct_name)
            if struct_def:
                self._structs[struct_name] = struct_def
                setattr(self, struct_name, struct_def)

        service_names = self._parser.list_services()
        for service_name in service_names:
            service_def = self._parser.get_service(service_name)
            if service_def:
                self._services[service_name] = service_def

        if hasattr(self._parser, "list_enums"):
            for enum_name in self._parser.list_enums():
                enum_values = self._parser.get_enum(enum_name)
                enum_def = ThriftEnum(enum_name, enum_values or {})
                self._enums[enum_name] = enum_def
                setattr(self, enum_name, enum_def)

    def __getattr__(self, name: str):
        if name in self._structs:
            return self._structs[name]
        if name in self._services:
            return ThriftService(self._parser, self._services[name])
        if name in self._enums:
            return self._enums[name]
        raise AttributeError(f"'{self.__class__.__name__}' object has no attribute '{name}'")


class ThriftEnum:
    def __init__(self, name: str, values: dict[str, int]):
        self.name = name
        self._values = dict(values)
        for item_name, item_value in self._values.items():
            setattr(self, item_name, item_value)

    def __getitem__(self, name: str) -> int:
        return self._values[name]

    def to_dict(self) -> dict[str, int]:
        return dict(self._values)

    def __repr__(self):
        return f"<ThriftEnum {self.name} {self._values}>"


class ThriftService:
    def __init__(self, parser: ThriftParser, service_def):
        self.parser = parser
        self.service_def = service_def

    def __repr__(self):
        return f"<ThriftService {self.service_def}>"


def _resolve_include(include_name: str, base_dir: str, include_dirs: Optional[list]) -> str:
    if os.path.isabs(include_name) and os.path.exists(include_name):
        return include_name

    search_dirs = [base_dir]
    if include_dirs:
        search_dirs.extend(include_dirs)

    for directory in search_dirs:
        candidate = os.path.abspath(os.path.join(directory, include_name))
        if os.path.exists(candidate):
            return candidate

    searched = ", ".join(os.path.abspath(path) for path in search_dirs)
    raise FileNotFoundError(f"Included thrift file not found: {include_name} (searched: {searched})")


def _read_thrift_with_includes(thrift_file: str, include_dirs: Optional[list], seen: set[str]) -> str:
    thrift_file = os.path.abspath(thrift_file)
    if thrift_file in seen:
        return ""
    seen.add(thrift_file)

    with open(thrift_file, 'r', encoding='utf-8') as f:
        content = f.read()

    base_dir = os.path.dirname(thrift_file)
    chunks = []
    for include_name in _INCLUDE_RE.findall(content):
        include_path = _resolve_include(include_name, base_dir, include_dirs)
        included_content = _read_thrift_with_includes(include_path, include_dirs, seen)
        if included_content:
            chunks.append(included_content)
    chunks.append(content)
    return "\n".join(chunks)


def load(thrift_file: str, module_name: Optional[str] = None, include_dirs: Optional[list] = None) -> ThriftModule:
    """
    Load a thrift file and return a module-like object

    Args:
        thrift_file: Path to the thrift file
        module_name: Optional module name (defaults to filename without extension)
        include_dirs: Optional list of directories to search for included files

    Returns:
        ThriftModule object with loaded definitions
    """
    if not os.path.exists(thrift_file):
        raise FileNotFoundError(f"Thrift file not found: {thrift_file}")

    if module_name is None:
        module_name = os.path.splitext(os.path.basename(thrift_file))[0]

    content = _read_thrift_with_includes(thrift_file, include_dirs, set())

    parser = ThriftParser()
    parser.parse(content)

    return ThriftModule(module_name, parser)


def load_fp(fp, module_name: str, **kwargs) -> ThriftModule:
    """Load thrift from file-like object"""
    content = fp.read()
    parser = ThriftParser()
    parser.parse(content)
    return ThriftModule(module_name, parser)


def make_client(
    service: ThriftService,
    host: str,
    port: int,
    transport: TransportType = TransportType.Buffered,
    protocol: ProtocolType = ProtocolType.Binary,
) -> ThriftClient:
    """
    Create a connected :class:`ThriftClient`.

    Analogous to ``thriftpy2.rpc.make_client``.

    Parameters
    ----------
    service : ThriftService
        Service attribute from the :class:`ThriftModule` returned by :func:`load`.
    host : str
        Remote host to connect to.
    port : int
        Remote port to connect to.
    transport : TransportType
        ``TransportType.Framed`` or ``TransportType.Buffered``
        (default ``Buffered``).
    protocol : ProtocolType
        ``ProtocolType.Binary``, ``ProtocolType.Compact`` or ``ProtocolType.JSON``
        (default ``Binary``). Client and server must use the same protocol.

    Returns
    -------
    ThriftClient
        An already-connected client instance.
    """
    client = ThriftClient(service.service_def, host, port, transport, protocol)
    client.set_parser(service.parser)
    client.open()
    return client


class ThriftServerWrapper:
    def __init__(
        self,
        service: ThriftService,
        handler,
        transport: TransportType,
        workers: int = 1,
        protocol: ProtocolType = ProtocolType.Binary,
    ):
        server = self._server = ThriftServer(service.service_def, transport, workers, protocol)
        self._server.set_parser(service.parser)
        if isinstance(handler, dict):
            for method_name, func in handler.items():
                server.register_handler(method_name, func)
        else:
            for attr_name in dir(handler):
                if service.service_def.get_method(attr_name) is None:
                    continue
                func = getattr(handler, attr_name)
                server.register_handler(attr_name, func)

        self._loop = asyncio.new_event_loop()
        self._th = threading.Thread(target=self._loop.run_forever, daemon=True)
        self._th.start()
        self._waiter = None

    def serve_forever(self, host, port, blocking=True):
        async def serve():
            self._server.serve(host, port)
            while not self._server.is_running():
                await asyncio.sleep(0.1)
            print('server started and listening on {}:{}'.format(host, port))

        async def wait():
            while self._server.is_running():
                await asyncio.sleep(1)

        asyncio.run_coroutine_threadsafe(serve(), self._loop).result()
        self._waiter = asyncio.run_coroutine_threadsafe(wait(), self._loop)
        if blocking:
            self._waiter.result()


def make_server(
    service: ThriftService,
    handler: Any,
    transport: TransportType = TransportType.Buffered,
    *,
    workers: int = 1,
    protocol: ProtocolType = ProtocolType.Binary,
) -> ThriftServerWrapper:
    """
    Create a :class:`ThriftServer`, register *handler* methods, and start serving.

    Analogous to ``thriftpy2.rpc.make_server``.
    """
    server = ThriftServerWrapper(service, handler, transport, workers, protocol)
    return server
