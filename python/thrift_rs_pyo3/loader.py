import os
from typing import Dict, Any, Optional
from .thrift_rs_pyo3 import ThriftParser, PyThriftService


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

    def __getattr__(self, name: str):
        if name in self._structs:
            return self._structs[name]
        if name in self._services:
            return self._services[name]
        raise AttributeError(f"'{self.__class__.__name__}' object has no attribute '{name}'")

    def create_service(self, service_name: str, handlers: Dict[str, Any]):
        """Create a service instance with the given handlers"""
        service_def = self._parser.get_service(service_name)
        if not service_def:
            raise ValueError(f"Service '{service_name}' not found in thrift definitions")
        service_instance = PyThriftService(service_def, handlers)
        self._services[service_name] = service_instance
        setattr(self, service_name, service_instance)
        return service_instance


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
