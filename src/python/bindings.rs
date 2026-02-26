use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList, PyBytes};
use crate::parser::{Parser, ast::*};
use crate::parser::ast::ThriftStruct as ThriftStructSchema;
use crate::protocol::{BinaryProtocolReader, BinaryProtocolWriter, TType, FieldBegin};
use std::collections::HashMap;
use std::io::Cursor;

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
        let mut parser = Parser::new(content)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(format!("Parse error: {}", e)))?;

        self.document = Some(parser.parse_document()
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(format!("Parse error: {}", e)))?);

        Ok(())
    }

    pub fn list_structs(&self) -> PyResult<Vec<String>> {
        match &self.document {
            Some(doc) => Ok(doc.structs.keys().cloned().collect()),
            None => Err(PyErr::new::<pyo3::exceptions::PyValueError, _>("No document parsed yet")),
        }
    }

    pub fn get_struct(&self, name: &str) -> PyResult<Option<ThriftStruct>> {
        match &self.document {
            Some(doc) => {
                Ok(doc.structs.get(name).map(|s| ThriftStruct {
                    name: s.name.clone(),
                    fields: s.fields.iter().map(|f| ThriftField {
                        id: f.id,
                        name: f.name.clone(),
                        required: f.required,
                        field_type: f.field_type.clone(),
                    }).collect(),
                }))
            }
            None => Err(PyErr::new::<pyo3::exceptions::PyValueError, _>("No document parsed yet")),
        }
    }
}

#[pyclass]
#[derive(Clone)]
pub struct ThriftStruct {
    #[pyo3(get)]
    pub name: String,
    #[pyo3(get)]
    pub fields: Vec<ThriftField>,
}

#[pymethods]
impl ThriftStruct {
    pub fn serialize(&self, data: &Bound<'_, PyDict>) -> PyResult<Vec<u8>> {
        let mut buffer = Vec::new();
        let mut writer = BinaryProtocolWriter::new(&mut buffer);

        for field in &self.fields {
            if let Some(value) = data.get_item(&field.name)? {
                let field_begin = FieldBegin {
                    name: Some(field.name.clone()),
                    field_type: thrift_type_to_ttype(&field.field_type),
                    id: field.id,
                };

                writer.write_field_begin(&field_begin)
                    .map_err(|e| PyErr::new::<pyo3::exceptions::PyIOError, _>(format!("Write error: {}", e)))?;

                write_value(&mut writer, &field.field_type, &value)?;
            }
        }

        writer.write_field_stop()
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyIOError, _>(format!("Write error: {}", e)))?;

        Ok(buffer)
    }

    pub fn deserialize(&self, data: &[u8]) -> PyResult<PyObject> {
        let mut cursor = Cursor::new(data);
        let mut reader = BinaryProtocolReader::new(&mut cursor);

        Python::with_gil(|py| {
            let result = PyDict::new(py);
            let field_map: HashMap<i16, &ThriftField> = self.fields.iter()
                .map(|f| (f.id, f))
                .collect();

            loop {
                let field_begin = reader.read_field_begin()
                    .map_err(|e| PyErr::new::<pyo3::exceptions::PyIOError, _>(format!("Read error: {}", e)))?;

                if field_begin.field_type == TType::Stop {
                    break;
                }

                if let Some(field) = field_map.get(&field_begin.id) {
                    let value = read_value(&mut reader, &field.field_type, py)?;
                    result.set_item(&field.name, value)?;
                }
            }

            Ok(result.into())
        })
    }

    pub fn __repr__(&self) -> String {
        format!("ThriftStruct(name={:?}, fields={:?})", self.name, self.fields)
    }
}

#[pyclass]
#[derive(Debug, Clone)]
pub struct ThriftField {
    #[pyo3(get)]
    pub id: i16,
    #[pyo3(get)]
    pub name: String,
    #[pyo3(get)]
    pub required: bool,
    field_type: ThriftType,
}

#[pymethods]
impl ThriftField {
    pub fn __repr__(&self) -> String {
        format!("ThriftField(id={}, name={:?}, required={}, field_type={:?})", self.id, self.name, self.required, self.field_type)
    }
}

#[pyclass]
pub struct BinaryProtocol;

#[pymethods]
impl BinaryProtocol {
    #[new]
    pub fn new() -> Self {
        Self
    }

    #[staticmethod]
    pub fn serialize_struct(struct_def: &ThriftStruct, data: &Bound<'_, PyDict>) -> PyResult<Vec<u8>> {
        struct_def.serialize(data)
    }

    #[staticmethod]
    pub fn deserialize_struct(struct_def: &ThriftStruct, data: &[u8]) -> PyResult<PyObject> {
        struct_def.deserialize(data)
    }
}

fn thrift_type_to_ttype(thrift_type: &ThriftType) -> TType {
    match thrift_type {
        ThriftType::Bool => TType::Bool,
        ThriftType::Byte => TType::Byte,
        ThriftType::I16 => TType::I16,
        ThriftType::I32 => TType::I32,
        ThriftType::I64 => TType::I64,
        ThriftType::Double => TType::Double,
        ThriftType::String => TType::String,
        ThriftType::Binary => TType::String, // Binary is encoded as string
        ThriftType::List(_) => TType::List,
        ThriftType::Set(_) => TType::Set,
        ThriftType::Map(_, _) => TType::Map,
        ThriftType::Struct(_) => TType::Struct,
    }
}

fn write_value<'py, W: std::io::Write>(
    writer: &mut BinaryProtocolWriter<W>,
    thrift_type: &ThriftType,
    value: &Bound<'py, PyAny>
) -> PyResult<()> {
    match thrift_type {
        ThriftType::Bool => {
            let val: bool = value.extract()?;
            writer.write_bool(val)
                .map_err(|e| PyErr::new::<pyo3::exceptions::PyIOError, _>(format!("Write error: {}", e)))?;
        }
        ThriftType::Byte => {
            let val: i8 = value.extract()?;
            writer.write_byte(val)
                .map_err(|e| PyErr::new::<pyo3::exceptions::PyIOError, _>(format!("Write error: {}", e)))?;
        }
        ThriftType::I16 => {
            let val: i16 = value.extract()?;
            writer.write_i16(val)
                .map_err(|e| PyErr::new::<pyo3::exceptions::PyIOError, _>(format!("Write error: {}", e)))?;
        }
        ThriftType::I32 => {
            let val: i32 = value.extract()?;
            writer.write_i32(val)
                .map_err(|e| PyErr::new::<pyo3::exceptions::PyIOError, _>(format!("Write error: {}", e)))?;
        }
        ThriftType::I64 => {
            let val: i64 = value.extract()?;
            writer.write_i64(val)
                .map_err(|e| PyErr::new::<pyo3::exceptions::PyIOError, _>(format!("Write error: {}", e)))?;
        }
        ThriftType::Double => {
            let val: f64 = value.extract()?;
            writer.write_double(val)
                .map_err(|e| PyErr::new::<pyo3::exceptions::PyIOError, _>(format!("Write error: {}", e)))?;
        }
        ThriftType::String => {
            let val: String = value.extract()?;
            writer.write_string(&val)
                .map_err(|e| PyErr::new::<pyo3::exceptions::PyIOError, _>(format!("Write error: {}", e)))?;
        }
        ThriftType::Binary => {
            let val: Vec<u8> = value.extract()?;
            writer.write_binary(&val)
                .map_err(|e| PyErr::new::<pyo3::exceptions::PyIOError, _>(format!("Write error: {}", e)))?;
        }
        _ => {
            return Err(PyErr::new::<pyo3::exceptions::PyNotImplementedError, _>(
                format!("Type {:?} not yet implemented", thrift_type)
            ));
        }
    }
    Ok(())
}

fn read_value<R: std::io::Read>(
    reader: &mut BinaryProtocolReader<R>,
    thrift_type: &ThriftType,
    py: Python,
) -> PyResult<PyObject> {
    match thrift_type {
        ThriftType::Bool => {
            let val = reader.read_bool()
                .map_err(|e| PyErr::new::<pyo3::exceptions::PyIOError, _>(format!("Read error: {}", e)))?;
            Ok(val.into_py(py))
        }
        ThriftType::Byte => {
            let val = reader.read_byte()
                .map_err(|e| PyErr::new::<pyo3::exceptions::PyIOError, _>(format!("Read error: {}", e)))?;
            Ok(val.into_py(py))
        }
        ThriftType::I16 => {
            let val = reader.read_i16()
                .map_err(|e| PyErr::new::<pyo3::exceptions::PyIOError, _>(format!("Read error: {}", e)))?;
            Ok(val.into_py(py))
        }
        ThriftType::I32 => {
            let val = reader.read_i32()
                .map_err(|e| PyErr::new::<pyo3::exceptions::PyIOError, _>(format!("Read error: {}", e)))?;
            Ok(val.into_py(py))
        }
        ThriftType::I64 => {
            let val = reader.read_i64()
                .map_err(|e| PyErr::new::<pyo3::exceptions::PyIOError, _>(format!("Read error: {}", e)))?;
            Ok(val.into_py(py))
        }
        ThriftType::Double => {
            let val = reader.read_double()
                .map_err(|e| PyErr::new::<pyo3::exceptions::PyIOError, _>(format!("Read error: {}", e)))?;
            Ok(val.into_py(py))
        }
        ThriftType::String => {
            let val = reader.read_string()
                .map_err(|e| PyErr::new::<pyo3::exceptions::PyIOError, _>(format!("Read error: {}", e)))?;
            Ok(val.into_py(py))
        }
        ThriftType::Binary => {
            let val = reader.read_binary()
                .map_err(|e| PyErr::new::<pyo3::exceptions::PyIOError, _>(format!("Read error: {}", e)))?;
            Ok(PyBytes::new(py, &val).into())
        }
        _ => {
            Err(PyErr::new::<pyo3::exceptions::PyNotImplementedError, _>(
                format!("Type {:?} not yet implemented", thrift_type)
            ))
        }
    }
}
