// ──────────────────────────────────────────────────────────────────────────────
// json.rs  –  TJSONProtocol implementation
//
// Wire format follows the Apache Thrift TJSONProtocol specification:
//   Message  → [1, "name", type, seqid, {struct_body}]
//   Struct   → {"fid": [type_str, value], ...}
//   List/Set → ["type_str", size, v0, v1, ...]
//   Map      → ["ktype_str", "vtype_str", size, {k: v, ...}]
//   Bool     → 1 / 0
//   i64      → string (to avoid JSON integer precision loss)
//   binary   → base64-encoded string
// ──────────────────────────────────────────────────────────────────────────────

use super::types::*;
use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use serde_json::{json, Map, Value};
use std::io::{Read, Write};

// ─── TType ↔ JSON type-name string ─────────────────────────────────────────

fn ttype_to_json_name(t: TType) -> &'static str {
    match t {
        TType::Bool   => "tf",
        TType::Byte   => "i8",
        TType::I16    => "i16",
        TType::I32    => "i32",
        TType::I64    => "i64",
        TType::Double => "dbl",
        TType::String => "str",
        TType::Struct => "rec",
        TType::Map    => "map",
        TType::List   => "lst",
        TType::Set    => "set",
        TType::Stop | TType::Void => "stop",
    }
}

fn json_name_to_ttype(s: &str) -> Option<TType> {
    match s {
        "tf"   => Some(TType::Bool),
        "i8"   => Some(TType::Byte),
        "i16"  => Some(TType::I16),
        "i32"  => Some(TType::I32),
        "i64"  => Some(TType::I64),
        "dbl"  => Some(TType::Double),
        "str"  => Some(TType::String),
        "rec"  => Some(TType::Struct),
        "map"  => Some(TType::Map),
        "lst"  => Some(TType::List),
        "set"  => Some(TType::Set),
        _      => None,
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// Writer state machine
// ──────────────────────────────────────────────────────────────────────────────

/// A cursor into the tree we are building.
enum WriteFrame {
    /// Top-level message array `[ver, name, mtype, seqid, body]`.
    Message {
        name: String,
        message_type: u8,
        seq_id: i32,
    },
    /// Struct body — a JSON object `{fid_str: [ttype_str, value]}`.
    Struct {
        obj: Map<String, Value>,
        current_field: Option<(i16, TType)>,
    },
    /// List/Set array `[ttype_str, size, v0, v1, …]`.
    List {
        elem_type: TType,
        items: Vec<Value>,
    },
    /// Map `[ktype_str, vtype_str, size, {k: v, …}]` — we collect pairs.
    Map {
        key_type: TType,
        val_type: TType,
        pairs: Vec<(Value, Value)>,
        pending_key: Option<Value>,
    },
}

pub struct JSONProtocolWriter<W: Write> {
    writer: W,
    stack: Vec<WriteFrame>,
}

impl<W: Write> JSONProtocolWriter<W> {
    pub fn new(writer: W) -> Self {
        Self {
            writer,
            stack: Vec::new(),
        }
    }

    /// Push a scalar value into the current frame.
    fn push_value(&mut self, v: Value) -> Result<(), ProtocolError> {
        match self.stack.last_mut() {
            None => {
                // No frame — write directly (standalone struct serialisation)
                let s = serde_json::to_string(&v)
                    .map_err(|e| ProtocolError::Io(std::io::Error::new(std::io::ErrorKind::Other, e)))?;
                self.writer.write_all(s.as_bytes())?;
            }
            Some(WriteFrame::Struct { current_field, obj }) => {
                if let Some((fid, ftype)) = current_field.take() {
                    let entry = json!([ttype_to_json_name(ftype), v]);
                    obj.insert(fid.to_string(), entry);
                }
            }
            Some(WriteFrame::List { items, .. }) => {
                items.push(v);
            }
            Some(WriteFrame::Map { pending_key, pairs, .. }) => {
                if let Some(k) = pending_key.take() {
                    pairs.push((k, v));
                } else {
                    *pending_key = Some(v);
                }
            }
            Some(WriteFrame::Message { .. }) => {}
        }
        Ok(())
    }

    /// Convert a finished frame into its JSON value.
    fn frame_to_value(frame: WriteFrame) -> Value {
        match frame {
            WriteFrame::Struct { obj, .. } => Value::Object(obj),
            WriteFrame::List { elem_type, items } => {
                let mut arr = vec![
                    Value::String(ttype_to_json_name(elem_type).to_owned()),
                    Value::Number((items.len() as u64).into()),
                ];
                arr.extend(items);
                Value::Array(arr)
            }
            WriteFrame::Map { key_type, val_type, pairs, .. } => {
                let map_obj: Map<String, Value> = pairs
                    .into_iter()
                    .map(|(k, v)| {
                        let key_str = match &k {
                            Value::String(s) => s.clone(),
                            Value::Number(n) => n.to_string(),
                            other => other.to_string(),
                        };
                        (key_str, v)
                    })
                    .collect();
                json!([
                    ttype_to_json_name(key_type),
                    ttype_to_json_name(val_type),
                    map_obj.len(),
                    Value::Object(map_obj)
                ])
            }
            WriteFrame::Message { name, message_type, seq_id } => {
                json!([1, name, message_type, seq_id])
            }
        }
    }
}

impl<W: Write> TOutputProtocol for JSONProtocolWriter<W> {
    fn write_message_begin(&mut self, identifier: &MessageBegin) -> Result<(), ProtocolError> {
        self.stack.push(WriteFrame::Message {
            name: identifier.name.clone(),
            message_type: identifier.message_type,
            seq_id: identifier.seq_id,
        });
        // Push a struct frame for the message body.
        self.stack.push(WriteFrame::Struct {
            obj: Map::new(),
            current_field: None,
        });
        Ok(())
    }

    fn write_message_end(&mut self) -> Result<(), ProtocolError> {
        // Pop struct body frame.
        let body_frame = self.stack.pop()
            .ok_or_else(|| ProtocolError::Io(std::io::Error::new(std::io::ErrorKind::Other, "stack underflow")))?;
        let body = Self::frame_to_value(body_frame);
        // Pop message frame.
        let msg_frame = self.stack.pop()
            .ok_or_else(|| ProtocolError::Io(std::io::Error::new(std::io::ErrorKind::Other, "stack underflow")))?;
        if let WriteFrame::Message { name, message_type, seq_id } = msg_frame {
            let msg = json!([1, name, message_type, seq_id, body]);
            let s = serde_json::to_string(&msg)
                .map_err(|e| ProtocolError::Io(std::io::Error::new(std::io::ErrorKind::Other, e)))?;
            self.writer.write_all(s.as_bytes())?;
        }
        Ok(())
    }

    fn write_struct_begin(&mut self, _name: &str) -> Result<(), ProtocolError> {
        self.stack.push(WriteFrame::Struct {
            obj: Map::new(),
            current_field: None,
        });
        Ok(())
    }

    fn write_struct_end(&mut self) -> Result<(), ProtocolError> {
        let frame = self.stack.pop()
            .ok_or_else(|| ProtocolError::Io(std::io::Error::new(std::io::ErrorKind::Other, "stack underflow")))?;
        let v = Self::frame_to_value(frame);
        if self.stack.is_empty() {
            // Outermost struct (standalone serialisation) — flush to writer.
            let s = serde_json::to_string(&v)
                .map_err(|e| ProtocolError::Io(std::io::Error::new(std::io::ErrorKind::Other, e)))?;
            self.writer.write_all(s.as_bytes())?;
        } else {
            self.push_value(v)?;
        }
        Ok(())
    }

    fn write_field_begin(&mut self, field: &FieldBegin) -> Result<(), ProtocolError> {
        if let Some(WriteFrame::Struct { current_field, .. }) = self.stack.last_mut() {
            *current_field = Some((field.id, field.field_type));
        }
        Ok(())
    }

    fn write_field_end(&mut self) -> Result<(), ProtocolError> { Ok(()) }
    fn write_field_stop(&mut self) -> Result<(), ProtocolError> { Ok(()) }

    fn write_bool(&mut self, value: bool) -> Result<(), ProtocolError> {
        self.push_value(Value::Number((if value { 1u64 } else { 0u64 }).into()))
    }

    fn write_byte(&mut self, value: i8) -> Result<(), ProtocolError> {
        self.push_value(Value::Number((value as i64).into()))
    }

    fn write_i16(&mut self, value: i16) -> Result<(), ProtocolError> {
        self.push_value(Value::Number((value as i64).into()))
    }

    fn write_i32(&mut self, value: i32) -> Result<(), ProtocolError> {
        self.push_value(Value::Number((value as i64).into()))
    }

    fn write_i64(&mut self, value: i64) -> Result<(), ProtocolError> {
        // i64 encoded as string to avoid JSON integer precision loss.
        self.push_value(Value::String(value.to_string()))
    }

    fn write_double(&mut self, value: f64) -> Result<(), ProtocolError> {
        let n = serde_json::Number::from_f64(value)
            .unwrap_or_else(|| serde_json::Number::from(0i64));
        self.push_value(Value::Number(n))
    }

    fn write_string(&mut self, value: &str) -> Result<(), ProtocolError> {
        self.push_value(Value::String(value.to_owned()))
    }

    fn write_binary(&mut self, value: &[u8]) -> Result<(), ProtocolError> {
        self.push_value(Value::String(BASE64.encode(value)))
    }

    fn write_map_begin(&mut self, identifier: &MapBegin) -> Result<(), ProtocolError> {
        self.stack.push(WriteFrame::Map {
            key_type: identifier.key_type,
            val_type: identifier.value_type,
            pairs: Vec::with_capacity(identifier.size as usize),
            pending_key: None,
        });
        Ok(())
    }

    fn write_map_end(&mut self) -> Result<(), ProtocolError> {
        let frame = self.stack.pop()
            .ok_or_else(|| ProtocolError::Io(std::io::Error::new(std::io::ErrorKind::Other, "stack underflow")))?;
        let v = Self::frame_to_value(frame);
        self.push_value(v)
    }

    fn write_list_begin(&mut self, identifier: &ListBegin) -> Result<(), ProtocolError> {
        self.stack.push(WriteFrame::List {
            elem_type: identifier.element_type,
            items: Vec::with_capacity(identifier.size as usize),
        });
        Ok(())
    }

    fn write_list_end(&mut self) -> Result<(), ProtocolError> {
        let frame = self.stack.pop()
            .ok_or_else(|| ProtocolError::Io(std::io::Error::new(std::io::ErrorKind::Other, "stack underflow")))?;
        let v = Self::frame_to_value(frame);
        self.push_value(v)
    }

    fn write_set_begin(&mut self, identifier: &SetBegin) -> Result<(), ProtocolError> {
        self.stack.push(WriteFrame::List {
            elem_type: identifier.element_type,
            items: Vec::with_capacity(identifier.size as usize),
        });
        Ok(())
    }

    fn write_set_end(&mut self) -> Result<(), ProtocolError> {
        self.write_list_end()
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// Reader state machine
// ──────────────────────────────────────────────────────────────────────────────

#[derive(Debug)]
enum ReadFrame {
    Struct {
        fields: Vec<(String, Value)>,
        index: usize,
        current_type: Option<TType>,
        current_value: Option<Value>,
    },
    List {
        elem_type: TType,
        items: Vec<Value>,
        index: usize,
    },
    Map {
        key_type: TType,
        val_type: TType,
        pairs: Vec<(Value, Value)>,
        index: usize,
        reading_key: bool,
    },
}

pub struct JSONProtocolReader<R: Read> {
    reader: R,
    root: Option<Value>,
    stack: Vec<ReadFrame>,
}

impl<R: Read> JSONProtocolReader<R> {
    pub fn new(reader: R) -> Self {
        Self {
            reader,
            root: None,
            stack: Vec::new(),
        }
    }

    fn ensure_parsed(&mut self) -> Result<(), ProtocolError> {
        if self.root.is_none() {
            let mut buf = String::new();
            self.reader.read_to_string(&mut buf).map_err(ProtocolError::Io)?;
            let v: Value = serde_json::from_str(&buf)
                .map_err(|e| ProtocolError::Io(std::io::Error::new(std::io::ErrorKind::InvalidData, e)))?;
            self.root = Some(v);
        }
        Ok(())
    }

    /// Pop the next scalar value from the current frame, or take root for standalone reads.
    fn pop_scalar(&mut self) -> Result<Value, ProtocolError> {
        match self.stack.last_mut() {
            Some(ReadFrame::Struct { current_value, .. }) => {
                current_value.take().ok_or(ProtocolError::InvalidFieldType)
            }
            Some(ReadFrame::List { items, index, .. }) => {
                let i = *index;
                *index += 1;
                items.get(i).cloned().ok_or(ProtocolError::InvalidFieldType)
            }
            Some(ReadFrame::Map { pairs, index, reading_key, .. }) => {
                let i = *index;
                let is_key = *reading_key;
                *reading_key = !is_key;
                if !is_key {
                    *index += 1;
                }
                let (k, v) = pairs.get(i).ok_or(ProtocolError::InvalidFieldType)?;
                Ok(if is_key { k.clone() } else { v.clone() })
            }
            None => self.root.take().ok_or(ProtocolError::InvalidFieldType),
        }
    }
}

impl<R: Read> TInputProtocol for JSONProtocolReader<R> {
    fn read_message_begin(&mut self) -> Result<MessageBegin, ProtocolError> {
        self.ensure_parsed()?;
        let data = self.root.as_ref().ok_or(ProtocolError::InvalidFieldType)?.clone();
        let arr = data.as_array().ok_or(ProtocolError::InvalidFieldType)?;
        if arr.len() < 5 {
            return Err(ProtocolError::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidData, "JSON message array too short")));
        }
        let name = arr[1].as_str().unwrap_or("").to_owned();
        let message_type = arr[2].as_u64().unwrap_or(1) as u8;
        let seq_id = arr[3].as_i64().unwrap_or(0) as i32;
        let body = arr[4].clone();
        if let Some(obj) = body.as_object() {
            let fields: Vec<(String, Value)> = obj.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
            self.stack.push(ReadFrame::Struct { fields, index: 0, current_type: None, current_value: None });
        }
        Ok(MessageBegin { name, message_type, seq_id })
    }

    fn read_message_end(&mut self) -> Result<(), ProtocolError> {
        self.stack.pop();
        Ok(())
    }

    fn read_struct_begin(&mut self) -> Result<(), ProtocolError> {
        // For top-level structs (no message wrapping), read from root.
        // For nested structs (inside a field), the value is in current_value.
        let v = if self.stack.is_empty() {
            self.ensure_parsed()?;
            self.root.take().ok_or(ProtocolError::InvalidFieldType)?
        } else {
            self.pop_scalar()?
        };
        let obj = v.as_object().ok_or(ProtocolError::InvalidFieldType)?;
        let fields: Vec<(String, Value)> = obj.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
        self.stack.push(ReadFrame::Struct { fields, index: 0, current_type: None, current_value: None });
        Ok(())
    }

    fn read_struct_end(&mut self) -> Result<(), ProtocolError> {
        self.stack.pop();
        Ok(())
    }

    fn read_field_begin(&mut self) -> Result<FieldBegin, ProtocolError> {
        match self.stack.last_mut() {
            Some(ReadFrame::Struct { fields, index, current_type, current_value }) => {
                let i = *index;
                if i >= fields.len() {
                    return Ok(FieldBegin { name: None, field_type: TType::Stop, id: 0 });
                }
                let (fid_str, entry) = &fields[i];
                let fid: i16 = fid_str.parse().unwrap_or(0);
                let arr = entry.as_array().ok_or(ProtocolError::InvalidFieldType)?;
                if arr.len() < 2 {
                    return Err(ProtocolError::InvalidFieldType);
                }
                let type_str = arr[0].as_str().unwrap_or("");
                let ftype = json_name_to_ttype(type_str).ok_or(ProtocolError::InvalidFieldType)?;
                *current_type = Some(ftype);
                *current_value = Some(arr[1].clone());
                *index += 1;
                Ok(FieldBegin { name: None, field_type: ftype, id: fid })
            }
            _ => Ok(FieldBegin { name: None, field_type: TType::Stop, id: 0 }),
        }
    }

    fn read_field_end(&mut self) -> Result<(), ProtocolError> { Ok(()) }

    fn read_bool(&mut self) -> Result<bool, ProtocolError> {
        let v = self.pop_scalar()?;
        Ok(v.as_i64().unwrap_or(0) != 0 || v.as_bool().unwrap_or(false))
    }

    fn read_byte(&mut self) -> Result<i8, ProtocolError> {
        let v = self.pop_scalar()?;
        Ok(v.as_i64().unwrap_or(0) as i8)
    }

    fn read_i16(&mut self) -> Result<i16, ProtocolError> {
        let v = self.pop_scalar()?;
        Ok(v.as_i64().unwrap_or(0) as i16)
    }

    fn read_i32(&mut self) -> Result<i32, ProtocolError> {
        let v = self.pop_scalar()?;
        Ok(v.as_i64().unwrap_or(0) as i32)
    }

    fn read_i64(&mut self) -> Result<i64, ProtocolError> {
        let v = self.pop_scalar()?;
        if let Some(s) = v.as_str() {
            Ok(s.parse().unwrap_or(0))
        } else {
            Ok(v.as_i64().unwrap_or(0))
        }
    }

    fn read_double(&mut self) -> Result<f64, ProtocolError> {
        let v = self.pop_scalar()?;
        if let Some(s) = v.as_str() {
            Ok(s.parse().unwrap_or(0.0))
        } else {
            Ok(v.as_f64().unwrap_or(0.0))
        }
    }

    fn read_string(&mut self) -> Result<String, ProtocolError> {
        let v = self.pop_scalar()?;
        Ok(v.as_str().unwrap_or("").to_owned())
    }

    fn read_binary(&mut self) -> Result<Vec<u8>, ProtocolError> {
        let v = self.pop_scalar()?;
        let s = v.as_str().unwrap_or("");
        BASE64.decode(s)
            .map_err(|e| ProtocolError::Io(std::io::Error::new(std::io::ErrorKind::InvalidData, e)))
    }

    fn read_map_begin(&mut self) -> Result<MapBegin, ProtocolError> {
        let v = self.pop_scalar()?;
        // format: [ktype_str, vtype_str, size, {k: v, ...}]
        let arr = v.as_array().ok_or(ProtocolError::InvalidFieldType)?;
        if arr.len() < 4 {
            return Err(ProtocolError::InvalidFieldType);
        }
        let key_type = json_name_to_ttype(arr[0].as_str().unwrap_or("")).ok_or(ProtocolError::InvalidFieldType)?;
        let val_type = json_name_to_ttype(arr[1].as_str().unwrap_or("")).ok_or(ProtocolError::InvalidFieldType)?;
        let size = arr[2].as_i64().unwrap_or(0) as i32;
        let map_obj = arr[3].as_object().ok_or(ProtocolError::InvalidFieldType)?;
        let pairs: Vec<(Value, Value)> = map_obj.iter()
            .map(|(k, v)| (Value::String(k.clone()), v.clone()))
            .collect();
        self.stack.push(ReadFrame::Map { key_type, val_type, pairs, index: 0, reading_key: true });
        Ok(MapBegin { key_type, value_type: val_type, size })
    }

    fn read_map_end(&mut self) -> Result<(), ProtocolError> {
        self.stack.pop();
        Ok(())
    }

    fn read_list_begin(&mut self) -> Result<ListBegin, ProtocolError> {
        let v = self.pop_scalar()?;
        // format: [elem_type_str, size, v0, v1, ...]
        let arr = v.as_array().ok_or(ProtocolError::InvalidFieldType)?;
        if arr.len() < 2 {
            return Err(ProtocolError::InvalidFieldType);
        }
        let elem_type = json_name_to_ttype(arr[0].as_str().unwrap_or("")).ok_or(ProtocolError::InvalidFieldType)?;
        let size = arr[1].as_i64().unwrap_or(0) as i32;
        let items: Vec<Value> = arr[2..].to_vec();
        self.stack.push(ReadFrame::List { elem_type, items, index: 0 });
        Ok(ListBegin { element_type: elem_type, size })
    }

    fn read_list_end(&mut self) -> Result<(), ProtocolError> {
        self.stack.pop();
        Ok(())
    }

    fn read_set_begin(&mut self) -> Result<SetBegin, ProtocolError> {
        let lb = self.read_list_begin()?;
        Ok(SetBegin { element_type: lb.element_type, size: lb.size })
    }

    fn read_set_end(&mut self) -> Result<(), ProtocolError> {
        self.read_list_end()
    }

    fn read_u8_raw(&mut self) -> Result<u8, ProtocolError> {
        Ok(self.read_byte()? as u8)
    }
    fn read_i16_raw(&mut self) -> Result<i16, ProtocolError> { self.read_i16() }
    fn read_i32_raw(&mut self) -> Result<i32, ProtocolError> { self.read_i32() }
    fn read_i64_raw(&mut self) -> Result<i64, ProtocolError> { self.read_i64() }
}


