// ──────────────────────────────────────────────────────────────────────────────
// types.rs  –  Python-visible Thrift type wrappers
// ──────────────────────────────────────────────────────────────────────────────
use crate::parser::ast::*;
use pyo3::prelude::*;
use pyo3::types::PyDict;
use std::collections::HashMap;
use std::sync::Arc;

use super::serde::{
    py_any_to_thrift_value, py_any_to_thrift_value_with_type, thrift_value_to_py,
};

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
}

#[pymethods]
impl ThriftStruct {
    /// Construct an empty ThriftStructInstance with all fields set to None.
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
                    instance.cache.insert(name.clone(), v.clone().unbind());
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

    /// Serialize a struct from either a `ThriftStructInstance` or a plain `dict` using specific protocol.
    pub fn serialize_with_protocol(
        &self,
        py: Python<'_>,
        data: &Bound<'_, PyAny>,
        protocol: crate::python::parser::ProtocolType,
    ) -> PyResult<Vec<u8>> {
        use crate::protocol::{
            BinaryProtocolWriter, CompactProtocolWriter, JSONProtocolWriter, TOutputProtocol,
        };
        use super::serde::serialize_struct_any;
        let mut buffer = Vec::with_capacity(128 + self.fields.len() * 16);
        match protocol {
            crate::python::parser::ProtocolType::Binary => {
                let mut writer = BinaryProtocolWriter::new(&mut buffer);
                serialize_struct_any(&mut writer, &self.fields, data, &self.struct_map, py)?;
                writer.write_field_stop().map_err(|e| {
                    PyErr::new::<pyo3::exceptions::PyIOError, _>(format!("Write error: {}", e))
                })?;
            }
            crate::python::parser::ProtocolType::Compact => {
                let mut writer = CompactProtocolWriter::new(&mut buffer);
                serialize_struct_any(&mut writer, &self.fields, data, &self.struct_map, py)?;
                writer.write_field_stop().map_err(|e| {
                    PyErr::new::<pyo3::exceptions::PyIOError, _>(format!("Write error: {}", e))
                })?;
            }
            crate::python::parser::ProtocolType::JSON => {
                let mut writer = JSONProtocolWriter::new(&mut buffer);
                serialize_struct_any(&mut writer, &self.fields, data, &self.struct_map, py)?;
                writer.write_field_stop().map_err(|e| {
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
        use std::io::Cursor;
        use crate::protocol::{
            BinaryProtocolReader, CompactProtocolReader, JSONProtocolReader, TInputProtocol,
        };
        use super::serde::deserialize_struct_fields_as_instance;
        let mut cursor = Cursor::new(data);
        let schema: HashMap<String, ThriftField> = self
            .fields
            .iter()
            .map(|f| (f.name.clone(), f.clone()))
            .collect();

        match protocol {
            crate::python::parser::ProtocolType::Binary => {
                let mut reader = BinaryProtocolReader::new(&mut cursor);
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
            crate::python::parser::ProtocolType::Compact => {
                let mut reader = CompactProtocolReader::new(&mut cursor);
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
            crate::python::parser::ProtocolType::JSON => {
                let mut reader = JSONProtocolReader::new(&mut cursor);
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
// RustStructValue  –  GIL-free pure-Rust representation of a struct instance.
// ──────────────────────────────────────────────────────────────────────────────

/// Pure-Rust struct value produced during deserialisation without holding the GIL.
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
// ──────────────────────────────────────────────────────────────────────────────

/// A live instance of a Thrift struct.
///
/// Fields are accessible as Python attributes (``instance.field_name``).
/// Unknown attribute names raise ``AttributeError``.
#[pyclass(skip_from_py_object)]
pub struct ThriftStructInstance {
    #[pyo3(get)]
    pub struct_name: String,
    pub field_names: Vec<String>,
    /// Pure-Rust inner store.  Populated at deserialisation time without GIL.
    pub inner: RustStructValue,
    /// Lazy Python-object cache.
    pub cache: HashMap<String, Py<PyAny>>,
    /// Schema: field name → ThriftField, used by __setattr__ for type-aware coercion.
    pub schema: HashMap<String, ThriftField>,
    /// Shared struct map for resolving nested struct types in schema-aware coercion.
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

    pub fn set_field(&mut self, name: &str, value: ThriftValue) {
        self.inner.set_field(name, value);
    }

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
            Err(PyErr::new::<pyo3::exceptions::PyAttributeError, _>(format!(
                "'{}' object has no attribute '{}'",
                self.struct_name, name
            )))
        }
    }

    pub fn __setattr__(&mut self, py: Python<'_>, name: &str, value: Py<PyAny>) -> PyResult<()> {
        if self.field_names.contains(&name.to_string()) {
            self.cache.insert(name.to_string(), value.clone_ref(py));
            let bound = value.bind(py);
            let tv_result = if let Some(field) = self.schema.get(name) {
                py_any_to_thrift_value_with_type(bound, &field.field_type.clone(), &self.struct_map)
            } else {
                py_any_to_thrift_value(bound)
            };
            if let Ok(tv) = tv_result {
                self.inner.values.insert(name.to_string(), tv);
            } else {
                self.inner.values.remove(name);
            }
            Ok(())
        } else {
            Err(PyErr::new::<pyo3::exceptions::PyAttributeError, _>(format!(
                "'{}' object has no attribute '{}'",
                self.struct_name, name
            )))
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

