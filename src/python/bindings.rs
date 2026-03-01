use crate::parser::{ast::*, Parser};
use crate::protocol::{
    BinaryProtocolReader, BinaryProtocolWriter, FieldBegin, ListBegin, MapBegin, SetBegin, TType,
    MESSAGE_TYPE_CALL, MESSAGE_TYPE_EXCEPTION, MESSAGE_TYPE_REPLY,
};
use byteorder::BigEndian;
use pyo3::prelude::*;
use pyo3::types::{PyBytes, PyDict, PyList};
use pyo3::Py;
use std::collections::HashMap;
use std::io::{BufReader, BufWriter, Cursor, Write};
use std::net::TcpStream;
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::{Arc, Mutex};
// ──────────────────────────────────────────────────────────────────────────────
// ThriftParser
// ──────────────────────────────────────────────────────────────────────────────

#[pyclass]
pub struct ThriftParser {
    document: Option<ThriftDocument>,
}

#[pymethods]
impl ThriftParser {
    #[new]
    pub fn new() -> Self {
        Self { document: None }
    }

    pub fn parse(&mut self, content: &str) -> PyResult<()> {
        let mut parser = Parser::new(content).map_err(|e| {
            PyErr::new::<pyo3::exceptions::PyValueError, _>(format!("Parse error: {}", e))
        })?;

        self.document = Some(parser.parse_document().map_err(|e| {
            PyErr::new::<pyo3::exceptions::PyValueError, _>(format!("Parse error: {}", e))
        })?);

        Ok(())
    }

    pub fn list_structs(&self) -> PyResult<Vec<String>> {
        match &self.document {
            Some(doc) => Ok(doc.structs.keys().cloned().collect()),
            None => Err(PyErr::new::<pyo3::exceptions::PyValueError, _>(
                "No document parsed yet",
            )),
        }
    }

    pub fn get_struct(&self, name: &str) -> PyResult<Option<ThriftStruct>> {
        match &self.document {
            Some(doc) => Ok(doc.structs.get(name).map(|s| {
                let fields: Vec<ThriftField> = s
                    .fields
                    .iter()
                    .map(|f| ThriftField {
                        id: f.id,
                        name: f.name.clone(),
                        required: f.required,
                        field_type: f.field_type.clone(),
                    })
                    .collect();
                let field_map: HashMap<i16, usize> = fields
                    .iter()
                    .enumerate()
                    .map(|(idx, f)| (f.id, idx))
                    .collect();
                ThriftStruct {
                    name: s.name.clone(),
                    fields,
                    field_map,
                    struct_map: Arc::new(HashMap::new()),
                }
            })),
            None => Err(PyErr::new::<pyo3::exceptions::PyValueError, _>(
                "No document parsed yet",
            )),
        }
    }

    pub fn list_services(&self) -> PyResult<Vec<String>> {
        match &self.document {
            Some(doc) => Ok(doc.services.keys().cloned().collect()),
            None => Err(PyErr::new::<pyo3::exceptions::PyValueError, _>(
                "No document parsed yet",
            )),
        }
    }

    pub fn get_service(&self, name: &str) -> PyResult<Option<PyThriftService>> {
        match &self.document {
            Some(doc) => Ok(doc.services.get(name).map(|svc| {
                let methods: Vec<PyThriftMethod> = svc
                    .methods
                    .iter()
                    .map(|m| {
                        let args: Vec<ThriftField> = m
                            .arguments
                            .iter()
                            .map(|f| ThriftField {
                                id: f.id,
                                name: f.name.clone(),
                                required: f.required,
                                field_type: f.field_type.clone(),
                            })
                            .collect();
                        let exceptions: Vec<ThriftField> = m
                            .exceptions
                            .iter()
                            .map(|f| ThriftField {
                                id: f.id,
                                name: f.name.clone(),
                                required: f.required,
                                field_type: f.field_type.clone(),
                            })
                            .collect();
                        let arg_field_map: HashMap<i16, usize> = args
                            .iter()
                            .enumerate()
                            .map(|(i, f)| (f.id, i))
                            .collect();
                        PyThriftMethod {
                            name: m.name.clone(),
                            return_type: m.return_type.clone(),
                            arguments: args,
                            exceptions,
                            arg_field_map,
                        }
                    })
                    .collect();
                PyThriftService {
                    name: svc.name.clone(),
                    methods,
                }
            })),
            None => Err(PyErr::new::<pyo3::exceptions::PyValueError, _>(
                "No document parsed yet",
            )),
        }
    }
}

impl ThriftParser {
    /// Return a snapshot of the whole parsed document's struct map so the server
    /// can resolve nested struct types by name during serialisation/deserialisation.
    pub(crate) fn struct_map(&self) -> Arc<HashMap<String, ThriftStruct>> {
        let map: HashMap<String, ThriftStruct> = match &self.document {
            Some(doc) => doc
                .structs
                .iter()
                .map(|(k, s)| {
                    let fields: Vec<ThriftField> = s
                        .fields
                        .iter()
                        .map(|f| ThriftField {
                            id: f.id,
                            name: f.name.clone(),
                            required: f.required,
                            field_type: f.field_type.clone(),
                        })
                        .collect();
                    let field_map: HashMap<i16, usize> = fields
                        .iter()
                        .enumerate()
                        .map(|(idx, f)| (f.id, idx))
                        .collect();
                    (
                        k.clone(),
                        ThriftStruct {
                            name: s.name.clone(),
                            fields,
                            field_map,
                            struct_map: Arc::new(HashMap::new()), // filled in below
                        },
                    )
                })
                .collect(),
            None => HashMap::new(),
        };
        // Now wrap in Arc and back-patch each ThriftStruct with the shared map.
        let arc = Arc::new(map);
        // Re-build with back-reference so ThriftStruct::new_instance can use it.
        // We reconstruct a new map where every ThriftStruct has struct_map set.
        let patched: HashMap<String, ThriftStruct> = arc
            .iter()
            .map(|(k, s)| {
                (
                    k.clone(),
                    ThriftStruct {
                        name: s.name.clone(),
                        fields: s.fields.clone(),
                        field_map: s.field_map.clone(),
                        struct_map: Arc::clone(&arc),
                    },
                )
            })
            .collect();
        Arc::new(patched)
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// ThriftStruct / ThriftField
// ──────────────────────────────────────────────────────────────────────────────

#[pyclass(skip_from_py_object)]
#[derive(Clone)]
pub struct ThriftStruct {
    #[pyo3(get)]
    pub name: String,
    #[pyo3(get)]
    pub fields: Vec<ThriftField>,
    /// Cached map from field id -> index in `fields`, built once at parse time.
    field_map: HashMap<i16, usize>,
    /// Shared map of all structs in the parsed document — used by new_instance
    /// to give the created ThriftStructInstance schema-aware setattr coercion.
    pub(crate) struct_map: Arc<HashMap<String, ThriftStruct>>,
}

#[pymethods]
impl ThriftStruct {
    /// construct an empty ThriftStructInstance with all fields set to None.
    pub fn new_instance(&self, _py: Python<'_>) -> ThriftStructInstance {
        let field_names = self.fields.iter().map(|f| f.name.clone()).collect();
        let schema: HashMap<String, ThriftField> = self
            .fields
            .iter()
            .map(|f| (f.name.clone(), f.clone()))
            .collect();
        ThriftStructInstance::empty(
            self.name.clone(),
            field_names,
            schema,
            Arc::clone(&self.struct_map),
        )
    }
    pub fn new_instance_from_dict(
        &self,
        py: Python<'_>,
        items: &Bound<'_, PyDict>,
    ) -> ThriftStructInstance {
        let field_names: Vec<String> = self.fields.iter().map(|f| f.name.clone()).collect();
        let schema: HashMap<String, ThriftField> = self
            .fields
            .iter()
            .map(|f| (f.name.clone(), f.clone()))
            .collect();
        let mut instance = ThriftStructInstance::empty(
            self.name.clone(),
            field_names,
            schema.clone(),
            Arc::clone(&self.struct_map),
        );
        for (k, v) in items.iter() {
            if let Ok(name) = k.extract::<String>() {
                if instance.field_names.contains(&name) {
                    // Populate cache (Python-visible)
                    instance.cache.insert(name.clone(), v.clone().unbind());
                    // Populate inner (GIL-free serialisation path), schema-aware
                    let tv_result = if let Some(field) = schema.get(&name) {
                        py_any_to_thrift_value_with_type(
                            &v,
                            &field.field_type.clone(),
                            &self.struct_map,
                        )
                    } else {
                        py_any_to_thrift_value(&v)
                    };
                    if let Ok(tv) = tv_result {
                        instance.inner.values.insert(name, tv);
                    }
                    let _ = py;
                }
            }
        }
        instance
    }
    #[pyo3(signature = (**kwargs))]
    pub fn __call__(
        &self,
        py: Python<'_>,
        kwargs: Option<&Bound<'_, PyDict>>,
    ) -> ThriftStructInstance {
        let field_names = self.fields.iter().map(|f| f.name.clone()).collect();
        let schema: HashMap<String, ThriftField> = self
            .fields
            .iter()
            .map(|f| (f.name.clone(), f.clone()))
            .collect();
        if let Some(kw) = kwargs {
            self.new_instance_from_dict(py, kw)
        } else {
            ThriftStructInstance::empty(
                self.name.clone(),
                field_names,
                schema,
                Arc::clone(&self.struct_map),
            )
        }
    }

    /// Serialize a struct from either a `ThriftStructInstance` or a plain `dict`.
    pub fn serialize(&self, py: Python<'_>, data: &Bound<'_, PyAny>) -> PyResult<Vec<u8>> {
        let mut buffer = Vec::with_capacity(128 + self.fields.len() * 16);
        let mut writer = BinaryProtocolWriter::new(&mut buffer);
        serialize_struct_any(&mut writer, &self.fields, data, &self.struct_map, py)?;
        writer.write_field_stop().map_err(|e| {
            PyErr::new::<pyo3::exceptions::PyIOError, _>(format!("Write error: {}", e))
        })?;
        Ok(buffer)
    }

    /// Deserialize bytes into a `ThriftStructInstance`.
    pub fn deserialize<'py>(
        &self,
        py: Python<'py>,
        data: &[u8],
    ) -> PyResult<Bound<'py, ThriftStructInstance>> {
        let mut cursor = Cursor::new(data);
        let mut reader = BinaryProtocolReader::new(&mut cursor);
        let schema: HashMap<String, ThriftField> = self
            .fields
            .iter()
            .map(|f| (f.name.clone(), f.clone()))
            .collect();
        deserialize_struct_fields_as_instance(
            &mut reader,
            &self.fields,
            &self.field_map,
            &self.struct_map,
            py,
            &self.name,
            schema,
            Arc::clone(&self.struct_map),
        )
    }

    pub fn __repr__(&self) -> String {
        format!(
            "ThriftStruct(name={:?}, fields={:?})",
            self.name, self.fields
        )
    }
}

#[pyclass(skip_from_py_object)]
#[derive(Debug, Clone)]
pub struct ThriftField {
    #[pyo3(get)]
    pub id: i16,
    #[pyo3(get)]
    pub name: String,
    #[pyo3(get)]
    pub required: bool,
    pub(crate) field_type: ThriftType,
}

#[pymethods]
impl ThriftField {
    pub fn __repr__(&self) -> String {
        format!(
            "ThriftField(id={}, name={:?}, required={}, field_type={:?})",
            self.id, self.name, self.required, self.field_type
        )
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// RustStructValue  –  GIL-free pure-Rust representation of a struct instance.
// ──────────────────────────────────────────────────────────────────────────────

/// Pure-Rust struct value produced during deserialisation without holding the
/// GIL.  Absent keys mean the field was not present on the wire (None).
#[derive(Clone)]
pub struct RustStructValue {
    pub struct_name: String,
    pub field_names: Vec<String>,
    pub values: HashMap<String, ThriftValue>,
}

impl RustStructValue {
    pub fn empty(struct_name: String, field_names: Vec<String>) -> Self {
        Self {
            struct_name,
            field_names,
            values: HashMap::new(),
        }
    }
    pub fn set_field(&mut self, name: &str, value: ThriftValue) {
        self.values.insert(name.to_string(), value);
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// ThriftStructInstance  –  Python-visible wrapper around RustStructValue.
//
// `cache` is authoritative for fields touched by Python.  Fields that have
// never been touched by Python live only in `inner`.
// ──────────────────────────────────────────────────────────────────────────────

/// A live instance of a Thrift struct.
///
/// Fields are accessible as Python attributes (``instance.field_name``).
/// Unknown attribute names raise ``AttributeError``.
#[pyclass]
pub struct ThriftStructInstance {
    #[pyo3(get)]
    pub struct_name: String,
    pub field_names: Vec<String>,
    /// Pure-Rust inner store.  Populated at deserialisation time without GIL.
    pub inner: RustStructValue,
    /// Lazy Python-object cache.  A field enters the cache on first
    /// `__getattr__` or `__setattr__`.  Serialisation checks cache first,
    /// then falls back to `inner` (GIL-free path).
    pub cache: HashMap<String, Py<PyAny>>,
    /// Schema: field name → ThriftField, used by __setattr__ for type-aware coercion.
    /// Empty for schemaless instances (created via #[new] without a ThriftStruct).
    pub schema: HashMap<String, ThriftField>,
    /// Shared struct map for resolving nested struct types in schema-aware coercion.
    /// Empty (Arc pointing to empty HashMap) for schemaless instances.
    pub struct_map: Arc<HashMap<String, ThriftStruct>>,
}

impl Clone for ThriftStructInstance {
    fn clone(&self) -> Self {
        Python::attach(|py| Self {
            struct_name: self.struct_name.clone(),
            field_names: self.field_names.clone(),
            inner: self.inner.clone(),
            cache: self
                .cache
                .iter()
                .map(|(k, v)| (k.clone(), v.clone_ref(py)))
                .collect(),
            schema: self.schema.clone(),
            struct_map: Arc::clone(&self.struct_map),
        })
    }
}

impl ThriftStructInstance {
    /// Construct from a pre-built `RustStructValue` with an empty cache,
    /// and optionally a schema + struct_map for type-aware __setattr__ coercion.
    pub fn from_rust(
        inner: RustStructValue,
        schema: HashMap<String, ThriftField>,
        struct_map: Arc<HashMap<String, ThriftStruct>>,
    ) -> Self {
        let struct_name = inner.struct_name.clone();
        let field_names = inner.field_names.clone();
        Self {
            struct_name,
            field_names,
            inner,
            cache: HashMap::new(),
            schema,
            struct_map,
        }
    }

    /// Construct an empty instance (no inner values, empty cache).
    pub fn empty(
        struct_name: String,
        field_names: Vec<String>,
        schema: HashMap<String, ThriftField>,
        struct_map: Arc<HashMap<String, ThriftStruct>>,
    ) -> Self {
        let inner = RustStructValue::empty(struct_name.clone(), field_names.clone());
        Self {
            struct_name,
            field_names,
            inner,
            cache: HashMap::new(),
            schema,
            struct_map,
        }
    }

    /// Set a field value from a native `ThriftValue` (used during deser).
    /// Does NOT touch the cache.
    pub fn set_field(&mut self, name: &str, value: ThriftValue) {
        self.inner.set_field(name, value);
    }

    /// Get a field as a Python object, populating the cache on first access.
    pub fn get_field<'py>(&mut self, py: Python<'py>, name: &str) -> Option<Bound<'py, PyAny>> {
        if !self.field_names.contains(&name.to_string()) {
            return None;
        }
        if let Some(v) = self.cache.get(name) {
            return Some(v.bind(py).clone());
        }
        let py_val = match self.inner.values.get(name) {
            Some(tv) => thrift_value_to_py(tv, py).unwrap_or_else(|_| py.None()),
            None => py.None(),
        };
        self.cache.insert(name.to_string(), py_val);
        Some(self.cache[name].bind(py).clone())
    }
}

#[pymethods]
impl ThriftStructInstance {
    #[new]
    #[pyo3(signature = (struct_name, **kwargs))]
    pub fn new(py: Python<'_>, struct_name: String, kwargs: Option<&Bound<'_, PyDict>>) -> Self {
        let mut field_names = Vec::new();
        let mut cache = HashMap::new();
        if let Some(kw) = kwargs {
            for (k, v) in kw.iter() {
                if let Ok(name) = k.extract::<String>() {
                    field_names.push(name.clone());
                    cache.insert(name.clone(), v.unbind());
                }
            }
        }
        let _ = py;
        let inner = RustStructValue {
            struct_name: struct_name.clone(),
            field_names: field_names.clone(),
            values: HashMap::new(),
        };
        Self {
            struct_name,
            field_names,
            inner,
            cache,
            schema: HashMap::new(),
            struct_map: Arc::new(HashMap::new()),
        }
    }

    pub fn __getattr__(&mut self, py: Python<'_>, name: &str) -> PyResult<Py<PyAny>> {
        if self.field_names.contains(&name.to_string()) {
            Ok(self
                .get_field(py, name)
                .map(|v| v.unbind())
                .unwrap_or_else(|| py.None()))
        } else {
            Err(PyErr::new::<pyo3::exceptions::PyAttributeError, _>(
                format!("'{}' object has no attribute '{}'", self.struct_name, name),
            ))
        }
    }

    pub fn __setattr__(&mut self, py: Python<'_>, name: &str, value: Py<PyAny>) -> PyResult<()> {
        if self.field_names.contains(&name.to_string()) {
            // cache is authoritative
            self.cache.insert(name.to_string(), value.clone_ref(py));
            // keep inner in sync for GIL-free serialization; soft failure is OK
            let bound = value.bind(py);
            let tv_result = if let Some(field) = self.schema.get(name) {
                // Schema-aware path: coerce to the exact ThriftValue variant.
                py_any_to_thrift_value_with_type(bound, &field.field_type.clone(), &self.struct_map)
            } else {
                // Schemaless fallback.
                py_any_to_thrift_value(bound)
            };
            if let Ok(tv) = tv_result {
                self.inner.values.insert(name.to_string(), tv);
            } else {
                self.inner.values.remove(name);
            }
            Ok(())
        } else {
            Err(PyErr::new::<pyo3::exceptions::PyAttributeError, _>(
                format!("'{}' object has no attribute '{}'", self.struct_name, name),
            ))
        }
    }

    pub fn __repr__(&mut self, py: Python<'_>) -> String {
        let names = self.field_names.clone();
        let fields: Vec<String> = names
            .iter()
            .map(|name| {
                let val = self
                    .get_field(py, name)
                    .map(|v| format!("{:?}", v))
                    .unwrap_or_else(|| "None".to_string());
                format!("{}={}", name, val)
            })
            .collect();
        format!("{}({})", self.struct_name, fields.join(", "))
    }

    pub fn to_dict<'py>(&mut self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let d = PyDict::new(py);
        let names = self.field_names.clone();
        for name in &names {
            let v = self
                .get_field(py, name)
                .unwrap_or_else(|| py.None().into_bound(py));
            d.set_item(name, v)?;
        }
        Ok(d)
    }

    pub fn field_names(&self) -> Vec<String> {
        self.field_names.clone()
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// PyThriftService / PyThriftMethod  (exposed to Python)
// ──────────────────────────────────────────────────────────────────────────────

#[pyclass]
#[derive(Clone)]
pub struct PyThriftService {
    #[pyo3(get)]
    pub name: String,
    #[pyo3(get)]
    pub methods: Vec<PyThriftMethod>,
}

#[pymethods]
impl PyThriftService {
    pub fn get_method(&self, name: &str) -> Option<PyThriftMethod> {
        self.methods.iter().find(|m| m.name == name).cloned()
    }

    pub fn __repr__(&self) -> String {
        let method_names: Vec<&str> = self.methods.iter().map(|m| m.name.as_str()).collect();
        format!(
            "ThriftService(name={:?}, methods={:?})",
            self.name, method_names
        )
    }
}

#[pyclass]
#[derive(Clone)]
pub struct PyThriftMethod {
    #[pyo3(get)]
    pub name: String,
    #[pyo3(get)]
    pub arguments: Vec<ThriftField>,
    #[pyo3(get)]
    pub exceptions: Vec<ThriftField>,
    pub(crate) return_type: ThriftType,
    /// Pre-computed field-id → index map for argument deserialisation.
    /// Built once at parse time so the hot `handle_connection` path never allocates it.
    pub(crate) arg_field_map: HashMap<i16, usize>,
}

#[pymethods]
impl PyThriftMethod {
    pub fn __repr__(&self) -> String {
        format!(
            "ThriftMethod(name={:?}, return_type={:?})",
            self.name, self.return_type
        )
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// BinaryProtocol
// ──────────────────────────────────────────────────────────────────────────────

#[pyclass]
pub struct BinaryProtocol;

#[pymethods]
impl BinaryProtocol {
    #[new]
    pub fn new() -> Self {
        Self
    }

    #[staticmethod]
    pub fn serialize_struct(
        py: Python<'_>,
        struct_def: &ThriftStruct,
        data: &Bound<'_, PyAny>,
    ) -> PyResult<Vec<u8>> {
        struct_def.serialize(py, data)
    }

    #[staticmethod]
    pub fn deserialize_struct<'py>(
        py: Python<'py>,
        struct_def: &ThriftStruct,
        data: &[u8],
    ) -> PyResult<Bound<'py, PyAny>> {
        struct_def.deserialize(py, data).map(|d| d.into_any())
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// TransportType
// ──────────────────────────────────────────────────────────────────────────────

/// Transport layer to use when reading/writing Thrift messages over TCP.
///
/// * `Framed`   – each message is prefixed by a 4-byte big-endian frame length
///                (TFramedTransport in the official Thrift SDKs).
/// * `Buffered` – messages are written directly to the socket with no framing
///                envelope (TBufferedTransport / TSocket in the official SDKs).
#[pyclass(eq, eq_int)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransportType {
    Framed = 0,
    Buffered = 1,
}

#[pymethods]
impl TransportType {
    #[new]
    pub fn new() -> Self {
        Self::Framed
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// ThriftServer
// ──────────────────────────────────────────────────────────────────────────────

#[pyclass]
pub struct ThriftServer {
    service: PyThriftService,
    handlers: HashMap<String, Py<PyAny>>,
    /// Snapshot of all structs from the parsed document, needed to resolve
    /// named struct types during handler argument de/serialisation.
    /// Stored as Arc to allow clone-free sharing across spawned threads.
    struct_map: Arc<HashMap<String, ThriftStruct>>,
    /// Transport type to use (framed or buffered).
    transport: TransportType,
    /// Number of worker threads in the connection-handler pool.
    /// 0 means use the number of logical CPUs.
    workers: usize,
}

#[pymethods]
impl ThriftServer {
    #[new]
    #[pyo3(signature = (service, transport = TransportType::Framed))]
    pub fn new(service: PyThriftService, transport: TransportType) -> Self {
        Self {
            service,
            handlers: HashMap::new(),
            struct_map: Arc::new(HashMap::new()),
            transport,
            workers: 1,
        }
    }

    /// Get the current transport type.
    #[getter]
    pub fn transport(&self) -> TransportType {
        self.transport
    }

    /// Set the transport type (framed or buffered).
    #[setter]
    pub fn set_transport(&mut self, transport: TransportType) {
        self.transport = transport;
    }

    /// Get the number of worker threads (0 = auto / number of logical CPUs).
    #[getter]
    pub fn workers(&self) -> usize {
        self.workers
    }

    /// Set the number of worker threads in the connection-handler pool.
    /// Set to 0 to use the number of logical CPUs (default).
    #[setter]
    pub fn set_workers(&mut self, workers: usize) {
        self.workers = workers;
    }

    /// Attach the parser so that nested struct types can be resolved.
    pub fn set_parser(&mut self, parser: &ThriftParser) {
        self.struct_map = parser.struct_map();
    }

    /// Register a Python callable as the handler for `method_name`.
    pub fn register_handler(&mut self, method_name: &str, handler: Py<PyAny>) {
        self.handlers.insert(method_name.to_string(), handler);
    }

    /// Start a **blocking** TCP server on `host:port`.
    /// Connections are handled by a fixed-size worker-thread pool.
    /// Releases the GIL while waiting for connections.
    pub fn serve(&self, py: Python<'_>, host: &str, port: u16) -> PyResult<()> {
        use std::net::TcpListener;

        let addr = format!("{}:{}", host, port);

        // Clone handlers while we still hold the GIL.
        let service = self.service.clone();
        let handlers: HashMap<String, Py<PyAny>> = self
            .handlers
            .iter()
            .map(|(k, v)| (k.clone(), v.clone_ref(py)))
            .collect();
        let struct_map = Arc::clone(&self.struct_map);
        let transport = self.transport;
        let n_workers = if self.workers == 0 {
            num_cpus::get().max(2)
        } else {
            self.workers
        };

        let listener = TcpListener::bind(&addr).map_err(|e| {
            PyErr::new::<pyo3::exceptions::PyOSError, _>(format!("bind error: {}", e))
        })?;

        println!(
            "ThriftServer ({:?}, {} workers) listening on {}",
            transport, n_workers, addr
        );

        let service = Arc::new(service);
        let handlers = Arc::new(handlers);

        // Release the GIL while blocking in accept loop.
        py.detach(|| {
            run_server_pool(listener, service, handlers, struct_map, transport, n_workers);
        });

        Ok(())
    }

    /// Start a **non-blocking** server in a background thread.
    /// Returns immediately. The server keeps running until the process exits.
    pub fn serve_nonblocking(&self, py: Python<'_>, host: &str, port: u16) -> PyResult<()> {
        use std::net::TcpListener;

        let addr = format!("{}:{}", host, port);
        let service = self.service.clone();
        let handlers: HashMap<String, Py<PyAny>> = self
            .handlers
            .iter()
            .map(|(k, v)| (k.clone(), v.clone_ref(py)))
            .collect();
        let struct_map = Arc::clone(&self.struct_map);
        let transport = self.transport;
        let n_workers = if self.workers == 0 {
            num_cpus::get().max(2)
        } else {
            self.workers
        };

        let listener = TcpListener::bind(&addr).map_err(|e| {
            PyErr::new::<pyo3::exceptions::PyOSError, _>(format!("bind error: {}", e))
        })?;

        println!(
            "ThriftServer (non-blocking, {:?}, {} workers) listening on {}",
            transport, n_workers, addr
        );

        let service = Arc::new(service);
        let handlers = Arc::new(handlers);

        std::thread::spawn(move || {
            run_server_pool(listener, service, handlers, struct_map, transport, n_workers);
        });

        Ok(())
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// Thread-pool server helpers
// ──────────────────────────────────────────────────────────────────────────────

/// Spin up a fixed-size pool of worker threads and an accept loop.
/// Accepted connections are dispatched to workers via a channel.
fn run_server_pool(
    listener: std::net::TcpListener,
    service: Arc<PyThriftService>,
    handlers: Arc<HashMap<String, Py<PyAny>>>,
    struct_map: Arc<HashMap<String, ThriftStruct>>,
    transport: TransportType,
    n_workers: usize,
) {
    use std::sync::mpsc;

    let (tx, rx) = mpsc::channel::<TcpStream>();
    let rx = Arc::new(Mutex::new(rx));

    for _ in 0..n_workers {
        let rx = Arc::clone(&rx);
        let service = Arc::clone(&service);
        let handlers = Arc::clone(&handlers);
        let struct_map = Arc::clone(&struct_map);
        std::thread::spawn(move || loop {
            let stream = match rx.lock().unwrap().recv() {
                Ok(s) => s,
                Err(_) => break, // channel closed
            };
            if let Err(e) =
                handle_connection(stream, &service, &handlers, &struct_map, transport)
            {
                if e.kind() != std::io::ErrorKind::UnexpectedEof
                    && e.kind() != std::io::ErrorKind::ConnectionReset
                    && e.kind() != std::io::ErrorKind::BrokenPipe
                {
                    eprintln!("Connection error: {}", e);
                }
            }
        });
    }

    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                let _ = tx.send(stream);
            }
            Err(e) => eprintln!("Accept error: {}", e),
        }
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// Connection handler
// ──────────────────────────────────────────────────────────────────────────────

fn handle_connection(
    stream: std::net::TcpStream,
    service: &PyThriftService,
    handlers: &HashMap<String, Py<PyAny>>,
    struct_map: &Arc<HashMap<String, ThriftStruct>>,
    transport: TransportType,
) -> std::io::Result<()> {
    use byteorder::ReadBytesExt;
    use std::io::Read;

    // TCP_NODELAY eliminates Nagle-algorithm batching latency.
    let _ = stream.set_nodelay(true);

    // Clone for independent read/write handles.
    let write_stream = stream.try_clone()?;

    // Always wrap read side in BufReader – efficient for both framed and buffered transports.
    let mut buf_reader = BufReader::with_capacity(65536, stream);
    // BufWriter coalesces header + payload into a single flush() syscall.
    let mut buf_writer = BufWriter::with_capacity(65536, write_stream);

    loop {
        // ── Read the next message payload ─────────────────────────────────────
        let frame: Vec<u8> = match transport {
            TransportType::Framed => {
                let frame_len = match buf_reader.read_i32::<BigEndian>() {
                    Ok(n) if n > 0 => n as usize,
                    Ok(_) => return Ok(()),
                    Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(()),
                    Err(e) => return Err(e),
                };
                let mut buf = vec![0u8; frame_len];
                buf_reader.read_exact(&mut buf)?;
                buf
            }
            TransportType::Buffered => {
                let mut hdr = [0u8; 4];
                match buf_reader.read_exact(&mut hdr) {
                    Ok(()) => {}
                    Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(()),
                    Err(e) => return Err(e),
                }
                let mut name_len_bytes = [0u8; 4];
                buf_reader.read_exact(&mut name_len_bytes)?;
                let name_len = i32::from_be_bytes(name_len_bytes) as usize;
                let mut name_buf = vec![0u8; name_len];
                buf_reader.read_exact(&mut name_buf)?;
                let mut seq_id_bytes = [0u8; 4];
                buf_reader.read_exact(&mut seq_id_bytes)?;

                let mut body: Vec<u8> = Vec::with_capacity(256);
                read_buffered_struct_body(&mut buf_reader, &mut body)?;

                let mut frame = Vec::with_capacity(4 + 4 + name_len + 4 + body.len());
                frame.extend_from_slice(&hdr);
                frame.extend_from_slice(&name_len_bytes);
                frame.extend_from_slice(&name_buf);
                frame.extend_from_slice(&seq_id_bytes);
                frame.extend_from_slice(&body);
                frame
            }
        };

        // ── Parse message header ──────────────────────────────────────────────
        let mut cursor = Cursor::new(&frame[..]);
        let mut reader = BinaryProtocolReader::new(&mut cursor);
        let msg_begin = reader
            .read_message_begin()
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?;

        // ── Find method definition ────────────────────────────────────────────
        let method_def = service.methods.iter().find(|m| m.name == msg_begin.name);

        let response_payload = match method_def {
            None => build_exception_reply(
                &msg_begin.name,
                msg_begin.seq_id,
                1,
                &format!("Unknown method: {}", msg_begin.name),
            ),
            Some(method) => {
                // ── Deserialise arguments (GIL-free) ─────────────────────────
                // Use pre-computed arg_field_map — no per-request HashMap allocation.
                let args_rust = deserialize_rust_struct(
                    &mut reader,
                    &method.arguments,
                    &method.arg_field_map,
                    struct_map,
                )
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?;

                let args_rust = RustStructValue {
                    struct_name: format!("{}_args", msg_begin.name),
                    field_names: args_rust.field_names,
                    values: args_rust.values,
                };

                // ── Find handler (before entering GIL) ───────────────────────
                let handler = match handlers.get(&msg_begin.name) {
                    Some(h) => h,
                    None => {
                        let payload = build_exception_reply(
                            &msg_begin.name,
                            msg_begin.seq_id,
                            1,
                            &format!("No handler registered for: {}", msg_begin.name),
                        );
                        send_response_buffered(&mut buf_writer, &payload, transport)?;
                        continue;
                    }
                };

                // ── Single GIL block: bind args, call handler, serialise reply ─
                let result: Result<Vec<u8>, String> = Python::attach(|py| {
                    let schema: HashMap<String, ThriftField> = method
                        .arguments
                        .iter()
                        .map(|f| (f.name.clone(), f.clone()))
                        .collect();
                    let args_instance =
                        ThriftStructInstance::from_rust(args_rust, schema, Arc::clone(struct_map));
                    let py_args = Bound::new(py, args_instance)?;

                    let kwargs = pyo3::types::PyDict::new(py);
                    let mut inst_borrow = py_args.borrow_mut();
                    let field_names: Vec<String> = inst_borrow.field_names.clone();
                    for name in &field_names {
                        let val = inst_borrow
                            .get_field(py, name)
                            .map(|v| v.unbind())
                            .unwrap_or_else(|| py.None());
                        kwargs.set_item(name, val)?;
                    }
                    drop(inst_borrow);

                    let result = handler.call(py, (), Some(&kwargs))?;
                    let reply_body =
                        build_reply_body(py, &method.return_type, result.bind(py), struct_map)?;
                    Ok(build_reply_frame(
                        &msg_begin.name,
                        msg_begin.seq_id,
                        &reply_body,
                    ))
                })
                .map_err(|e: PyErr| e.to_string());

                match result {
                    Ok(frame) => frame,
                    Err(err_msg) => {
                        build_exception_reply(&msg_begin.name, msg_begin.seq_id, 6, &err_msg)
                    }
                }
            }
        };

        send_response_buffered(&mut buf_writer, &response_payload, transport)?;
    }
}

/// Send a response via a `BufWriter` — header and payload are coalesced into
/// a single `flush()` call instead of multiple `write_all` syscalls.
fn send_response_buffered<W: Write>(
    writer: &mut W,
    payload: &[u8],
    transport: TransportType,
) -> std::io::Result<()> {
    use byteorder::WriteBytesExt;
    match transport {
        TransportType::Framed => {
            writer.write_i32::<BigEndian>(payload.len() as i32)?;
            writer.write_all(payload)?;
        }
        TransportType::Buffered => {
            writer.write_all(payload)?;
        }
    }
    writer.flush()
}


/// Read a Thrift struct body (including nested structs) from a buffered reader
/// until the outermost STOP byte (0x00) is consumed, appending all bytes
/// (including the final STOP) to `out`.
///
/// This is used by the buffered-transport path to reassemble a complete
/// on-wire message before handing it to the normal cursor-based parser.
fn read_buffered_struct_body<R: std::io::Read>(
    reader: &mut R,
    out: &mut Vec<u8>,
) -> std::io::Result<()> {
    use byteorder::ReadBytesExt;

    loop {
        // Read the field type byte.
        let field_type_byte = reader.read_u8()?;
        out.push(field_type_byte);

        if field_type_byte == 0x00 {
            // STOP field — end of this struct.
            return Ok(());
        }

        // Read the 2-byte field id.
        let id_hi = reader.read_u8()?;
        let id_lo = reader.read_u8()?;
        out.push(id_hi);
        out.push(id_lo);

        // Read the field value according to its type.
        read_buffered_value(reader, field_type_byte, out)?;
    }
}

/// Read a single Thrift value of the given wire type into `out`.
fn read_buffered_value<R: std::io::Read>(
    reader: &mut R,
    field_type_byte: u8,
    out: &mut Vec<u8>,
) -> std::io::Result<()> {
    use byteorder::ReadBytesExt;

    match field_type_byte {
        // BOOL, BYTE
        0x02 | 0x03 => {
            let b = reader.read_u8()?;
            out.push(b);
        }
        // I16
        0x06 => {
            let mut buf = [0u8; 2];
            reader.read_exact(&mut buf)?;
            out.extend_from_slice(&buf);
        }
        // I32
        0x08 => {
            let mut buf = [0u8; 4];
            reader.read_exact(&mut buf)?;
            out.extend_from_slice(&buf);
        }
        // I64, DOUBLE
        0x0a | 0x04 => {
            let mut buf = [0u8; 8];
            reader.read_exact(&mut buf)?;
            out.extend_from_slice(&buf);
        }
        // STRING / BINARY  (4-byte length prefix + data)
        0x0b => {
            let mut len_bytes = [0u8; 4];
            reader.read_exact(&mut len_bytes)?;
            out.extend_from_slice(&len_bytes);
            let len = u32::from_be_bytes(len_bytes) as usize;
            let start = out.len();
            out.resize(start + len, 0);
            reader.read_exact(&mut out[start..])?;
        }
        // STRUCT (recursive)
        0x0c => {
            read_buffered_struct_body(reader, out)?;
        }
        // MAP
        0x0d => {
            let mut header = [0u8; 6]; // key_type(1) + val_type(1) + size(4)
            reader.read_exact(&mut header)?;
            out.extend_from_slice(&header);
            let key_type = header[0];
            let val_type = header[1];
            let size = i32::from_be_bytes([header[2], header[3], header[4], header[5]]);
            for _ in 0..size {
                read_buffered_value(reader, key_type, out)?;
                read_buffered_value(reader, val_type, out)?;
            }
        }
        // LIST, SET
        0x0f | 0x0e => {
            let mut header = [0u8; 5]; // elem_type(1) + size(4)
            reader.read_exact(&mut header)?;
            out.extend_from_slice(&header);
            let elem_type = header[0];
            let size = i32::from_be_bytes([header[1], header[2], header[3], header[4]]);
            for _ in 0..size {
                read_buffered_value(reader, elem_type, out)?;
            }
        }
        _ => {
            // Unknown type; we can't safely advance — return an error.
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!(
                    "Unknown field type byte 0x{:02x} in buffered stream",
                    field_type_byte
                ),
            ));
        }
    }
    Ok(())
}

/// Serialise the return value of a method call into a reply struct body.
/// The result field has id=0; success goes in field 0.
fn build_reply_body(
    py: Python<'_>,
    return_type: &ThriftType,
    value: &Bound<'_, PyAny>,
    struct_map: &HashMap<String, ThriftStruct>,
) -> PyResult<Vec<u8>> {
    let mut buf = Vec::with_capacity(64);
    let mut writer = BinaryProtocolWriter::new(&mut buf);

    match return_type {
        ThriftType::Struct(name) if name == "void" => {
            // void — just write STOP
        }
        _ => {
            // write field 0 = success result
            let field_begin = FieldBegin {
                name: None,
                field_type: thrift_type_to_ttype(return_type),
                id: 0,
            };
            writer
                .write_field_begin(&field_begin)
                .map_err(|e| PyErr::new::<pyo3::exceptions::PyIOError, _>(e.to_string()))?;
            write_value_with_structs(&mut writer, return_type, value, struct_map)?;
        }
    }

    writer
        .write_field_stop()
        .map_err(|e| PyErr::new::<pyo3::exceptions::PyIOError, _>(e.to_string()))?;
    drop(writer);
    let _ = py;
    Ok(buf)
}

/// Wrap a reply body into a full Thrift Binary Protocol message frame (no outer i32 length).
fn build_reply_frame(method_name: &str, seq_id: i32, body: &[u8]) -> Vec<u8> {
    let mut buf = Vec::with_capacity(32 + body.len());
    {
        let mut writer = BinaryProtocolWriter::new(&mut buf);
        writer
            .write_message_begin(method_name, MESSAGE_TYPE_REPLY, seq_id)
            .unwrap();
    }
    buf.extend_from_slice(body);
    buf
}

/// Build an application-exception reply frame.
fn build_exception_reply(method_name: &str, seq_id: i32, ex_type: i32, message: &str) -> Vec<u8> {
    let mut buf = Vec::with_capacity(64);
    {
        let mut writer = BinaryProtocolWriter::new(&mut buf);
        writer
            .write_message_begin(method_name, MESSAGE_TYPE_EXCEPTION, seq_id)
            .unwrap();
        // TApplicationException struct: field 1 = message (string), field 2 = type (i32)
        let msg_field = FieldBegin {
            name: None,
            field_type: TType::String,
            id: 1,
        };
        writer.write_field_begin(&msg_field).unwrap();
        writer.write_string(message).unwrap();
        let type_field = FieldBegin {
            name: None,
            field_type: TType::I32,
            id: 2,
        };
        writer.write_field_begin(&type_field).unwrap();
        writer.write_i32(ex_type).unwrap();
        writer.write_field_stop().unwrap();
    }
    buf
}

// ──────────────────────────────────────────────────────────────────────────────
// Helpers shared by ThriftStruct and server handler
// ──────────────────────────────────────────────────────────────────────────────

/// Serialise struct fields from either a `ThriftStructInstance` or a plain `PyDict`.
pub(crate) fn serialize_struct_any<W: std::io::Write>(
    writer: &mut BinaryProtocolWriter<W>,
    fields: &[ThriftField],
    data: &Bound<'_, PyAny>,
    struct_map: &HashMap<String, ThriftStruct>,
    py: Python<'_>,
) -> PyResult<()> {
    if let Ok(instance) = data.cast::<ThriftStructInstance>() {
        let instance = instance.borrow();
        for field in fields {
            // Fast path: field is in the Python cache — use the existing GIL path.
            if let Some(py_val) = instance.cache.get(&field.name) {
                let bound = py_val.bind(py);
                if !bound.is_none() {
                    let field_begin = FieldBegin {
                        name: None,
                        field_type: thrift_type_to_ttype(&field.field_type),
                        id: field.id,
                    };
                    writer.write_field_begin(&field_begin).map_err(|e| {
                        PyErr::new::<pyo3::exceptions::PyIOError, _>(format!("Write error: {}", e))
                    })?;
                    write_value_with_structs(writer, &field.field_type, bound, struct_map)?;
                }
            } else if let Some(tv) = instance.inner.values.get(&field.name) {
                // GIL-free path: field is in inner but was never touched by Python.
                let field_begin = FieldBegin {
                    name: None,
                    field_type: thrift_type_to_ttype(&field.field_type),
                    id: field.id,
                };
                writer.write_field_begin(&field_begin).map_err(|e| {
                    PyErr::new::<pyo3::exceptions::PyIOError, _>(format!("Write error: {}", e))
                })?;
                write_thrift_value(writer, tv, struct_map).map_err(|e| {
                    PyErr::new::<pyo3::exceptions::PyIOError, _>(format!("Write error: {}", e))
                })?;
            }
            // else: field absent in both cache and inner — skip (None)
        }
        Ok(())
    } else {
        let dict = data.cast::<PyDict>()?;
        serialize_struct_fields(writer, fields, dict, struct_map)
    }
}

pub(crate) fn serialize_struct_fields<W: std::io::Write>(
    writer: &mut BinaryProtocolWriter<W>,
    fields: &[ThriftField],
    data: &Bound<'_, PyDict>,
    struct_map: &HashMap<String, ThriftStruct>,
) -> PyResult<()> {
    for field in fields {
        if let Some(value) = data.get_item(&field.name)? {
            let field_begin = FieldBegin {
                name: None,
                field_type: thrift_type_to_ttype(&field.field_type),
                id: field.id,
            };
            writer.write_field_begin(&field_begin).map_err(|e| {
                PyErr::new::<pyo3::exceptions::PyIOError, _>(format!("Write error: {}", e))
            })?;
            write_value_with_structs(writer, &field.field_type, &value, struct_map)?;
        }
    }
    Ok(())
}

#[allow(dead_code)]
pub(crate) fn deserialize_struct_fields<'py, R: std::io::Read>(
    reader: &mut BinaryProtocolReader<R>,
    fields: &[ThriftField],
    field_map: &HashMap<i16, usize>,
    struct_map: &HashMap<String, ThriftStruct>,
    py: Python<'py>,
) -> PyResult<Bound<'py, PyDict>> {
    let result = PyDict::new(py);
    loop {
        let field_begin = reader.read_field_begin().map_err(|e| {
            PyErr::new::<pyo3::exceptions::PyIOError, _>(format!("Read error: {}", e))
        })?;
        if field_begin.field_type == TType::Stop {
            break;
        }
        if let Some(&idx) = field_map.get(&field_begin.id) {
            let field = &fields[idx];
            let value = read_value_with_structs(reader, &field.field_type, struct_map, py)?;
            result.set_item(&field.name, value)?;
        } else {
            skip_value(reader, field_begin.field_type).map_err(|e| {
                PyErr::new::<pyo3::exceptions::PyIOError, _>(format!("Skip error: {}", e))
            })?;
        }
    }
    Ok(result)
}

/// Deserialise struct fields into a `ThriftStructInstance`.
pub(crate) fn deserialize_struct_fields_as_instance<'py, R: std::io::Read>(
    reader: &mut BinaryProtocolReader<R>,
    fields: &[ThriftField],
    field_map: &HashMap<i16, usize>,
    struct_map: &HashMap<String, ThriftStruct>,
    py: Python<'py>,
    struct_name: &str,
    schema: HashMap<String, ThriftField>,
    struct_map_arc: Arc<HashMap<String, ThriftStruct>>,
) -> PyResult<Bound<'py, ThriftStructInstance>> {
    let mut rust_val = deserialize_rust_struct(reader, fields, field_map, struct_map)
        .map_err(|e| PyErr::new::<pyo3::exceptions::PyIOError, _>(format!("Read error: {}", e)))?;
    rust_val.struct_name = struct_name.to_string();
    let instance = ThriftStructInstance::from_rust(rust_val, schema, struct_map_arc);
    Ok(Bound::new(py, instance)?)
}

// ──────────────────────────────────────────────────────────────────────────────
// GIL-free deserialisation  (bytes → ThriftValue / RustStructValue)
// ──────────────────────────────────────────────────────────────────────────────

/// Read a single Thrift value from the wire into a `ThriftValue`, entirely
/// without touching the GIL.  Nested structs are represented as
/// `ThriftValue::Struct { name, fields }`.
pub(crate) fn read_rust_value<R: std::io::Read>(
    reader: &mut BinaryProtocolReader<R>,
    thrift_type: &ThriftType,
    struct_map: &HashMap<String, ThriftStruct>,
) -> std::io::Result<ThriftValue> {
    match thrift_type {
        ThriftType::Bool => Ok(ThriftValue::Bool(reader.read_bool()?)),
        ThriftType::Byte => Ok(ThriftValue::Byte(reader.read_byte()?)),
        ThriftType::I16 => Ok(ThriftValue::I16(reader.read_i16()?)),
        ThriftType::I32 => Ok(ThriftValue::I32(reader.read_i32()?)),
        ThriftType::I64 => Ok(ThriftValue::I64(reader.read_i64()?)),
        ThriftType::Double => Ok(ThriftValue::Double(reader.read_double()?)),
        ThriftType::String => Ok(ThriftValue::String(reader.read_string()?)),
        ThriftType::Binary => Ok(ThriftValue::Binary(reader.read_binary()?)),
        ThriftType::List(elem_type) => {
            let lb = reader
                .read_list_begin()
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?;
            let mut items = Vec::with_capacity(lb.size as usize);
            for _ in 0..lb.size {
                items.push(read_rust_value(reader, elem_type, struct_map)?);
            }
            Ok(ThriftValue::List(items))
        }
        ThriftType::Set(elem_type) => {
            let sb = reader
                .read_set_begin()
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?;
            let mut items = Vec::with_capacity(sb.size as usize);
            for _ in 0..sb.size {
                items.push(read_rust_value(reader, elem_type, struct_map)?);
            }
            Ok(ThriftValue::Set(items))
        }
        ThriftType::Map(key_type, val_type) => {
            let mb = reader
                .read_map_begin()
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?;
            let mut pairs = Vec::with_capacity(mb.size as usize);
            for _ in 0..mb.size {
                let k = read_rust_value(reader, key_type, struct_map)?;
                let v = read_rust_value(reader, val_type, struct_map)?;
                pairs.push((k, v));
            }
            Ok(ThriftValue::Map(pairs))
        }
        ThriftType::Struct(struct_name) => {
            let struct_def = struct_map.get(struct_name).ok_or_else(|| {
                std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!("Unknown struct type: {}", struct_name),
                )
            })?;
            let fm: HashMap<i16, usize> = struct_def
                .fields
                .iter()
                .enumerate()
                .map(|(i, f)| (f.id, i))
                .collect();
            let nested = deserialize_rust_struct(reader, &struct_def.fields, &fm, struct_map)?;
            Ok(ThriftValue::Struct {
                name: Some(struct_name.clone()),
                fields: nested.values,
            })
        }
    }
}

/// Deserialise Thrift struct fields from the wire into a `RustStructValue`,
/// entirely without touching the GIL.
pub(crate) fn deserialize_rust_struct<R: std::io::Read>(
    reader: &mut BinaryProtocolReader<R>,
    fields: &[ThriftField],
    field_map: &HashMap<i16, usize>,
    struct_map: &HashMap<String, ThriftStruct>,
) -> std::io::Result<RustStructValue> {
    let field_names: Vec<String> = fields.iter().map(|f| f.name.clone()).collect();
    let mut result = RustStructValue {
        struct_name: String::new(), // caller sets struct_name if needed
        field_names,
        values: HashMap::new(),
    };
    loop {
        let field_begin = reader
            .read_field_begin()
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?;
        if field_begin.field_type == TType::Stop {
            break;
        }
        if let Some(&idx) = field_map.get(&field_begin.id) {
            let field = &fields[idx];
            let value = read_rust_value(reader, &field.field_type, struct_map)?;
            result.values.insert(field.name.clone(), value);
        } else {
            skip_value(reader, field_begin.field_type)
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?;
        }
    }
    Ok(result)
}

// ──────────────────────────────────────────────────────────────────────────────
// GIL-free serialisation  (ThriftValue → wire bytes)
// ──────────────────────────────────────────────────────────────────────────────

/// Serialise a `ThriftValue` directly to the wire without touching the GIL.
/// Used by `serialize_struct_any` for fields that are in `inner` but have
/// never been promoted to the Python cache.
pub(crate) fn write_thrift_value<W: std::io::Write>(
    writer: &mut BinaryProtocolWriter<W>,
    val: &ThriftValue,
    struct_map: &HashMap<String, ThriftStruct>,
) -> std::io::Result<()> {
    match val {
        ThriftValue::Bool(v) => writer.write_bool(*v),
        ThriftValue::Byte(v) => writer.write_byte(*v),
        ThriftValue::I16(v) => writer.write_i16(*v),
        ThriftValue::I32(v) => writer.write_i32(*v),
        ThriftValue::I64(v) => writer.write_i64(*v),
        ThriftValue::Double(v) => writer.write_double(*v),
        ThriftValue::String(v) => writer.write_string(v),
        ThriftValue::Binary(v) => writer.write_binary(v),
        ThriftValue::List(items) => {
            let elem_ttype = items
                .first()
                .map(|v| thrift_value_ttype(v))
                .unwrap_or(TType::String);
            writer.write_list_begin(&crate::protocol::ListBegin {
                element_type: elem_ttype,
                size: items.len() as i32,
            })?;
            for item in items {
                write_thrift_value(writer, item, struct_map)?;
            }
            Ok(())
        }
        ThriftValue::Set(items) => {
            let elem_ttype = items
                .first()
                .map(|v| thrift_value_ttype(v))
                .unwrap_or(TType::String);
            writer.write_set_begin(&crate::protocol::SetBegin {
                element_type: elem_ttype,
                size: items.len() as i32,
            })?;
            for item in items {
                write_thrift_value(writer, item, struct_map)?;
            }
            Ok(())
        }
        ThriftValue::Map(pairs) => {
            let (kt, vt) = pairs
                .first()
                .map(|(k, v)| (thrift_value_ttype(k), thrift_value_ttype(v)))
                .unwrap_or((TType::String, TType::String));
            writer.write_map_begin(&crate::protocol::MapBegin {
                key_type: kt,
                value_type: vt,
                size: pairs.len() as i32,
            })?;
            for (k, v) in pairs {
                write_thrift_value(writer, k, struct_map)?;
                write_thrift_value(writer, v, struct_map)?;
            }
            Ok(())
        }
        ThriftValue::Struct { name, fields } => {
            // Write each field in definition order if we know the struct def,
            // otherwise fall back to arbitrary key order.
            let ordered_fields: Vec<(&String, &ThriftValue)> = if let Some(sname) = name {
                if let Some(def) = struct_map.get(sname) {
                    def.fields
                        .iter()
                        .filter_map(|fd| fields.get(&fd.name).map(|v| (&fd.name, v)))
                        .collect()
                } else {
                    fields.iter().collect()
                }
            } else {
                fields.iter().collect()
            };

            // Look up field ids from the struct_map so we write correct wire ids.
            let struct_def = name.as_deref().and_then(|n| struct_map.get(n));

            for (fname, fval) in &ordered_fields {
                let field_id: i16 = struct_def
                    .and_then(|def| def.fields.iter().find(|f| &f.name == *fname))
                    .map(|f| f.id)
                    .unwrap_or(0);
                let ttype = thrift_value_ttype(fval);
                let field_begin = crate::protocol::FieldBegin {
                    name: None,
                    field_type: ttype,
                    id: field_id,
                };
                writer.write_field_begin(&field_begin)?;
                write_thrift_value(writer, fval, struct_map)?;
            }
            writer.write_field_stop()?;
            Ok(())
        }
    }
}

/// Derive the `TType` wire tag from a `ThriftValue` without schema info.
#[inline]
fn thrift_value_ttype(val: &ThriftValue) -> TType {
    match val {
        ThriftValue::Bool(_) => TType::Bool,
        ThriftValue::Byte(_) => TType::Byte,
        ThriftValue::I16(_) => TType::I16,
        ThriftValue::I32(_) => TType::I32,
        ThriftValue::I64(_) => TType::I64,
        ThriftValue::Double(_) => TType::Double,
        ThriftValue::String(_) => TType::String,
        ThriftValue::Binary(_) => TType::String,
        ThriftValue::List(_) => TType::List,
        ThriftValue::Set(_) => TType::Set,
        ThriftValue::Map(_) => TType::Map,
        ThriftValue::Struct { .. } => TType::Struct,
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// Type helpers
// ──────────────────────────────────────────────────────────────────────────────

#[inline]
pub(crate) fn thrift_type_to_ttype(thrift_type: &ThriftType) -> TType {
    match thrift_type {
        ThriftType::Bool => TType::Bool,
        ThriftType::Byte => TType::Byte,
        ThriftType::I16 => TType::I16,
        ThriftType::I32 => TType::I32,
        ThriftType::I64 => TType::I64,
        ThriftType::Double => TType::Double,
        ThriftType::String => TType::String,
        ThriftType::Binary => TType::String,
        ThriftType::List(_) => TType::List,
        ThriftType::Set(_) => TType::Set,
        ThriftType::Map(_, _) => TType::Map,
        ThriftType::Struct(_) => TType::Struct,
    }
}

fn write_value_with_structs<'py, W: std::io::Write>(
    writer: &mut BinaryProtocolWriter<W>,
    thrift_type: &ThriftType,
    value: &Bound<'py, PyAny>,
    struct_map: &HashMap<String, ThriftStruct>,
) -> PyResult<()> {
    match thrift_type {
        ThriftType::Bool => {
            writer
                .write_bool(value.extract()?)
                .map_err(|e| PyErr::new::<pyo3::exceptions::PyIOError, _>(e.to_string()))?;
        }
        ThriftType::Byte => {
            writer
                .write_byte(value.extract()?)
                .map_err(|e| PyErr::new::<pyo3::exceptions::PyIOError, _>(e.to_string()))?;
        }
        ThriftType::I16 => {
            writer
                .write_i16(value.extract()?)
                .map_err(|e| PyErr::new::<pyo3::exceptions::PyIOError, _>(e.to_string()))?;
        }
        ThriftType::I32 => {
            writer
                .write_i32(value.extract()?)
                .map_err(|e| PyErr::new::<pyo3::exceptions::PyIOError, _>(e.to_string()))?;
        }
        ThriftType::I64 => {
            writer
                .write_i64(value.extract()?)
                .map_err(|e| PyErr::new::<pyo3::exceptions::PyIOError, _>(e.to_string()))?;
        }
        ThriftType::Double => {
            writer
                .write_double(value.extract()?)
                .map_err(|e| PyErr::new::<pyo3::exceptions::PyIOError, _>(e.to_string()))?;
        }
        ThriftType::String => {
            let val: String = value.extract()?;
            writer
                .write_string(&val)
                .map_err(|e| PyErr::new::<pyo3::exceptions::PyIOError, _>(e.to_string()))?;
        }
        ThriftType::Binary => {
            let val: Vec<u8> = value.extract()?;
            writer
                .write_binary(&val)
                .map_err(|e| PyErr::new::<pyo3::exceptions::PyIOError, _>(e.to_string()))?;
        }
        ThriftType::List(elem_type) | ThriftType::Set(elem_type) => {
            use crate::protocol::ListBegin;
            let list = value.cast::<PyList>()?;
            let lb = ListBegin {
                element_type: thrift_type_to_ttype(elem_type),
                size: list.len() as i32,
            };
            if matches!(thrift_type, ThriftType::List(_)) {
                writer
                    .write_list_begin(&lb)
                    .map_err(|e| PyErr::new::<pyo3::exceptions::PyIOError, _>(e.to_string()))?;
            } else {
                use crate::protocol::SetBegin;
                writer
                    .write_set_begin(&SetBegin {
                        element_type: lb.element_type,
                        size: lb.size,
                    })
                    .map_err(|e| PyErr::new::<pyo3::exceptions::PyIOError, _>(e.to_string()))?;
            }
            for item in list.iter() {
                write_value_with_structs(writer, elem_type, &item, struct_map)?;
            }
        }
        ThriftType::Map(key_type, val_type) => {
            use crate::protocol::MapBegin;
            use pyo3::types::PyDict as PyDictType;
            let dict = value.cast::<PyDictType>()?;
            let mb = MapBegin {
                key_type: thrift_type_to_ttype(key_type),
                value_type: thrift_type_to_ttype(val_type),
                size: dict.len() as i32,
            };
            writer
                .write_map_begin(&mb)
                .map_err(|e| PyErr::new::<pyo3::exceptions::PyIOError, _>(e.to_string()))?;
            for (k, v) in dict.iter() {
                write_value_with_structs(writer, key_type, &k, struct_map)?;
                write_value_with_structs(writer, val_type, &v, struct_map)?;
            }
        }
        ThriftType::Struct(struct_name) => {
            let struct_def = struct_map.get(struct_name).ok_or_else(|| {
                PyErr::new::<pyo3::exceptions::PyValueError, _>(format!(
                    "Unknown struct type: {}",
                    struct_name
                ))
            })?;
            let py = value.py();
            serialize_struct_any(writer, &struct_def.fields, value, struct_map, py)?;
            writer
                .write_field_stop()
                .map_err(|e| PyErr::new::<pyo3::exceptions::PyIOError, _>(e.to_string()))?;
        }
    }
    Ok(())
}

#[allow(dead_code)]
fn read_value_with_structs<'py, R: std::io::Read>(
    reader: &mut BinaryProtocolReader<R>,
    thrift_type: &ThriftType,
    struct_map: &HashMap<String, ThriftStruct>,
    py: Python<'py>,
) -> PyResult<Py<PyAny>> {
    match thrift_type {
        ThriftType::Bool => {
            let val = reader
                .read_bool()
                .map_err(|e| PyErr::new::<pyo3::exceptions::PyIOError, _>(e.to_string()))?;
            Ok(val
                .into_pyobject(py)
                .unwrap()
                .to_owned()
                .into_any()
                .unbind())
        }
        ThriftType::Byte => {
            let val = reader
                .read_byte()
                .map_err(|e| PyErr::new::<pyo3::exceptions::PyIOError, _>(e.to_string()))?;
            Ok(val.into_pyobject(py).unwrap().into_any().unbind())
        }
        ThriftType::I16 => {
            let val = reader
                .read_i16()
                .map_err(|e| PyErr::new::<pyo3::exceptions::PyIOError, _>(e.to_string()))?;
            Ok(val.into_pyobject(py).unwrap().into_any().unbind())
        }
        ThriftType::I32 => {
            let val = reader
                .read_i32()
                .map_err(|e| PyErr::new::<pyo3::exceptions::PyIOError, _>(e.to_string()))?;
            Ok(val.into_pyobject(py).unwrap().into_any().unbind())
        }
        ThriftType::I64 => {
            let val = reader
                .read_i64()
                .map_err(|e| PyErr::new::<pyo3::exceptions::PyIOError, _>(e.to_string()))?;
            Ok(val.into_pyobject(py).unwrap().into_any().unbind())
        }
        ThriftType::Double => {
            let val = reader
                .read_double()
                .map_err(|e| PyErr::new::<pyo3::exceptions::PyIOError, _>(e.to_string()))?;
            Ok(val.into_pyobject(py).unwrap().into_any().unbind())
        }
        ThriftType::String => {
            let val = reader
                .read_string()
                .map_err(|e| PyErr::new::<pyo3::exceptions::PyIOError, _>(e.to_string()))?;
            Ok(val.into_pyobject(py).unwrap().into_any().unbind())
        }
        ThriftType::Binary => {
            let val = reader
                .read_binary()
                .map_err(|e| PyErr::new::<pyo3::exceptions::PyIOError, _>(e.to_string()))?;
            Ok(PyBytes::new(py, &val).into_any().unbind())
        }
        ThriftType::List(elem_type) | ThriftType::Set(elem_type) => {
            let (_elem_ttype, size) = if matches!(thrift_type, ThriftType::List(_)) {
                let lb = reader
                    .read_list_begin()
                    .map_err(|e| PyErr::new::<pyo3::exceptions::PyIOError, _>(e.to_string()))?;
                (lb.element_type, lb.size)
            } else {
                let sb = reader
                    .read_set_begin()
                    .map_err(|e| PyErr::new::<pyo3::exceptions::PyIOError, _>(e.to_string()))?;
                (sb.element_type, sb.size)
            };
            let list = PyList::empty(py);
            for _ in 0..size {
                let item = read_value_with_structs(reader, elem_type, struct_map, py)?;
                list.append(item.bind(py))?;
            }
            Ok(list.into_any().unbind())
        }
        ThriftType::Map(key_type, val_type) => {
            use pyo3::types::PyDict as PyDictType;
            let mb = reader
                .read_map_begin()
                .map_err(|e| PyErr::new::<pyo3::exceptions::PyIOError, _>(e.to_string()))?;
            let dict = PyDictType::new(py);
            for _ in 0..mb.size {
                let k = read_value_with_structs(reader, key_type, struct_map, py)?;
                let v = read_value_with_structs(reader, val_type, struct_map, py)?;
                dict.set_item(k.bind(py), v.bind(py))?;
            }
            Ok(dict.into_any().unbind())
        }
        ThriftType::Struct(struct_name) => {
            let struct_def = struct_map.get(struct_name).ok_or_else(|| {
                PyErr::new::<pyo3::exceptions::PyValueError, _>(format!(
                    "Unknown struct type: {}",
                    struct_name
                ))
            })?;
            let fm: HashMap<i16, usize> = struct_def
                .fields
                .iter()
                .enumerate()
                .map(|(i, f)| (f.id, i))
                .collect();
            let schema: HashMap<String, ThriftField> = struct_def
                .fields
                .iter()
                .map(|f| (f.name.clone(), f.clone()))
                .collect();
            let instance = deserialize_struct_fields_as_instance(
                reader,
                &struct_def.fields,
                &fm,
                struct_map,
                py,
                struct_name,
                schema,
                struct_def.struct_map.clone(),
            )?;
            Ok(instance.into_any().unbind())
        }
    }
}

/// Skip over a value of the given wire type without allocating Python objects.
fn skip_value<R: std::io::Read>(
    reader: &mut BinaryProtocolReader<R>,
    ttype: TType,
) -> std::io::Result<()> {
    match ttype {
        TType::Bool | TType::Byte => {
            reader.read_u8_raw()?;
        }
        TType::I16 => {
            reader.read_i16_raw()?;
        }
        TType::I32 => {
            reader.read_i32_raw()?;
        }
        TType::I64 | TType::Double => {
            reader.read_i64_raw()?;
        }
        TType::String => {
            reader.read_string()?;
        }
        TType::Struct => loop {
            let ft = reader.read_u8_raw()?;
            let ft = TType::from_u8(ft)
                .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidData, "bad type"))?;
            if ft == TType::Stop {
                break;
            }
            reader.read_i16_raw()?;
            skip_value(reader, ft)?;
        },
        TType::Map => {
            let key_type = TType::from_u8(reader.read_u8_raw()?)
                .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidData, "bad type"))?;
            let val_type = TType::from_u8(reader.read_u8_raw()?)
                .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidData, "bad type"))?;
            let size = reader.read_i32_raw()?;
            for _ in 0..size {
                skip_value(reader, key_type)?;
                skip_value(reader, val_type)?;
            }
        }
        TType::List | TType::Set => {
            let elem_type = TType::from_u8(reader.read_u8_raw()?)
                .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidData, "bad type"))?;
            let size = reader.read_i32_raw()?;
            for _ in 0..size {
                skip_value(reader, elem_type)?;
            }
        }
        _ => {}
    }
    Ok(())
}

/// Convert a `ThriftValue` to a Python object.  Nested structs become
/// `ThriftStructInstance` with `inner` pre-populated and an empty cache.
pub(crate) fn thrift_value_to_py(val: &ThriftValue, py: Python<'_>) -> PyResult<Py<PyAny>> {
    match val {
        ThriftValue::Bool(v) => Ok(v.into_pyobject(py).unwrap().to_owned().into_any().unbind()),
        ThriftValue::Byte(v) => Ok(v.into_pyobject(py).unwrap().into_any().unbind()),
        ThriftValue::I16(v) => Ok(v.into_pyobject(py).unwrap().into_any().unbind()),
        ThriftValue::I32(v) => Ok(v.into_pyobject(py).unwrap().into_any().unbind()),
        ThriftValue::I64(v) => Ok(v.into_pyobject(py).unwrap().into_any().unbind()),
        ThriftValue::Double(v) => Ok(v.into_pyobject(py).unwrap().into_any().unbind()),
        ThriftValue::String(v) => Ok(v.into_pyobject(py).unwrap().into_any().unbind()),
        ThriftValue::Binary(v) => Ok(PyBytes::new(py, v).into_any().unbind()),
        ThriftValue::List(items) | ThriftValue::Set(items) => {
            let list = PyList::empty(py);
            for item in items {
                list.append(thrift_value_to_py(item, py)?.bind(py))?;
            }
            Ok(list.into_any().unbind())
        }
        ThriftValue::Map(pairs) => {
            let d = PyDict::new(py);
            for (k, v) in pairs {
                d.set_item(
                    thrift_value_to_py(k, py)?.bind(py),
                    thrift_value_to_py(v, py)?.bind(py),
                )?;
            }
            Ok(d.into_any().unbind())
        }
        ThriftValue::Struct { name, fields } => {
            let field_names: Vec<String> = fields.keys().cloned().collect();
            let struct_name = name.clone().unwrap_or_default();
            let inner = RustStructValue {
                struct_name: struct_name.clone(),
                field_names: field_names.clone(),
                values: fields.clone(),
            };
            let instance =
                ThriftStructInstance::from_rust(inner, HashMap::new(), Arc::new(HashMap::new()));
            Ok(Bound::new(py, instance)?.into_any().unbind())
        }
    }
}

/// Best-effort conversion of an arbitrary Python object to a `ThriftValue`
/// without schema type information.  Used by `__setattr__` to keep `inner`
/// in sync.  Returns `Err` for values that cannot be represented (e.g. None).
pub(crate) fn py_any_to_thrift_value(val: &Bound<'_, PyAny>) -> PyResult<ThriftValue> {
    if val.is_none() {
        return Err(PyErr::new::<pyo3::exceptions::PyTypeError, _>(
            "None has no ThriftValue representation",
        ));
    }
    // bool must come before i64 because Python bool is a subclass of int.
    if let Ok(v) = val.extract::<bool>() {
        return Ok(ThriftValue::Bool(v));
    }
    if let Ok(v) = val.extract::<i64>() {
        return Ok(ThriftValue::I64(v));
    }
    if let Ok(v) = val.extract::<f64>() {
        return Ok(ThriftValue::Double(v));
    }
    if let Ok(v) = val.extract::<String>() {
        return Ok(ThriftValue::String(v));
    }
    if let Ok(v) = val.extract::<Vec<u8>>() {
        return Ok(ThriftValue::Binary(v));
    }
    if let Ok(list) = val.cast::<PyList>() {
        let items: PyResult<Vec<ThriftValue>> =
            list.iter().map(|i| py_any_to_thrift_value(&i)).collect();
        return Ok(ThriftValue::List(items?));
    }
    if let Ok(dict) = val.cast::<PyDict>() {
        let pairs: PyResult<Vec<(ThriftValue, ThriftValue)>> = dict
            .iter()
            .map(|(k, v)| Ok((py_any_to_thrift_value(&k)?, py_any_to_thrift_value(&v)?)))
            .collect();
        return Ok(ThriftValue::Map(pairs?));
    }
    if let Ok(inst) = val.cast::<ThriftStructInstance>() {
        let inst = inst.borrow();
        let mut fields = inst.inner.values.clone();
        // Cache entries override inner — use schema-aware coercion when available.
        let has_schema = !inst.schema.is_empty();
        for name in &inst.field_names {
            if let Some(py_val) = inst.cache.get(name) {
                let bound = py_val.bind(val.py());
                let tv_result = if has_schema {
                    if let Some(field) = inst.schema.get(name) {
                        py_any_to_thrift_value_with_type(
                            bound,
                            &field.field_type.clone(),
                            &inst.struct_map,
                        )
                    } else {
                        py_any_to_thrift_value(bound)
                    }
                } else {
                    py_any_to_thrift_value(bound)
                };
                if let Ok(tv) = tv_result {
                    fields.insert(name.clone(), tv);
                }
            }
        }
        return Ok(ThriftValue::Struct {
            name: Some(inst.struct_name.clone()),
            fields,
        });
    }
    Err(PyErr::new::<pyo3::exceptions::PyTypeError, _>(format!(
        "Cannot convert Python value of type '{}' to ThriftValue",
        val.get_type()
            .qualname()
            .map(|s| s.to_string())
            .unwrap_or_else(|_| "<unknown>".to_string())
    )))
}

/// Schema-aware conversion of a Python value to the exact `ThriftValue` variant
/// dictated by `thrift_type`.  Falls back to `py_any_to_thrift_value` for
/// primitive scalars where the schema just confirms the obvious mapping.
pub(crate) fn py_any_to_thrift_value_with_type(
    val: &Bound<'_, PyAny>,
    thrift_type: &ThriftType,
    struct_map: &Arc<HashMap<String, ThriftStruct>>,
) -> PyResult<ThriftValue> {
    if val.is_none() {
        return Err(PyErr::new::<pyo3::exceptions::PyTypeError, _>(
            "None has no ThriftValue representation",
        ));
    }
    match thrift_type {
        ThriftType::Bool => Ok(ThriftValue::Bool(val.extract::<bool>()?)),
        ThriftType::Byte => Ok(ThriftValue::Byte(val.extract::<i8>()?)),
        ThriftType::I16 => {
            // Python int can always fit in i16 if the value is in range.
            Ok(ThriftValue::I16(val.extract::<i16>()?))
        }
        ThriftType::I32 => Ok(ThriftValue::I32(val.extract::<i32>()?)),
        ThriftType::I64 => Ok(ThriftValue::I64(val.extract::<i64>()?)),
        ThriftType::Double => Ok(ThriftValue::Double(val.extract::<f64>()?)),
        ThriftType::String => Ok(ThriftValue::String(val.extract::<String>()?)),
        ThriftType::Binary => Ok(ThriftValue::Binary(val.extract::<Vec<u8>>()?)),
        ThriftType::List(elem_type) => {
            let list = val.cast::<PyList>()?;
            let items: PyResult<Vec<ThriftValue>> = list
                .iter()
                .map(|i| py_any_to_thrift_value_with_type(&i, elem_type, struct_map))
                .collect();
            Ok(ThriftValue::List(items?))
        }
        ThriftType::Set(elem_type) => {
            let list = val.cast::<PyList>()?;
            let items: PyResult<Vec<ThriftValue>> = list
                .iter()
                .map(|i| py_any_to_thrift_value_with_type(&i, elem_type, struct_map))
                .collect();
            Ok(ThriftValue::Set(items?))
        }
        ThriftType::Map(key_type, val_type) => {
            let dict = val.cast::<PyDict>()?;
            let pairs: PyResult<Vec<(ThriftValue, ThriftValue)>> = dict
                .iter()
                .map(|(k, v)| {
                    Ok((
                        py_any_to_thrift_value_with_type(&k, key_type, struct_map)?,
                        py_any_to_thrift_value_with_type(&v, val_type, struct_map)?,
                    ))
                })
                .collect();
            Ok(ThriftValue::Map(pairs?))
        }
        ThriftType::Struct(struct_name) => {
            if let Ok(inst) = val.cast::<ThriftStructInstance>() {
                let inst = inst.borrow();
                // Resolve field schema from local instance or global struct_map.
                let schema: Option<&ThriftStruct> = struct_map.get(struct_name.as_str());
                let mut fields = inst.inner.values.clone();
                for name in &inst.field_names {
                    if let Some(py_val) = inst.cache.get(name) {
                        let bound = py_val.bind(val.py());
                        // Use schema from global struct_map for coercion if available.
                        let tv_result = if let Some(def) = schema {
                            if let Some(field) = def.fields.iter().find(|f| f.name == *name) {
                                py_any_to_thrift_value_with_type(
                                    bound,
                                    &field.field_type.clone(),
                                    struct_map,
                                )
                            } else {
                                py_any_to_thrift_value(bound)
                            }
                        } else if let Some(field) = inst.schema.get(name) {
                            py_any_to_thrift_value_with_type(
                                bound,
                                &field.field_type.clone(),
                                &inst.struct_map,
                            )
                        } else {
                            py_any_to_thrift_value(bound)
                        };
                        if let Ok(tv) = tv_result {
                            fields.insert(name.clone(), tv);
                        }
                    }
                }
                Ok(ThriftValue::Struct {
                    name: Some(inst.struct_name.clone()),
                    fields,
                })
            } else if let Ok(dict) = val.cast::<PyDict>() {
                // Plain dict: use struct_map for coercion if available.
                let schema_def = struct_map.get(struct_name.as_str());
                let mut fields: HashMap<String, ThriftValue> = HashMap::new();
                for (k, v) in dict.iter() {
                    let field_name: String = k.extract()?;
                    let tv = if let Some(def) = schema_def {
                        if let Some(field) = def.fields.iter().find(|f| f.name == field_name) {
                            py_any_to_thrift_value_with_type(
                                &v,
                                &field.field_type.clone(),
                                struct_map,
                            )
                            .unwrap_or_else(|_| {
                                py_any_to_thrift_value(&v)
                                    .unwrap_or(ThriftValue::String(String::new()))
                            })
                        } else {
                            py_any_to_thrift_value(&v).unwrap_or(ThriftValue::String(String::new()))
                        }
                    } else {
                        py_any_to_thrift_value(&v)?
                    };
                    fields.insert(field_name, tv);
                }
                Ok(ThriftValue::Struct {
                    name: Some(struct_name.clone()),
                    fields,
                })
            } else {
                // Fall back to schemaless conversion.
                py_any_to_thrift_value(val)
            }
        }
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// ThriftApplicationException
// Raised by ThriftClient.call() when the server returns MESSAGE_TYPE_EXCEPTION.
// ──────────────────────────────────────────────────────────────────────────────

/// A Thrift application-level exception returned by the remote server.
///
/// Attributes
/// ----------
/// message : str
///     Human-readable error message sent by the server.
/// type_ : int
///     TApplicationException type code (1 = UNKNOWN, 6 = INTERNAL_ERROR, …).
#[pyclass(extends = pyo3::exceptions::PyException)]
#[derive(Debug)]
pub struct ThriftApplicationException {
    #[pyo3(get)]
    pub message: String,
    #[pyo3(get)]
    pub type_: i32,
}

#[pymethods]
impl ThriftApplicationException {
    #[new]
    pub fn new(message: String, type_: i32) -> Self {
        ThriftApplicationException {
            message: message.clone(),
            type_,
        }
    }

    pub fn __repr__(&self) -> String {
        format!(
            "ThriftApplicationException(type={}, message={:?})",
            self.type_, self.message
        )
    }

    pub fn __str__(&self) -> String {
        self.message.clone()
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// ThriftClient
// ──────────────────────────────────────────────────────────────────────────────

/// A synchronous Thrift Binary Protocol client.
///
/// The socket I/O is performed **outside** the GIL so other Python threads can
/// run while a call is in-flight.  A `Mutex` guards the single `TcpStream` so
/// the client can safely be used from multiple Python threads (calls are
/// serialised at the socket level).
///
/// Usage::
///
///     client = ThriftClient(service_def, "127.0.0.1", 9090,
///                           TransportType.Buffered)
///     client.set_parser(thrift_module._parser)
///     client.open()
///     result = client.call("get_user", user_id=1)
///     client.close()
///
/// Or as a context manager::
///
///     with ThriftClient(service_def, "127.0.0.1", 9090) as client:
///         result = client.call("get_user", user_id=1)
#[pyclass]
pub struct ThriftClient {
    service: PyThriftService,
    struct_map: Arc<HashMap<String, ThriftStruct>>,
    transport: TransportType,
    host: String,
    port: u16,
    /// The live TCP connection — `None` until `open()` is called.
    socket: Mutex<Option<TcpStream>>,
    /// Monotonically-increasing sequence-id counter.
    seq_id: AtomicI32,
}

#[pymethods]
impl ThriftClient {
    #[new]
    #[pyo3(signature = (service, host, port, transport = TransportType::Framed))]
    pub fn new(
        service: PyThriftService,
        host: String,
        port: u16,
        transport: TransportType,
    ) -> Self {
        Self {
            service,
            struct_map: Arc::new(HashMap::new()),
            transport,
            host,
            port,
            socket: Mutex::new(None),
            seq_id: AtomicI32::new(0),
        }
    }

    /// Attach the parser so nested struct types can be resolved during
    /// argument serialisation and return-value deserialisation.
    pub fn set_parser(&mut self, parser: &ThriftParser) {
        self.struct_map = parser.struct_map();
    }

    /// Open the TCP connection to the remote server.
    /// Releases the GIL while the OS is establishing the connection.
    pub fn open(&self, py: Python<'_>) -> PyResult<()> {
        let addr = format!("{}:{}", self.host, self.port);
        let stream = py.detach(|| TcpStream::connect(&addr)).map_err(|e| {
            PyErr::new::<pyo3::exceptions::PyOSError, _>(format!("connect to {}: {}", addr, e))
        })?;
        // Disable Nagle's algorithm for low-latency request/response.
        let _ = stream.set_nodelay(true);
        let mut guard = self.socket.lock().unwrap();
        *guard = Some(stream);
        Ok(())
    }

    /// Close the TCP connection.
    pub fn close(&self) {
        let mut guard = self.socket.lock().unwrap();
        *guard = None; // Drop closes the TcpStream.
    }

    /// Returns `True` if the socket is currently open.
    pub fn is_open(&self) -> bool {
        self.socket.lock().unwrap().is_some()
    }

    // ── Context-manager support ──────────────────────────────────────────────

    pub fn __enter__<'a>(slf: PyRef<'a, Self>, py: Python<'a>) -> PyResult<PyRef<'a, Self>> {
        slf.open(py)?;
        Ok(slf)
    }

    pub fn __exit__(
        &self,
        _exc_type: &Bound<'_, PyAny>,
        _exc_value: &Bound<'_, PyAny>,
        _traceback: &Bound<'_, PyAny>,
    ) {
        self.close();
    }

    // ── Transport getter/setter (mirrors ThriftServer) ───────────────────────

    #[getter]
    pub fn transport(&self) -> TransportType {
        self.transport
    }

    #[setter]
    pub fn set_transport(&mut self, transport: TransportType) {
        self.transport = transport;
    }

    /// Invoke a remote method.
    ///
    /// Parameters
    /// ----------
    /// method_name : str
    ///     Name of the service method to call.
    /// **kwargs
    ///     Keyword arguments matching the method's argument names.
    ///
    /// Returns
    /// -------
    /// The deserialised return value, or ``None`` for ``void`` methods.
    ///
    /// Raises
    /// ------
    /// ThriftApplicationException
    ///     If the server replies with a Thrift application-exception.
    /// OSError
    ///     If the socket is not open or the network call fails.
    #[pyo3(signature = (method_name, **kwargs))]
    pub fn call(
        &self,
        py: Python<'_>,
        method_name: &str,
        kwargs: Option<&Bound<'_, PyDict>>,
    ) -> PyResult<Py<PyAny>> {
        // ── Find the method definition ────────────────────────────────────────
        let method = self
            .service
            .methods
            .iter()
            .find(|m| m.name == method_name)
            .ok_or_else(|| {
                PyErr::new::<pyo3::exceptions::PyValueError, _>(format!(
                    "Unknown method: {}",
                    method_name
                ))
            })?
            .clone();

        let seq_id = self.seq_id.fetch_add(1, Ordering::Relaxed);

        // ── Serialise the call frame (GIL held — pure in-memory work) ─────────
        let call_frame: Vec<u8> = {
            let empty_dict;
            let kw: &Bound<'_, PyDict> = if let Some(k) = kwargs {
                k
            } else {
                empty_dict = PyDict::new(py);
                &empty_dict
            };

            // Build the message header.
            let mut buf = Vec::with_capacity(256);
            {
                let mut writer = BinaryProtocolWriter::new(&mut buf);
                writer
                    .write_message_begin(method_name, MESSAGE_TYPE_CALL, seq_id)
                    .map_err(|e| PyErr::new::<pyo3::exceptions::PyIOError, _>(e.to_string()))?;
            }

            // Serialise each argument field in definition order.
            {
                let mut writer = BinaryProtocolWriter::new(&mut buf);
                for field in &method.arguments {
                    // Fetch the value from kwargs by field name, skip if absent.
                    let value = match kw.get_item(&field.name)? {
                        Some(v) => v,
                        None => continue,
                    };
                    // Write field header.
                    let fb = FieldBegin {
                        name: None,
                        field_type: thrift_type_to_ttype(&field.field_type),
                        id: field.id,
                    };
                    writer
                        .write_field_begin(&fb)
                        .map_err(|e| PyErr::new::<pyo3::exceptions::PyIOError, _>(e.to_string()))?;
                    write_value_with_structs(
                        &mut writer,
                        &field.field_type,
                        &value,
                        &self.struct_map,
                    )?;
                }
                writer
                    .write_field_stop()
                    .map_err(|e| PyErr::new::<pyo3::exceptions::PyIOError, _>(e.to_string()))?;
            }

            buf
        };

        // ── Capture return type and struct_map before releasing the GIL ───────
        let return_type = method.return_type.clone();
        let struct_map = Arc::clone(&self.struct_map);
        let transport = self.transport;

        // ── Network round-trip — GIL released ────────────────────────────────
        let reply_payload: Vec<u8> = py
            .detach(|| -> Result<Vec<u8>, String> {
                let mut guard = self.socket.lock().unwrap();
                let stream = guard.as_mut().ok_or_else(|| {
                    "ThriftClient is not open; call client.open() first".to_string()
                })?;

                // Send the call frame.
                client_send_frame(stream, &call_frame, transport)
                    .map_err(|e| format!("send error: {}", e))?;

                // Receive the reply frame.
                client_recv_frame(stream, transport).map_err(|e| format!("recv error: {}", e))
            })
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyOSError, _>(e))?;

        // ── Deserialise reply (GIL held again) ────────────────────────────────
        let mut cursor = Cursor::new(&reply_payload[..]);
        let mut reader = BinaryProtocolReader::new(&mut cursor);

        let msg_begin = reader
            .read_message_begin()
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyIOError, _>(e.to_string()))?;

        if msg_begin.message_type == MESSAGE_TYPE_EXCEPTION {
            // Deserialise TApplicationException: field 1 = message (string),
            // field 2 = type (i32).
            let (ex_msg, ex_type) = read_application_exception(&mut reader)
                .map_err(|e| PyErr::new::<pyo3::exceptions::PyIOError, _>(e.to_string()))?;
            // Raise as ThriftApplicationException.
            return Err(PyErr::new::<ThriftApplicationException, _>((
                ex_msg, ex_type,
            )));
        }

        // MESSAGE_TYPE_REPLY — field 0 = success, field N = declared exception.
        // Read the single reply field.
        let field_begin = reader
            .read_field_begin()
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyIOError, _>(e.to_string()))?;

        if field_begin.field_type == TType::Stop {
            // void method or empty reply.
            return Ok(py.None());
        }

        if field_begin.id != 0 {
            // Non-zero field id in a reply means a declared service exception.
            // Read and deserialise it as a struct, then raise as a Python RuntimeError.
            // (Declared exceptions could be raised as proper typed exceptions in future.)
            let ex_val = read_rust_value(&mut reader, &return_type, &struct_map)
                .map_err(|e| PyErr::new::<pyo3::exceptions::PyIOError, _>(e.to_string()));
            let ex_py = match ex_val {
                Ok(tv) => thrift_value_to_py(&tv, py)?,
                Err(_) => py.None(),
            };
            return Err(PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(format!(
                "Service exception (field {}): {:?}",
                field_begin.id,
                ex_py
                    .bind(py)
                    .repr()
                    .map(|s| s.to_string())
                    .unwrap_or_default()
            )));
        }

        // Field 0 = success result.
        let rust_val = read_rust_value(&mut reader, &return_type, &struct_map)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyIOError, _>(e.to_string()))?;
        thrift_value_to_py(&rust_val, py)
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// Client I/O helpers  (GIL-free, no Python types)
// ──────────────────────────────────────────────────────────────────────────────

/// Send a call frame over the socket, respecting the chosen transport framing.
/// Uses a `BufWriter` to coalesce the length header and payload into a single
/// `flush()` call instead of two separate `write_all` syscalls.
fn client_send_frame(
    stream: &mut TcpStream,
    payload: &[u8],
    transport: TransportType,
) -> std::io::Result<()> {
    use byteorder::WriteBytesExt;
    let mut w: BufWriter<&mut TcpStream> = BufWriter::with_capacity(payload.len() + 4, stream);
    match transport {
        TransportType::Framed => {
            w.write_i32::<BigEndian>(payload.len() as i32)?;
            w.write_all(payload)?;
        }
        TransportType::Buffered => {
            w.write_all(payload)?;
        }
    }
    w.flush()
}

/// Receive a reply frame from the socket, reassembling it into a single
/// contiguous buffer (mirrors the server-side read logic).
fn client_recv_frame(stream: &mut TcpStream, transport: TransportType) -> std::io::Result<Vec<u8>> {
    use byteorder::ReadBytesExt;
    use std::io::Read;

    match transport {
        TransportType::Framed => {
            let frame_len = match stream.read_i32::<BigEndian>() {
                Ok(n) if n >= 0 => n as usize,
                Ok(_) => return Ok(vec![]),
                Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(vec![]),
                Err(e) => return Err(e),
            };
            let mut buf = vec![0u8; frame_len];
            stream.read_exact(&mut buf)?;
            Ok(buf)
        }
        TransportType::Buffered => {
            // Wrap in BufReader for efficient byte-at-a-time reads.
            let mut r: BufReader<&mut TcpStream> = BufReader::with_capacity(65536, stream);

            // 1. Version + type (4 bytes)
            let mut hdr = [0u8; 4];
            r.read_exact(&mut hdr)?;

            // 2. Name length + name + seq_id
            let mut name_len_bytes = [0u8; 4];
            r.read_exact(&mut name_len_bytes)?;
            let name_len = i32::from_be_bytes(name_len_bytes) as usize;
            let mut name_buf = vec![0u8; name_len];
            r.read_exact(&mut name_buf)?;
            let mut seq_id_bytes = [0u8; 4];
            r.read_exact(&mut seq_id_bytes)?;

            // 3. Struct body up to the outermost STOP byte.
            let mut body: Vec<u8> = Vec::with_capacity(256);
            read_buffered_struct_body(&mut r, &mut body)?;

            // 4. Reassemble.
            let mut frame = Vec::with_capacity(4 + 4 + name_len + 4 + body.len());
            frame.extend_from_slice(&hdr);
            frame.extend_from_slice(&name_len_bytes);
            frame.extend_from_slice(&name_buf);
            frame.extend_from_slice(&seq_id_bytes);
            frame.extend_from_slice(&body);
            Ok(frame)
        }
    }
}

/// Deserialise the two fields of a `TApplicationException` struct from the wire.
/// Returns `(message, type_code)`.
fn read_application_exception<R: std::io::Read>(
    reader: &mut BinaryProtocolReader<R>,
) -> std::io::Result<(String, i32)> {
    use byteorder::ReadBytesExt;
    let mut msg = String::new();
    let mut type_code: i32 = 0;
    loop {
        let ft = reader.read_u8_raw()?;
        let ttype = TType::from_u8(ft)
            .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidData, "bad type"))?;
        if ttype == TType::Stop {
            break;
        }
        let id = reader.read_i16_raw()?;
        match (ttype, id) {
            (TType::String, 1) => {
                msg = reader.read_string()?;
            }
            (TType::I32, 2) => {
                type_code = reader.read_i32_raw()?;
            }
            _ => {
                // Skip unknown field.
                skip_value(reader, ttype).map_err(|e| {
                    std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string())
                })?;
            }
        }
    }
    Ok((msg, type_code))
}
