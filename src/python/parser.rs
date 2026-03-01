// ──────────────────────────────────────────────────────────────────────────────
// parser.rs  –  ThriftParser and BinaryProtocol Python bindings
// ──────────────────────────────────────────────────────────────────────────────
use crate::parser::{ast::*, Parser};
use pyo3::prelude::*;
use std::collections::HashMap;
use std::sync::Arc;

use super::types::{PyThriftMethod, PyThriftService, ThriftField, ThriftStruct};

#[pyclass]
pub struct ThriftParser {
    pub(crate) document: Option<ThriftDocument>,
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
    /// Return a snapshot of the whole parsed document's struct map.
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
                            struct_map: Arc::new(HashMap::new()),
                        },
                    )
                })
                .collect(),
            None => HashMap::new(),
        };
        let arc = Arc::new(map);
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


