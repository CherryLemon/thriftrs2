// ──────────────────────────────────────────────────────────────────────────────
// serde.rs  –  Serialization / Deserialization helpers
// ──────────────────────────────────────────────────────────────────────────────
use crate::parser::ast::*;
use crate::protocol::{
    FieldBegin, ListBegin, MapBegin, SetBegin, TInputProtocol, TOutputProtocol, TType,
};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use pyo3::prelude::*;
use pyo3::types::{PyBytes, PyDict, PyList};
use serde_json::Value as JsonValue;
use std::collections::HashMap;
use std::sync::Arc;

use super::types::{ThriftField, ThriftStruct, ThriftStructInstance};

// ──────────────────────────────────────────────────────────────────────────────
// RustStructValue  –  GIL-free pure-Rust representation used during deserialization.
// ──────────────────────────────────────────────────────────────────────────────

/// Pure-Rust struct value produced during deserialisation without holding the GIL.
#[derive(Clone)]
pub(crate) struct RustStructValue {
    pub struct_name: String,
    pub field_names: Vec<String>,
    pub values: HashMap<String, ThriftValue>,
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

/// Derive the `TType` wire tag from a `ThriftValue` without schema info.
#[inline]
pub(crate) fn thrift_value_ttype(val: &ThriftValue) -> TType {
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
// Serialisation helpers (Python → wire bytes)
// ──────────────────────────────────────────────────────────────────────────────

/// serialize struct fields from either a `ThriftStructInstance` or a plain `PyDict`.
pub(crate) fn serialize_struct_any<P: TOutputProtocol>(
    writer: &mut P,
    fields: &[ThriftField],
    data: &Bound<'_, PyAny>,
    struct_map: &HashMap<String, ThriftStruct>,
    py: Python<'_>,
) -> PyResult<()> {
    if let Ok(instance) = data.cast::<ThriftStructInstance>() {
        let instance = instance.borrow();
        for field in fields {
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
                    writer.write_field_end().map_err(|e| {
                        PyErr::new::<pyo3::exceptions::PyIOError, _>(format!("Write error: {}", e))
                    })?;
                }
            } else if let Some(tv) = instance.values.get(&field.name) {
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
                writer.write_field_end().map_err(|e| {
                    PyErr::new::<pyo3::exceptions::PyIOError, _>(format!("Write error: {}", e))
                })?;
            }
        }
        Ok(())
    } else {
        let dict = data.cast::<PyDict>()?;
        serialize_struct_fields(writer, fields, dict, struct_map)
    }
}

pub(crate) fn serialize_struct_fields<P: TOutputProtocol>(
    writer: &mut P,
    fields: &[ThriftField],
    data: &Bound<'_, PyDict>,
    struct_map: &HashMap<String, ThriftStruct>,
) -> PyResult<()> {
    for field in fields {
        let field_begin = FieldBegin {
            name: None,
            field_type: thrift_type_to_ttype(&field.field_type),
            id: field.id,
        };
        if let Some(value) = data.get_item(&field.name)? {
            if value.is_none() {
                if field.required {
                    return Err(PyErr::new::<pyo3::exceptions::PyTypeError, _>(format!(
                        "Required field '{}' cannot be None",
                        field.name
                    )));
                }
                continue;
            }
            writer.write_field_begin(&field_begin).map_err(|e| {
                PyErr::new::<pyo3::exceptions::PyIOError, _>(format!("Write error: {}", e))
            })?;
            write_value_with_structs(writer, &field.field_type, &value, struct_map)?;
            writer.write_field_end().map_err(|e| {
                PyErr::new::<pyo3::exceptions::PyIOError, _>(format!("Write error: {}", e))
            })?;
        } else if let Some(default_value) = &field.default_value {
            writer.write_field_begin(&field_begin).map_err(|e| {
                PyErr::new::<pyo3::exceptions::PyIOError, _>(format!("Write error: {}", e))
            })?;
            write_thrift_value(writer, default_value, struct_map).map_err(|e| {
                PyErr::new::<pyo3::exceptions::PyIOError, _>(format!("Write error: {}", e))
            })?;
            writer.write_field_end().map_err(|e| {
                PyErr::new::<pyo3::exceptions::PyIOError, _>(format!("Write error: {}", e))
            })?;
        }
    }
    Ok(())
}

pub(crate) fn write_value_with_structs<'py, P: TOutputProtocol>(
    writer: &mut P,
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
            if matches!(thrift_type, ThriftType::List(_)) {
                writer
                    .write_list_end()
                    .map_err(|e| PyErr::new::<pyo3::exceptions::PyIOError, _>(e.to_string()))?;
            } else {
                writer
                    .write_set_end()
                    .map_err(|e| PyErr::new::<pyo3::exceptions::PyIOError, _>(e.to_string()))?;
            }
        }
        ThriftType::Map(key_type, val_type) => {
            let dict = value.cast::<PyDict>()?;
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
            writer
                .write_map_end()
                .map_err(|e| PyErr::new::<pyo3::exceptions::PyIOError, _>(e.to_string()))?;
        }
        ThriftType::Struct(name) => {
            if let Some(target_struct) = struct_map.get(name) {
                writer
                    .write_struct_begin(name)
                    .map_err(|e| PyErr::new::<pyo3::exceptions::PyIOError, _>(e.to_string()))?;
                serialize_struct_any(writer, &target_struct.fields, value, struct_map, value.py())?;
                writer
                    .write_field_stop()
                    .map_err(|e| PyErr::new::<pyo3::exceptions::PyIOError, _>(e.to_string()))?;
                writer
                    .write_struct_end()
                    .map_err(|e| PyErr::new::<pyo3::exceptions::PyIOError, _>(e.to_string()))?;
            } else {
                return Err(PyErr::new::<pyo3::exceptions::PyValueError, _>(format!(
                    "Unknown struct in schema: {}",
                    name
                )));
            }
        }
    }
    Ok(())
}

/// serialize a `ThriftValue` directly to the wire without touching the GIL.
pub(crate) fn write_thrift_value<P: TOutputProtocol>(
    writer: &mut P,
    val: &ThriftValue,
    struct_map: &HashMap<String, ThriftStruct>,
) -> std::io::Result<()> {
    match val {
        ThriftValue::Bool(v) => Ok(writer.write_bool(*v)?),
        ThriftValue::Byte(v) => Ok(writer.write_byte(*v)?),
        ThriftValue::I16(v) => Ok(writer.write_i16(*v)?),
        ThriftValue::I32(v) => Ok(writer.write_i32(*v)?),
        ThriftValue::I64(v) => Ok(writer.write_i64(*v)?),
        ThriftValue::Double(v) => Ok(writer.write_double(*v)?),
        ThriftValue::String(v) => Ok(writer.write_string(v)?),
        ThriftValue::Binary(v) => Ok(writer.write_binary(v)?),
        ThriftValue::List(items) => {
            let elem_ttype = items
                .first()
                .map(thrift_value_ttype)
                .unwrap_or(TType::String);
            writer.write_list_begin(&ListBegin {
                element_type: elem_ttype,
                size: items.len() as i32,
            })?;
            for item in items {
                write_thrift_value(writer, item, struct_map)?;
            }
            writer.write_list_end()?;
            Ok(())
        }
        ThriftValue::Set(items) => {
            let elem_ttype = items
                .first()
                .map(thrift_value_ttype)
                .unwrap_or(TType::String);
            writer.write_set_begin(&SetBegin {
                element_type: elem_ttype,
                size: items.len() as i32,
            })?;
            for item in items {
                write_thrift_value(writer, item, struct_map)?;
            }
            writer.write_set_end()?;
            Ok(())
        }
        ThriftValue::Map(pairs) => {
            let (kt, vt) = pairs
                .first()
                .map(|(k, v)| (thrift_value_ttype(k), thrift_value_ttype(v)))
                .unwrap_or((TType::String, TType::String));
            writer.write_map_begin(&MapBegin {
                key_type: kt,
                value_type: vt,
                size: pairs.len() as i32,
            })?;
            for (k, v) in pairs {
                write_thrift_value(writer, k, struct_map)?;
                write_thrift_value(writer, v, struct_map)?;
            }
            writer.write_map_end()?;
            Ok(())
        }
        ThriftValue::Struct { name, fields } => {
            writer.write_struct_begin(name.as_deref().unwrap_or(""))?;

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

            let struct_def = name.as_deref().and_then(|n| struct_map.get(n));

            for (fname, fval) in &ordered_fields {
                let field_id: i16 = struct_def
                    .and_then(|def| def.fields.iter().find(|f| &f.name == *fname))
                    .map(|f| f.id)
                    .unwrap_or(0);
                let ttype = thrift_value_ttype(fval);
                let field_begin = FieldBegin {
                    name: None,
                    field_type: ttype,
                    id: field_id,
                };
                writer.write_field_begin(&field_begin)?;
                write_thrift_value(writer, fval, struct_map)?;
                writer.write_field_end()?;
            }
            writer.write_field_stop()?;
            writer.write_struct_end()?;
            Ok(())
        }
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// Deserialisation helpers (wire bytes → Python / Rust)
// ──────────────────────────────────────────────────────────────────────────────

#[allow(dead_code)]
pub(crate) fn deserialize_struct_fields<'py, P: TInputProtocol>(
    reader: &mut P,
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
        reader.read_field_end().map_err(|e| {
            PyErr::new::<pyo3::exceptions::PyIOError, _>(format!("Read error: {}", e))
        })?;
    }
    Ok(result)
}

/// Deserialize struct fields into a `ThriftStructInstance`.
pub(crate) fn deserialize_struct_fields_as_instance<'py, P: TInputProtocol>(
    reader: &mut P,
    struct_def: &ThriftStruct,
    py: Python<'py>,
) -> PyResult<Bound<'py, ThriftStructInstance>> {
    let mut rust_val = deserialize_rust_struct(
        reader,
        &struct_def.fields,
        &struct_def.field_map,
        &struct_def.struct_map,
    )
    .map_err(|e| PyErr::new::<pyo3::exceptions::PyIOError, _>(format!("Read error: {}", e)))?;
    rust_val.struct_name = struct_def.name.clone();
    let instance = ThriftStructInstance::from_rust(
        rust_val.struct_name,
        Arc::new(rust_val.field_names),
        rust_val.values,
        Arc::clone(&struct_def.schema_arc),
        Arc::clone(&struct_def.struct_map),
    );
    Bound::new(py, instance)
}

/// Read a single Thrift value from the wire into a `ThriftValue`, entirely
/// without touching the GIL.
pub(crate) fn read_rust_value<P: TInputProtocol>(
    reader: &mut P,
    thrift_type: &ThriftType,
    struct_map: &HashMap<String, ThriftStruct>,
) -> std::io::Result<ThriftValue> {
    match thrift_type {
        ThriftType::Bool => Ok(ThriftValue::Bool(reader.read_bool().map_err(|e| {
            std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string())
        })?)),
        ThriftType::Byte => Ok(ThriftValue::Byte(reader.read_byte().map_err(|e| {
            std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string())
        })?)),
        ThriftType::I16 => Ok(ThriftValue::I16(reader.read_i16().map_err(|e| {
            std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string())
        })?)),
        ThriftType::I32 => Ok(ThriftValue::I32(reader.read_i32().map_err(|e| {
            std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string())
        })?)),
        ThriftType::I64 => Ok(ThriftValue::I64(reader.read_i64().map_err(|e| {
            std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string())
        })?)),
        ThriftType::Double => Ok(ThriftValue::Double(reader.read_double().map_err(|e| {
            std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string())
        })?)),
        ThriftType::String => Ok(ThriftValue::String(reader.read_string().map_err(|e| {
            std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string())
        })?)),
        ThriftType::Binary => Ok(ThriftValue::Binary(reader.read_binary().map_err(|e| {
            std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string())
        })?)),
        ThriftType::List(elem_type) => {
            let lb = reader
                .read_list_begin()
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?;
            let mut items = Vec::with_capacity(lb.size as usize);
            for _ in 0..lb.size {
                items.push(read_rust_value(reader, elem_type, struct_map)?);
            }
            reader
                .read_list_end()
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?;
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
            reader
                .read_set_end()
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?;
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
            reader
                .read_map_end()
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?;
            Ok(ThriftValue::Map(pairs))
        }
        ThriftType::Struct(struct_name) => {
            let struct_def = struct_map.get(struct_name).ok_or_else(|| {
                std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!("Unknown struct type: {}", struct_name),
                )
            })?;
            reader
                .read_struct_begin()
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?;
            let nested = deserialize_rust_struct(
                reader,
                &struct_def.fields,
                &struct_def.field_map,
                struct_map,
            )?;
            reader
                .read_struct_end()
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?;
            Ok(ThriftValue::Struct {
                name: Some(struct_name.clone()),
                fields: nested.values,
            })
        }
    }
}

/// Deserialize Thrift struct fields from the wire into a `RustStructValue`,
/// entirely without touching the GIL.
pub(crate) fn deserialize_rust_struct<P: TInputProtocol>(
    reader: &mut P,
    fields: &[ThriftField],
    field_map: &HashMap<i16, usize>,
    struct_map: &HashMap<String, ThriftStruct>,
) -> std::io::Result<RustStructValue> {
    let field_names: Vec<String> = fields.iter().map(|f| f.name.clone()).collect();
    let mut result = RustStructValue {
        struct_name: String::new(),
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
        reader
            .read_field_end()
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?;
    }
    Ok(result)
}

pub(crate) fn read_value_with_structs<'py, P: TInputProtocol>(
    reader: &mut P,
    thrift_type: &ThriftType,
    struct_map: &HashMap<String, ThriftStruct>,
    py: Python<'py>,
) -> PyResult<Py<PyAny>> {
    match thrift_type {
        ThriftType::Bool => {
            let val = reader
                .read_bool()
                .map_err(|e| PyErr::new::<pyo3::exceptions::PyIOError, _>(e.to_string()))?;
            Ok(val.into_pyobject(py)?.to_owned().into_any().unbind())
        }
        ThriftType::Byte => {
            let val = reader
                .read_byte()
                .map_err(|e| PyErr::new::<pyo3::exceptions::PyIOError, _>(e.to_string()))?;
            Ok(val.into_pyobject(py)?.into_any().unbind())
        }
        ThriftType::I16 => {
            let val = reader
                .read_i16()
                .map_err(|e| PyErr::new::<pyo3::exceptions::PyIOError, _>(e.to_string()))?;
            Ok(val.into_pyobject(py)?.into_any().unbind())
        }
        ThriftType::I32 => {
            let val = reader
                .read_i32()
                .map_err(|e| PyErr::new::<pyo3::exceptions::PyIOError, _>(e.to_string()))?;
            Ok(val.into_pyobject(py)?.into_any().unbind())
        }
        ThriftType::I64 => {
            let val = reader
                .read_i64()
                .map_err(|e| PyErr::new::<pyo3::exceptions::PyIOError, _>(e.to_string()))?;
            Ok(val.into_pyobject(py)?.into_any().unbind())
        }
        ThriftType::Double => {
            let val = reader
                .read_double()
                .map_err(|e| PyErr::new::<pyo3::exceptions::PyIOError, _>(e.to_string()))?;
            Ok(val.into_pyobject(py)?.into_any().unbind())
        }
        ThriftType::String => {
            let val = reader
                .read_string()
                .map_err(|e| PyErr::new::<pyo3::exceptions::PyIOError, _>(e.to_string()))?;
            Ok(val.into_pyobject(py)?.into_any().unbind())
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
            if matches!(thrift_type, ThriftType::List(_)) {
                reader
                    .read_list_end()
                    .map_err(|e| PyErr::new::<pyo3::exceptions::PyIOError, _>(e.to_string()))?;
            } else {
                reader
                    .read_set_end()
                    .map_err(|e| PyErr::new::<pyo3::exceptions::PyIOError, _>(e.to_string()))?;
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
            reader
                .read_map_end()
                .map_err(|e| PyErr::new::<pyo3::exceptions::PyIOError, _>(e.to_string()))?;
            Ok(dict.into_any().unbind())
        }
        ThriftType::Struct(struct_name) => {
            let struct_def = struct_map.get(struct_name).ok_or_else(|| {
                PyErr::new::<pyo3::exceptions::PyValueError, _>(format!(
                    "Unknown struct type: {}",
                    struct_name
                ))
            })?;
            reader
                .read_struct_begin()
                .map_err(|e| PyErr::new::<pyo3::exceptions::PyIOError, _>(e.to_string()))?;
            let instance = deserialize_struct_fields_as_instance(reader, struct_def, py)?;
            reader
                .read_struct_end()
                .map_err(|e| PyErr::new::<pyo3::exceptions::PyIOError, _>(e.to_string()))?;
            Ok(instance.into_any().unbind())
        }
    }
}

#[allow(dead_code)]
/// Decode a struct from the wire directly into a `PyDict` (field_name → Python value).
/// This is allocation-free on the Rust side (no ThriftStructInstance, no HashSet, no schema map).
pub(crate) fn read_struct_as_dict<'py, P: TInputProtocol>(
    reader: &mut P,
    struct_def: &ThriftStruct,
    struct_map: &HashMap<String, ThriftStruct>,
    py: Python<'py>,
) -> PyResult<Bound<'py, PyDict>> {
    let dict = PyDict::new(py);
    loop {
        let field_begin = reader
            .read_field_begin()
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyIOError, _>(e.to_string()))?;
        if field_begin.field_type == TType::Stop {
            break;
        }
        if let Some(&idx) = struct_def.field_map.get(&field_begin.id) {
            let field = &struct_def.fields[idx];
            let val = read_value_with_structs(reader, &field.field_type, struct_map, py)?;
            dict.set_item(&field.name, val.bind(py))?;
        } else {
            skip_value(reader, field_begin.field_type)
                .map_err(|e| PyErr::new::<pyo3::exceptions::PyIOError, _>(e.to_string()))?;
        }
        reader
            .read_field_end()
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyIOError, _>(e.to_string()))?;
    }
    Ok(dict)
}

/// Skip over a value of the given wire type without allocating Python objects.
pub(crate) fn skip_value<P: TInputProtocol>(reader: &mut P, ttype: TType) -> std::io::Result<()> {
    match ttype {
        TType::Bool => {
            reader.read_bool()?;
        }
        TType::Byte => {
            reader.read_byte()?;
        }
        TType::I16 => {
            reader.read_i16()?;
        }
        TType::I32 => {
            reader.read_i32()?;
        }
        TType::I64 => {
            reader.read_i64()?;
        }
        TType::Double => {
            reader.read_double()?;
        }
        TType::String => {
            reader.read_string()?;
        }
        TType::Struct => {
            reader.read_struct_begin()?;
            loop {
                let field = reader.read_field_begin()?;
                if field.field_type == TType::Stop {
                    break;
                }
                skip_value(reader, field.field_type)?;
                reader.read_field_end()?;
            }
            reader.read_struct_end()?;
        }
        TType::Map => {
            let map = reader.read_map_begin()?;
            for _ in 0..map.size {
                skip_value(reader, map.key_type)?;
                skip_value(reader, map.value_type)?;
            }
            reader.read_map_end()?;
        }
        TType::List | TType::Set => {
            if ttype == TType::List {
                let list = reader.read_list_begin()?;
                for _ in 0..list.size {
                    skip_value(reader, list.element_type)?;
                }
                reader.read_list_end()?;
            } else {
                let set = reader.read_set_begin()?;
                for _ in 0..set.size {
                    skip_value(reader, set.element_type)?;
                }
                reader.read_set_end()?;
            }
        }
        _ => {}
    }
    Ok(())
}

// ──────────────────────────────────────────────────────────────────────────────
// ThriftValue ↔ Python conversion
// ──────────────────────────────────────────────────────────────────────────────

/// Convert a `ThriftValue` to a Python object.
///
/// `struct_map` is used to give nested `ThriftStructInstance` objects their
/// schema so that *their* fields are also lazily converted on first access.
pub(crate) fn thrift_value_to_py(
    val: &ThriftValue,
    py: Python<'_>,
    struct_map: &Arc<HashMap<String, ThriftStruct>>,
) -> PyResult<Py<PyAny>> {
    match val {
        ThriftValue::Bool(v) => Ok(v.into_pyobject(py)?.to_owned().into_any().unbind()),
        ThriftValue::Byte(v) => Ok(v.into_pyobject(py)?.into_any().unbind()),
        ThriftValue::I16(v) => Ok(v.into_pyobject(py)?.into_any().unbind()),
        ThriftValue::I32(v) => Ok(v.into_pyobject(py)?.into_any().unbind()),
        ThriftValue::I64(v) => Ok(v.into_pyobject(py)?.into_any().unbind()),
        ThriftValue::Double(v) => Ok(v.into_pyobject(py)?.into_any().unbind()),
        ThriftValue::String(v) => Ok(v.into_pyobject(py)?.into_any().unbind()),
        ThriftValue::Binary(v) => Ok(PyBytes::new(py, v).into_any().unbind()),
        ThriftValue::List(items) | ThriftValue::Set(items) => {
            let list = PyList::empty(py);
            for item in items {
                list.append(thrift_value_to_py(item, py, struct_map)?.bind(py))?;
            }
            Ok(list.into_any().unbind())
        }
        ThriftValue::Map(pairs) => {
            let d = PyDict::new(py);
            for (k, v) in pairs {
                d.set_item(
                    thrift_value_to_py(k, py, struct_map)?.bind(py),
                    thrift_value_to_py(v, py, struct_map)?.bind(py),
                )?;
            }
            Ok(d.into_any().unbind())
        }
        ThriftValue::Struct { name, fields } => {
            let struct_name = name.clone().unwrap_or_default();
            // Resolve the schema for this nested struct from the shared struct_map.
            let schema: Arc<HashMap<String, ThriftField>> = struct_map
                .get(&struct_name)
                .map(|def| Arc::clone(&def.schema_arc))
                .unwrap_or_default();
            // Preserve field order from schema definition when available, fall back
            // to HashMap iteration order otherwise.
            let field_names: Arc<Vec<String>> = struct_map
                .get(&struct_name)
                .map(|def| Arc::clone(&def.field_names_arc))
                .unwrap_or_else(|| Arc::new(fields.keys().cloned().collect()));
            // Build a fully schema-aware instance so its own get_field calls are
            // also lazy and schema-correct.
            let instance = ThriftStructInstance::from_rust(
                struct_name,
                field_names,
                fields.clone(),
                schema,
                Arc::clone(struct_map),
            );
            Ok(Bound::new(py, instance)?.into_any().unbind())
        }
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// JSON Value → Python direct conversion (skips ThriftValue intermediate)
// ──────────────────────────────────────────────────────────────────────────────

/// Walk a `serde_json::Value` tree with Thrift schema to build Python objects
/// directly, avoiding the ThriftValue intermediate allocation.
///
/// `value` is the TJSON-wrapped field value (i.e. `arr[1]` from a
/// `[type_str, value]` pair, already peeled from the outer type tag).
pub(crate) fn json_value_to_py(
    value: &JsonValue,
    field_type: &ThriftType,
    struct_map: &Arc<HashMap<String, ThriftStruct>>,
    py: Python<'_>,
) -> PyResult<Py<PyAny>> {
    match field_type {
        ThriftType::Bool => {
            let v = value.as_i64().unwrap_or(0) != 0 || value.as_bool().unwrap_or(false);
            Ok(v.into_pyobject(py)?.to_owned().into_any().unbind())
        }
        ThriftType::Byte => {
            Ok((value.as_i64().unwrap_or(0) as i8).into_pyobject(py)?.into_any().unbind())
        }
        ThriftType::I16 => {
            Ok((value.as_i64().unwrap_or(0) as i16).into_pyobject(py)?.into_any().unbind())
        }
        ThriftType::I32 => {
            Ok((value.as_i64().unwrap_or(0) as i32).into_pyobject(py)?.into_any().unbind())
        }
        ThriftType::I64 => {
            if let Some(s) = value.as_str() {
                let n: i64 = s.parse().unwrap_or(0);
                Ok(n.into_pyobject(py)?.into_any().unbind())
            } else {
                Ok(value.as_i64().unwrap_or(0).into_pyobject(py)?.into_any().unbind())
            }
        }
        ThriftType::Double => {
            if let Some(s) = value.as_str() {
                let n: f64 = s.parse().unwrap_or(0.0);
                Ok(n.into_pyobject(py)?.into_any().unbind())
            } else {
                Ok(value.as_f64().unwrap_or(0.0).into_pyobject(py)?.into_any().unbind())
            }
        }
        ThriftType::String => {
            let s = value.as_str().unwrap_or("");
            Ok(s.into_pyobject(py)?.into_any().unbind())
        }
        ThriftType::Binary => {
            let s = value.as_str().unwrap_or("");
            let decoded = BASE64.decode(s).map_err(|e| {
                PyErr::new::<pyo3::exceptions::PyValueError, _>(format!("Base64 error: {}", e))
            })?;
            Ok(PyBytes::new(py, &decoded).into_any().unbind())
        }
        // TJSON list value: [elem_type_str, size, item0, item1, ...]
        ThriftType::List(elem_type) | ThriftType::Set(elem_type) => {
            let arr = value
                .as_array()
                .ok_or_else(|| PyErr::new::<pyo3::exceptions::PyTypeError, _>("Expected JSON array"))?;
            let items = if arr.len() >= 2 { &arr[2..] } else { &[] };
            let list = PyList::empty(py);
            for item in items {
                let py_item = json_value_to_py(item, elem_type, struct_map, py)?;
                list.append(py_item.bind(py))?;
            }
            Ok(list.into_any().unbind())
        }
        // TJSON map value: [key_type_str, val_type_str, size, {k: v, ...}]
        ThriftType::Map(key_type, val_type) => {
            let arr = value
                .as_array()
                .ok_or_else(|| PyErr::new::<pyo3::exceptions::PyTypeError, _>("Expected JSON array for map"))?;
            if arr.len() < 4 {
                return Err(PyErr::new::<pyo3::exceptions::PyTypeError, _>("Map array too short"));
            }
            let map_obj = arr[3]
                .as_object()
                .ok_or_else(|| PyErr::new::<pyo3::exceptions::PyTypeError, _>("Expected JSON object in map"))?;
            let dict = PyDict::new(py);
            for (k_str, v) in map_obj.iter() {
                let py_key = json_map_key_to_py(k_str, key_type, py)?;
                let py_val = json_value_to_py(v, val_type, struct_map, py)?;
                dict.set_item(py_key.bind(py), py_val.bind(py))?;
            }
            Ok(dict.into_any().unbind())
        }
        // TJSON struct value: {"fid_str": [type_str, field_value], ...}
        ThriftType::Struct(struct_name) => {
            let struct_def = struct_map.get(struct_name).ok_or_else(|| {
                PyErr::new::<pyo3::exceptions::PyValueError, _>(format!(
                    "Unknown struct: {}", struct_name
                ))
            })?;
            let obj = value
                .as_object()
                .ok_or_else(|| PyErr::new::<pyo3::exceptions::PyTypeError, _>("Expected JSON object for struct"))?;
            let mut cache = HashMap::new();
            for (fid_str, entry) in obj.iter() {
                let fid: i16 = fid_str.parse().unwrap_or(0);
                let entry_arr = entry.as_array();
                if entry_arr.is_none() || entry_arr.unwrap().len() < 2 {
                    continue;
                }
                let entry_arr = entry_arr.unwrap();
                if let Some(&idx) = struct_def.field_map.get(&fid) {
                    let field = &struct_def.fields[idx];
                    let py_val = json_value_to_py(&entry_arr[1], &field.field_type, struct_map, py)?;
                    cache.insert(field.name.clone(), py_val);
                }
            }
            let instance = ThriftStructInstance::from_python_cache(
                struct_name.clone(),
                Arc::clone(&struct_def.field_names_arc),
                cache,
                Arc::clone(&struct_def.schema_arc),
                Arc::clone(struct_map),
            );
            Ok(Bound::new(py, instance)?.into_any().unbind())
        }
    }
}

/// Convert a TJSON map key string to the appropriate Python type.
fn json_map_key_to_py(key: &str, key_type: &ThriftType, py: Python<'_>) -> PyResult<Py<PyAny>> {
    match key_type {
        ThriftType::Bool => {
            let v = key == "1" || key.eq_ignore_ascii_case("true");
            Ok(v.into_pyobject(py)?.to_owned().into_any().unbind())
        }
        ThriftType::Byte => {
            let n: i8 = key.parse().unwrap_or(0);
            Ok(n.into_pyobject(py)?.into_any().unbind())
        }
        ThriftType::I16 => {
            let n: i16 = key.parse().unwrap_or(0);
            Ok(n.into_pyobject(py)?.into_any().unbind())
        }
        ThriftType::I32 => {
            let n: i32 = key.parse().unwrap_or(0);
            Ok(n.into_pyobject(py)?.into_any().unbind())
        }
        ThriftType::I64 => {
            let n: i64 = key.parse().unwrap_or(0);
            Ok(n.into_pyobject(py)?.into_any().unbind())
        }
        ThriftType::Double => {
            let n: f64 = key.parse().unwrap_or(0.0);
            Ok(n.into_pyobject(py)?.into_any().unbind())
        }
        _ => Ok(key.into_pyobject(py)?.into_any().unbind()),
    }
}

/// Parse JSON bytes and convert directly to Python dict using schema,
/// then wrap in ThriftStructInstance. Skips the ThriftValue intermediate.
pub(crate) fn deserialize_json_direct<'py>(
    struct_def: &ThriftStruct,
    json_bytes: &[u8],
    py: Python<'py>,
) -> PyResult<Bound<'py, ThriftStructInstance>> {
    let root: JsonValue = serde_json::from_slice(json_bytes).map_err(|e| {
        PyErr::new::<pyo3::exceptions::PyValueError, _>(format!("JSON parse error: {}", e))
    })?;

    let obj = root
        .as_object()
        .ok_or_else(|| PyErr::new::<pyo3::exceptions::PyTypeError, _>("Expected JSON object for struct"))?;

    let mut cache = HashMap::new();
    for (fid_str, entry) in obj.iter() {
        let fid: i16 = fid_str.parse().unwrap_or(0);
        let entry_arr = entry.as_array();
        if entry_arr.is_none() || entry_arr.unwrap().len() < 2 {
            continue;
        }
        let entry_arr = entry_arr.unwrap();
        if let Some(&idx) = struct_def.field_map.get(&fid) {
            let field = &struct_def.fields[idx];
            let py_val = json_value_to_py(&entry_arr[1], &field.field_type, &struct_def.struct_map, py)?;
            cache.insert(field.name.clone(), py_val);
        }
    }

    Ok(Bound::new(
        py,
        ThriftStructInstance::from_python_cache(
            struct_def.name.clone(),
            Arc::clone(&struct_def.field_names_arc),
            cache,
            Arc::clone(&struct_def.schema_arc),
            Arc::clone(&struct_def.struct_map),
        ),
    )?)
}

/// Best-effort conversion of an arbitrary Python object to a `ThriftValue`
/// without schema type information.
pub(crate) fn py_any_to_thrift_value(val: &Bound<'_, PyAny>) -> PyResult<ThriftValue> {
    if val.is_none() {
        return Err(PyErr::new::<pyo3::exceptions::PyTypeError, _>(
            "None has no ThriftValue representation",
        ));
    }
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
        let mut fields = inst.values.clone();
        let has_schema = !inst.schema.is_empty();
        for name in inst.field_names.as_ref() {
            if let Some(py_val) = inst.cache.get(name.as_str()) {
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

/// Schema-aware conversion of a Python value to the exact `ThriftValue` variant.
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
        ThriftType::I16 => Ok(ThriftValue::I16(val.extract::<i16>()?)),
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
                let schema: Option<&ThriftStruct> = struct_map.get(struct_name.as_str());
                let mut fields = inst.values.clone();
                for name in inst.field_names.as_ref() {
                    if let Some(py_val) = inst.cache.get(name) {
                        let bound = py_val.bind(val.py());
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
                py_any_to_thrift_value(val)
            }
        }
    }
}
