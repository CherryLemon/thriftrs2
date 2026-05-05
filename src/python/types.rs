// ──────────────────────────────────────────────────────────────────────────────
// types.rs  –  Python-visible Thrift type wrappers
// ──────────────────────────────────────────────────────────────────────────────
use crate::parser::ast::*;
use pyo3::prelude::*;
use pyo3::types::PyDict;
use std::collections::HashMap;
use std::sync::Arc;

use super::serde::{py_any_to_thrift_value, py_any_to_thrift_value_with_type, thrift_value_to_py};

// ──────────────────────────────────────────────────────────────────────────────
// ThriftField
// ──────────────────────────────────────────────────────────────────────────────

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
// ThriftStruct
// ──────────────────────────────────────────────────────────────────────────────

#[pyclass(skip_from_py_object)]
#[derive(Clone)]
pub struct ThriftStruct {
    #[pyo3(get)]
    pub name: String,
    #[pyo3(get)]
    pub fields: Vec<ThriftField>,
    /// Cached map from field id -> index in `fields`, built once at parse time.
    pub(crate) field_map: HashMap<i16, usize>,
    /// Shared map of all structs in the parsed document — used by new_instance
    /// to give the created ThriftStructInstance schema-aware setattr coercion.
    pub(crate) struct_map: Arc<HashMap<String, ThriftStruct>>,
    /// Pre-built schema map (field name -> ThriftField), shared cheaply via Arc.
    pub(crate) schema_arc: Arc<HashMap<String, ThriftField>>,
    /// Pre-built ordered field name list, shared cheaply via Arc.
    pub(crate) field_names_arc: Arc<Vec<String>>,
}

#[pymethods]
impl ThriftStruct {
    /// Construct an empty ThriftStructInstance with all fields set to None.
    pub fn new_instance(&self, _py: Python<'_>) -> ThriftStructInstance {
        ThriftStructInstance::empty(
            self.name.clone(),
            Arc::clone(&self.field_names_arc),
            Arc::clone(&self.schema_arc),
            Arc::clone(&self.struct_map),
        )
    }

    pub fn new_instance_from_dict(
        &self,
        py: Python<'_>,
        items: &Bound<'_, PyDict>,
    ) -> ThriftStructInstance {
        let mut instance = ThriftStructInstance::empty(
            self.name.clone(),
            Arc::clone(&self.field_names_arc),
            Arc::clone(&self.schema_arc),
            Arc::clone(&self.struct_map),
        );
        for (k, v) in items.iter() {
            if let Ok(name) = k.extract::<String>() {
                if instance.is_valid_field(&name) {
                    instance.cache.insert(name.clone(), v.clone().unbind());
                    let tv_result = if let Some(field) = self.schema_arc.get(&name) {
                        py_any_to_thrift_value_with_type(
                            &v,
                            &field.field_type.clone(),
                            &self.struct_map,
                        )
                    } else {
                        py_any_to_thrift_value(&v)
                    };
                    if let Ok(tv) = tv_result {
                        instance.values.insert(name, tv);
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
        if let Some(kw) = kwargs {
            self.new_instance_from_dict(py, kw)
        } else {
            ThriftStructInstance::empty(
                self.name.clone(),
                Arc::clone(&self.field_names_arc),
                Arc::clone(&self.schema_arc),
                Arc::clone(&self.struct_map),
            )
        }
    }

    /// Serialize a struct from either a `ThriftStructInstance` or a plain `dict` using specific protocol.
    pub fn serialize_with_protocol(
        &self,
        py: Python<'_>,
        data: &Bound<'_, PyAny>,
        protocol: crate::python::parser::ProtocolType,
    ) -> PyResult<Vec<u8>> {
        use super::serde::serialize_struct_any;
        use crate::protocol::{
            BinaryProtocolWriter, CompactProtocolWriter, JSONProtocolWriter, TOutputProtocol,
        };
        let mut buffer = Vec::with_capacity(128 + self.fields.len() * 16);
        match protocol {
            crate::python::parser::ProtocolType::Binary => {
                let mut writer = BinaryProtocolWriter::new(&mut buffer);
                writer.write_struct_begin(&self.name).map_err(|e| {
                    PyErr::new::<pyo3::exceptions::PyIOError, _>(format!("Write error: {}", e))
                })?;
                serialize_struct_any(&mut writer, &self.fields, data, &self.struct_map, py)?;
                writer.write_field_stop().map_err(|e| {
                    PyErr::new::<pyo3::exceptions::PyIOError, _>(format!("Write error: {}", e))
                })?;
                writer.write_struct_end().map_err(|e| {
                    PyErr::new::<pyo3::exceptions::PyIOError, _>(format!("Write error: {}", e))
                })?;
            }
            crate::python::parser::ProtocolType::Compact => {
                let mut writer = CompactProtocolWriter::new(&mut buffer);
                writer.write_struct_begin(&self.name).map_err(|e| {
                    PyErr::new::<pyo3::exceptions::PyIOError, _>(format!("Write error: {}", e))
                })?;
                serialize_struct_any(&mut writer, &self.fields, data, &self.struct_map, py)?;
                writer.write_field_stop().map_err(|e| {
                    PyErr::new::<pyo3::exceptions::PyIOError, _>(format!("Write error: {}", e))
                })?;
                writer.write_struct_end().map_err(|e| {
                    PyErr::new::<pyo3::exceptions::PyIOError, _>(format!("Write error: {}", e))
                })?;
            }
            crate::python::parser::ProtocolType::JSON => {
                let mut writer = JSONProtocolWriter::new(&mut buffer);
                writer.write_struct_begin(&self.name).map_err(|e| {
                    PyErr::new::<pyo3::exceptions::PyIOError, _>(format!("Write error: {}", e))
                })?;
                serialize_struct_any(&mut writer, &self.fields, data, &self.struct_map, py)?;
                writer.write_field_stop().map_err(|e| {
                    PyErr::new::<pyo3::exceptions::PyIOError, _>(format!("Write error: {}", e))
                })?;
                writer.write_struct_end().map_err(|e| {
                    PyErr::new::<pyo3::exceptions::PyIOError, _>(format!("Write error: {}", e))
                })?;
            }
        }
        Ok(buffer)
    }

    /// Serialize a struct from either a `ThriftStructInstance` or a plain `dict`.
    pub fn serialize(&self, py: Python<'_>, data: &Bound<'_, PyAny>) -> PyResult<Vec<u8>> {
        self.serialize_with_protocol(py, data, crate::python::parser::ProtocolType::Binary)
    }

    pub fn deserialize_with_protocol<'py>(
        &self,
        py: Python<'py>,
        data: &[u8],
        protocol: crate::python::parser::ProtocolType,
    ) -> PyResult<Bound<'py, ThriftStructInstance>> {
        use super::serde::deserialize_struct_fields_as_instance;
        use crate::protocol::{
            BinaryProtocolReader, CompactProtocolReader, JSONProtocolReader, TInputProtocol,
        };
        use std::io::Cursor;
        let mut cursor = Cursor::new(data);

        match protocol {
            crate::python::parser::ProtocolType::Binary => {
                let mut reader = BinaryProtocolReader::new(&mut cursor);
                reader.read_struct_begin().map_err(|e| {
                    PyErr::new::<pyo3::exceptions::PyIOError, _>(format!("Read error: {}", e))
                })?;
                let instance = deserialize_struct_fields_as_instance(&mut reader, self, py)?;
                reader.read_struct_end().map_err(|e| {
                    PyErr::new::<pyo3::exceptions::PyIOError, _>(format!("Read error: {}", e))
                })?;
                Ok(instance)
            }
            crate::python::parser::ProtocolType::Compact => {
                let mut reader = CompactProtocolReader::new(&mut cursor);
                reader.read_struct_begin().map_err(|e| {
                    PyErr::new::<pyo3::exceptions::PyIOError, _>(format!("Read error: {}", e))
                })?;
                let instance = deserialize_struct_fields_as_instance(&mut reader, self, py)?;
                reader.read_struct_end().map_err(|e| {
                    PyErr::new::<pyo3::exceptions::PyIOError, _>(format!("Read error: {}", e))
                })?;
                Ok(instance)
            }
            crate::python::parser::ProtocolType::JSON => {
                let mut reader = JSONProtocolReader::new(&mut cursor);
                reader.read_struct_begin().map_err(|e| {
                    PyErr::new::<pyo3::exceptions::PyIOError, _>(format!("Read error: {}", e))
                })?;
                let instance = deserialize_struct_fields_as_instance(&mut reader, self, py)?;
                reader.read_struct_end().map_err(|e| {
                    PyErr::new::<pyo3::exceptions::PyIOError, _>(format!("Read error: {}", e))
                })?;
                Ok(instance)
            }
        }
    }

    /// Deserialize bytes into a `ThriftStructInstance`.
    pub fn deserialize<'py>(
        &self,
        py: Python<'py>,
        data: &[u8],
    ) -> PyResult<Bound<'py, ThriftStructInstance>> {
        self.deserialize_with_protocol(py, data, crate::python::parser::ProtocolType::Binary)
    }

    pub fn __repr__(&self) -> String {
        format!(
            "ThriftStruct(name={:?}, fields={:?})",
            self.name, self.fields
        )
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// ThriftStructInstance  –  Python-visible Thrift struct instance.
// ──────────────────────────────────────────────────────────────────────────────

/// A live instance of a Thrift struct.
///
/// Fields are accessible as Python attributes (``instance.field_name``).
/// Unknown attribute names raise ``AttributeError``.
#[pyclass(skip_from_py_object)]
pub struct ThriftStructInstance {
    #[pyo3(get)]
    pub struct_name: String,
    pub field_names: Arc<Vec<String>>,
    /// Thrift-value store, populated at deserialisation time or via __setattr__.
    pub values: HashMap<String, ThriftValue>,
    /// Lazy Python-object cache.
    pub cache: HashMap<String, Py<PyAny>>,
    /// Schema: field name → ThriftField, used by __setattr__ for type-aware coercion.
    pub schema: Arc<HashMap<String, ThriftField>>,
    /// Shared struct map for resolving nested struct types in schema-aware coercion.
    pub struct_map: Arc<HashMap<String, ThriftStruct>>,
}

impl Clone for ThriftStructInstance {
    fn clone(&self) -> Self {
        Python::attach(|py| Self {
            struct_name: self.struct_name.clone(),
            field_names: Arc::clone(&self.field_names),
            values: self.values.clone(),
            cache: self
                .cache
                .iter()
                .map(|(k, v)| (k.clone(), v.clone_ref(py)))
                .collect(),
            schema: Arc::clone(&self.schema),
            struct_map: Arc::clone(&self.struct_map),
        })
    }
}

impl ThriftStructInstance {
    /// Returns `true` if `name` is a declared field of this struct.
    #[inline]
    fn is_valid_field(&self, name: &str) -> bool {
        if !self.schema.is_empty() {
            self.schema.contains_key(name)
        } else {
            self.field_names.iter().any(|n| n == name)
        }
    }

    pub fn from_rust(
        struct_name: String,
        field_names: Arc<Vec<String>>,
        values: HashMap<String, ThriftValue>,
        schema: Arc<HashMap<String, ThriftField>>,
        struct_map: Arc<HashMap<String, ThriftStruct>>,
    ) -> Self {
        Self {
            struct_name,
            field_names,
            values,
            cache: HashMap::new(),
            schema,
            struct_map,
        }
    }

    pub fn empty(
        struct_name: String,
        field_names: Arc<Vec<String>>,
        schema: Arc<HashMap<String, ThriftField>>,
        struct_map: Arc<HashMap<String, ThriftStruct>>,
    ) -> Self {
        Self {
            struct_name,
            field_names,
            values: HashMap::new(),
            cache: HashMap::new(),
            schema,
            struct_map,
        }
    }

    pub fn set_field(&mut self, name: &str, value: ThriftValue) {
        self.values.insert(name.to_string(), value);
    }

    pub fn get_field<'py>(&mut self, py: Python<'py>, name: &str) -> Option<Bound<'py, PyAny>> {
        if !self.is_valid_field(name) {
            return None;
        }
        if let Some(v) = self.cache.get(name) {
            return Some(v.bind(py).clone());
        }
        let py_val = match self.values.get(name) {
            Some(tv) => thrift_value_to_py(tv, py, &self.struct_map).unwrap_or_else(|_| py.None()),
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
        Self {
            struct_name,
            field_names: Arc::new(field_names),
            values: HashMap::new(),
            cache,
            schema: Arc::new(HashMap::new()),
            struct_map: Arc::new(HashMap::new()),
        }
    }

    pub fn __getattr__(&mut self, py: Python<'_>, name: &str) -> PyResult<Py<PyAny>> {
        if self.is_valid_field(name) {
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
        if self.is_valid_field(name) {
            self.cache.insert(name.to_string(), value.clone_ref(py));
            let bound = value.bind(py);
            let tv_result = if let Some(field) = self.schema.get(name) {
                py_any_to_thrift_value_with_type(bound, &field.field_type.clone(), &self.struct_map)
            } else {
                py_any_to_thrift_value(bound)
            };
            if let Ok(tv) = tv_result {
                self.values.insert(name.to_string(), tv);
            } else {
                self.values.remove(name);
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

    #[allow(clippy::wrong_self_convention)]
    pub fn to_dict<'py>(&mut self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let d = PyDict::new(py);
        let names = self.field_names.clone();
        for name in names.as_ref() {
            let v = self
                .get_field(py, name)
                .unwrap_or_else(|| py.None().into_bound(py));
            d.set_item(name, v)?;
        }
        Ok(d)
    }

    pub fn field_names(&self) -> Vec<String> {
        self.field_names.as_ref().clone()
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// PyThriftService / PyThriftMethod  (exposed to Python)
// ──────────────────────────────────────────────────────────────────────────────

#[pyclass(from_py_object)]
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

#[pyclass(skip_from_py_object)]
#[derive(Clone)]
pub struct PyThriftMethod {
    #[pyo3(get)]
    pub name: String,
    #[pyo3(get)]
    pub arguments: Vec<ThriftField>,
    #[pyo3(get)]
    pub exceptions: Vec<ThriftField>,
    #[pyo3(get)]
    pub oneway: bool,
    pub(crate) return_type: ThriftType,
    /// Pre-computed field-id → index map for argument deserialisation.
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
// TransportType
// ──────────────────────────────────────────────────────────────────────────────

/// Transport layer to use when reading/writing Thrift messages over TCP.
///
/// * `Framed`   – each message is prefixed by a 4-byte big-endian frame length.
/// * `Buffered` – messages are written directly to the socket with no framing.
#[pyclass(eq, eq_int, from_py_object)]
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
