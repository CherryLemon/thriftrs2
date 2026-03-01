import os
from typing import Dict, Any, Optional, Union
from .thrift_rs_pyo3 import ThriftParser, PyThriftService, ThriftClient, ThriftServer, TransportType


class ThriftModule:
    """Similar to thriftpy2's thrift module representation"""

    def __init__(self, name: str, parser: ThriftParser):
        self.name = name
        self._parser = parser
        self._structs = {}
        self._services = {}
        self._load_definitions()

    def _load_definitions(self):
        """Load struct and service definitions from parser"""
        struct_names = self._parser.list_structs()
        for struct_name in struct_names:
            struct_def = self._parser.get_struct(struct_name)
            if struct_def:
                self._structs[struct_name] = struct_def
                # Make structs accessible as attributes
                setattr(self, struct_name, struct_def)
        service_name = self._parser.list_services()
        for service_name in service_name:
            service_def = self._parser.get_service(service_name)
            if service_def:
                self._services[service_name] = service_def
                # Services are not instantiated yet, so we don't set them as attributes here

    def __getattr__(self, name: str):
        if name in self._structs:
            return self._structs[name]
        if name in self._services:
            return ThriftService(self._parser, self._services[name])
        raise AttributeError(f"'{self.__class__.__name__}' object has no attribute '{name}'")


class ThriftService:
    def __init__(self, parser: ThriftParser, service_def):
        self.parser = parser
        self.service_def = service_def

    def __repr__(self):
        return f"<ThriftService {self.service_def}>"


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

    with open(thrift_file, 'r', encoding='utf-8') as f:
        content = f.read()

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
) -> ThriftClient:
    """
    Create a connected :class:`ThriftClient`.

    Analogous to ``thriftpy2.rpc.make_client``.

    Parameters
    ----------
    service : ThriftModule or PyThriftService
        Either the :class:`ThriftModule` returned by :func:`load` (recommended),
        or a raw ``PyThriftService`` obtained from
        ``thrift_module._parser.get_service("ServiceName")``.
    host : str
        Remote host to connect to.
    port : int
        Remote port to connect to.
    transport : TransportType
        ``TransportType.Framed`` or ``TransportType.Buffered``
        (default ``Buffered``).
    service_name : str, optional
        Only needed when *service* is a :class:`ThriftModule` that defines
        more than one service.
    parser : ThriftParser, optional
        Override the parser used for nested struct resolution.  When *service*
        is a :class:`ThriftModule` the module's own parser is used automatically.

    Returns
    -------
    ThriftClient
        An already-connected client instance.
    """
    client = ThriftClient(service.service_def, host, port, transport)
    client.set_parser(service.parser)
    client.open()
    return client


def make_server(
        service: ThriftService,
        handler: Any,
        host: str = "127.0.0.1",
        port: int = 9090,
        transport: TransportType = TransportType.Buffered,
        *,
        workers: int = 1
) -> ThriftServer:
    """
    Create a :class:`ThriftServer`, register *handler* methods, and start
    serving.

    Analogous to ``thriftpy2.rpc.make_server``.

    The *handler* object is inspected: every public method whose name matches
    a method declared in the service is registered automatically.  You can
    also pass a plain ``dict`` mapping method names to callables.

    Parameters
    ----------
    service : ThriftModule or PyThriftService
        Either the :class:`ThriftModule` returned by :func:`load` (recommended),
        or a raw ``PyThriftService`` obtained from
        ``thrift_module._parser.get_service("ServiceName")``.
    handler : object or dict
        Object whose methods (or dict whose values) implement the service
        methods.
    host : str
        Address to bind to (default ``"127.0.0.1"``).
    port : int
        Port to listen on (default ``9090``).
    transport : TransportType
        ``TransportType.Framed`` or ``TransportType.Buffered``
        (default ``Buffered``).
    service_name : str, optional
        Only needed when *service* is a :class:`ThriftModule` that defines
        more than one service.
    workers : int
        Number of worker threads (default ``1``).
    parser : ThriftParser, optional
        Override the parser used for nested struct resolution.  When *service*
        is a :class:`ThriftModule` the module's own parser is used automatically.

    Returns
    -------
    ThriftServer
        The server is already serving (this call blocks until the server
        is stopped).
    """
    server = ThriftServer(service.service_def, transport, workers)
    server.set_parser(service.parser)

    # Accept either a plain dict or an object with handler methods.
    if isinstance(handler, dict):
        for method_name, func in handler.items():
            server.register_handler(method_name, func)
    else:
        for attr_name in dir(handler):
            if attr_name.startswith("_"):
                continue
            func = getattr(handler, attr_name)
            if callable(func):
                server.register_handler(attr_name, func)

    server.serve_nonblocking(host, port)
    return server
