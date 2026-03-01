use super::types::*;
use byteorder::{ReadBytesExt, WriteBytesExt, BigEndian};
use std::io::{Read, Write};

pub const COMPACT_PROTOCOL_ID: u8 = 0x82;
pub const COMPACT_VERSION: u8 = 1;
pub const COMPACT_VERSION_MASK: u8 = 0x1f;
pub const COMPACT_TYPE_MASK: u8 = 0xe0;
pub const COMPACT_TYPE_SHIFT: u32 = 5;

#[derive(Debug, Clone, Copy)]
#[repr(u8)]
enum CompactType {
    Stop = 0x00,
    BooleanTrue = 0x01,
    BooleanFalse = 0x02,
    Byte = 0x03,
    I16 = 0x04,
    I32 = 0x05,
    I64 = 0x06,
    Double = 0x07,
    Binary = 0x08,
    List = 0x09,
    Set = 0x0A,
    Map = 0x0B,
    Struct = 0x0C,
}

impl CompactType {
    fn from_u8(b: u8) -> Option<Self> {
        match b & 0x0f {
            0x00 => Some(CompactType::Stop),
            0x01 => Some(CompactType::BooleanTrue),
            0x02 => Some(CompactType::BooleanFalse),
            0x03 => Some(CompactType::Byte),
            0x04 => Some(CompactType::I16),
            0x05 => Some(CompactType::I32),
            0x06 => Some(CompactType::I64),
            0x07 => Some(CompactType::Double),
            0x08 => Some(CompactType::Binary),
            0x09 => Some(CompactType::List),
            0x0A => Some(CompactType::Set),
            0x0B => Some(CompactType::Map),
            0x0C => Some(CompactType::Struct),
            _ => None,
        }
    }

    fn to_ttype(self) -> TType {
        match self {
            CompactType::Stop => TType::Stop,
            CompactType::BooleanTrue | CompactType::BooleanFalse => TType::Bool,
            CompactType::Byte => TType::Byte,
            CompactType::I16 => TType::I16,
            CompactType::I32 => TType::I32,
            CompactType::I64 => TType::I64,
            CompactType::Double => TType::Double,
            CompactType::Binary => TType::String,
            CompactType::List => TType::List,
            CompactType::Set => TType::Set,
            CompactType::Map => TType::Map,
            CompactType::Struct => TType::Struct,
        }
    }

    fn from_ttype(ttype: TType) -> Self {
        match ttype {
            TType::Stop => CompactType::Stop,
            TType::Bool => CompactType::BooleanTrue, // or False
            TType::Byte => CompactType::Byte,
            TType::I16 => CompactType::I16,
            TType::I32 => CompactType::I32,
            TType::I64 => CompactType::I64,
            TType::Double => CompactType::Double,
            TType::String => CompactType::Binary,
            TType::List => CompactType::List,
            TType::Set => CompactType::Set,
            TType::Map => CompactType::Map,
            TType::Struct => CompactType::Struct,
            _ => CompactType::Stop,
        }
    }
}

pub struct CompactProtocolReader<R: Read> {
    reader: R,
    last_field_id: Vec<i16>,
    boolean_field: Option<bool>,
}

impl<R: Read> CompactProtocolReader<R> {
    pub fn new(reader: R) -> Self {
        Self {
            reader,
            last_field_id: vec![0],
            boolean_field: None,
        }
    }

    fn read_varint32(&mut self) -> Result<i32, ProtocolError> {
        let mut result: u32 = 0;
        let mut shift = 0;
        loop {
            let b = self.reader.read_u8()?;
            result |= ((b & 0x7f) as u32) << shift;
            if (b & 0x80) == 0 {
                break;
            }
            shift += 7;
        }
        Ok(((result >> 1) ^ (-( (result & 1) as i32)) as u32) as i32)
    }

    fn read_varint64(&mut self) -> Result<i64, ProtocolError> {
        let mut result: u64 = 0;
        let mut shift = 0;
        loop {
            let b = self.reader.read_u8()?;
            result |= ((b & 0x7f) as u64) << shift;
            if (b & 0x80) == 0 {
                break;
            }
            shift += 7;
        }
        Ok(((result >> 1) ^ (-( (result & 1) as i64)) as u64) as i64)
    }
}

impl<R: Read> TInputProtocol for CompactProtocolReader<R> {
    fn read_message_begin(&mut self) -> Result<MessageBegin, ProtocolError> {
        let protocol_id = self.reader.read_u8()?;
        if protocol_id != COMPACT_PROTOCOL_ID {
            return Err(ProtocolError::Io(std::io::Error::new(std::io::ErrorKind::InvalidData, "Bad protocol id")));
        }
        let version_and_type = self.reader.read_u8()?;
        let version = version_and_type & COMPACT_VERSION_MASK;
        if version != COMPACT_VERSION {
            return Err(ProtocolError::Io(std::io::Error::new(std::io::ErrorKind::InvalidData, "Bad protocol version")));
        }
        let message_type = (version_and_type & COMPACT_TYPE_MASK) >> COMPACT_TYPE_SHIFT;
        let seq_id = self.read_varint32()?;
        let name_len = self.read_varint32()? as usize;
        let mut name_buf = vec![0u8; name_len];
        self.reader.read_exact(&mut name_buf)?;
        let name = String::from_utf8(name_buf).map_err(|e| ProtocolError::Io(std::io::Error::new(std::io::ErrorKind::InvalidData, e)))?;
        Ok(MessageBegin { name, message_type, seq_id })
    }

    fn read_message_end(&mut self) -> Result<(), ProtocolError> { Ok(()) }

    fn read_struct_begin(&mut self) -> Result<(), ProtocolError> {
        self.last_field_id.push(0);
        Ok(())
    }

    fn read_struct_end(&mut self) -> Result<(), ProtocolError> {
        self.last_field_id.pop();
        Ok(())
    }

    fn read_field_begin(&mut self) -> Result<FieldBegin, ProtocolError> {
        let b = self.reader.read_u8()?;
        if (b & 0x0f) == CompactType::Stop as u8 {
            return Ok(FieldBegin { name: None, field_type: TType::Stop, id: 0 });
        }

        let modifier = (b & 0xf0) >> 4;
        let id = if modifier == 0 {
            self.reader.read_i16::<BigEndian>()?
        } else {
            let last_id = self.last_field_id.last_mut().unwrap();
            *last_id += modifier as i16;
            *last_id
        };

        let compact_type = CompactType::from_u8(b & 0x0f).ok_or(ProtocolError::InvalidFieldType)?;
        match compact_type {
            CompactType::BooleanTrue => self.boolean_field = Some(true),
            CompactType::BooleanFalse => self.boolean_field = Some(false),
            _ => {}
        }

        Ok(FieldBegin { name: None, field_type: compact_type.to_ttype(), id })
    }

    fn read_field_end(&mut self) -> Result<(), ProtocolError> { Ok(()) }

    fn read_bool(&mut self) -> Result<bool, ProtocolError> {
        if let Some(v) = self.boolean_field.take() {
            Ok(v)
        } else {
            Ok(self.reader.read_u8()? == CompactType::BooleanTrue as u8)
        }
    }

    fn read_byte(&mut self) -> Result<i8, ProtocolError> { Ok(self.reader.read_i8()?) }

    fn read_i16(&mut self) -> Result<i16, ProtocolError> { Ok(self.read_varint32()? as i16) }

    fn read_i32(&mut self) -> Result<i32, ProtocolError> { self.read_varint32() }

    fn read_i64(&mut self) -> Result<i64, ProtocolError> { self.read_varint64() }

    fn read_double(&mut self) -> Result<f64, ProtocolError> { Ok(self.reader.read_f64::<BigEndian>()?) }

    fn read_string(&mut self) -> Result<String, ProtocolError> {
        let length = self.read_varint32()? as usize;
        let mut buffer = vec![0u8; length];
        self.reader.read_exact(&mut buffer)?;
        String::from_utf8(buffer).map_err(|e| ProtocolError::Io(std::io::Error::new(std::io::ErrorKind::InvalidData, e)))
    }

    fn read_binary(&mut self) -> Result<Vec<u8>, ProtocolError> {
        let length = self.read_varint32()? as usize;
        let mut buffer = vec![0u8; length];
        self.reader.read_exact(&mut buffer)?;
        Ok(buffer)
    }

    fn read_map_begin(&mut self) -> Result<MapBegin, ProtocolError> {
        let size = self.read_varint32()?;
        if size == 0 {
            return Ok(MapBegin { key_type: TType::Stop, value_type: TType::Stop, size: 0 });
        }
        let types = self.reader.read_u8()?;
        let key_type = CompactType::from_u8(types >> 4).ok_or(ProtocolError::InvalidFieldType)?.to_ttype();
        let value_type = CompactType::from_u8(types & 0x0f).ok_or(ProtocolError::InvalidFieldType)?.to_ttype();
        Ok(MapBegin { key_type, value_type, size })
    }

    fn read_map_end(&mut self) -> Result<(), ProtocolError> { Ok(()) }

    fn read_list_begin(&mut self) -> Result<ListBegin, ProtocolError> {
        let b = self.reader.read_u8()?;
        let mut size = (b >> 4) as i32;
        if size == 15 {
            size = self.read_varint32()?;
        }
        let element_type = CompactType::from_u8(b & 0x0f).ok_or(ProtocolError::InvalidFieldType)?.to_ttype();
        Ok(ListBegin { element_type, size })
    }

    fn read_list_end(&mut self) -> Result<(), ProtocolError> { Ok(()) }

    fn read_set_begin(&mut self) -> Result<SetBegin, ProtocolError> {
        let lb = self.read_list_begin()?;
        Ok(SetBegin { element_type: lb.element_type, size: lb.size })
    }

    fn read_set_end(&mut self) -> Result<(), ProtocolError> { Ok(()) }

    fn read_u8_raw(&mut self) -> Result<u8, ProtocolError> { Ok(self.reader.read_u8()?) }
    fn read_i16_raw(&mut self) -> Result<i16, ProtocolError> { Ok(self.read_varint32()? as i16) }
    fn read_i32_raw(&mut self) -> Result<i32, ProtocolError> { self.read_varint32() }
    fn read_i64_raw(&mut self) -> Result<i64, ProtocolError> { self.read_varint64() }
}

pub struct CompactProtocolWriter<W: Write> {
    writer: W,
    last_field_id: Vec<i16>,
    /// When we encounter write_field_begin for a Bool field, we stash the field id
    /// here and defer writing the byte until write_bool is called (because the
    /// compact protocol encodes the bool value in the type nibble of the field header).
    bool_field_pending: Option<i16>,
}

impl<W: Write> CompactProtocolWriter<W> {
    pub fn new(writer: W) -> Self {
        Self {
            writer,
            last_field_id: vec![0],
            bool_field_pending: None,
        }
    }

    fn write_field_begin_inner(&mut self, field_type: TType, field_id: i16) -> Result<(), ProtocolError> {
        let last_id = *self.last_field_id.last().unwrap_or(&0);
        let delta = field_id - last_id;
        if delta > 0 && delta <= 15 {
            self.writer.write_u8(((delta as u8) << 4) | (CompactType::from_ttype(field_type) as u8))?;
        } else {
            self.writer.write_u8(CompactType::from_ttype(field_type) as u8)?;
            self.writer.write_i16::<BigEndian>(field_id)?;
        }
        *self.last_field_id.last_mut().unwrap() = field_id;
        Ok(())
    }

    fn write_varint32(&mut self, n: i32) -> Result<(), ProtocolError> {
        let mut u = ( (n << 1) ^ (n >> 31) ) as u32;
        loop {
            if (u & !0x7f) == 0 {
                self.writer.write_u8(u as u8)?;
                return Ok(());
            } else {
                self.writer.write_u8(((u & 0x7f) | 0x80) as u8)?;
                u >>= 7;
            }
        }
    }

    fn write_varint64(&mut self, n: i64) -> Result<(), ProtocolError> {
        let mut u = ( (n << 1) ^ (n >> 63) ) as u64;
        loop {
            if (u & !0x7f) == 0 {
                self.writer.write_u8(u as u8)?;
                return Ok(());
            } else {
                self.writer.write_u8(((u & 0x7f) | 0x80) as u8)?;
                u >>= 7;
            }
        }
    }
}

impl<W: Write> TOutputProtocol for CompactProtocolWriter<W> {
    fn write_message_begin(&mut self, identifier: &MessageBegin) -> Result<(), ProtocolError> {
        self.writer.write_u8(COMPACT_PROTOCOL_ID)?;
        self.writer.write_u8(COMPACT_VERSION | ((identifier.message_type << COMPACT_TYPE_SHIFT) & COMPACT_TYPE_MASK))?;
        self.write_varint32(identifier.seq_id)?;
        self.write_string(&identifier.name)?;
        Ok(())
    }

    fn write_message_end(&mut self) -> Result<(), ProtocolError> { Ok(()) }

    fn write_struct_begin(&mut self, _name: &str) -> Result<(), ProtocolError> {
        let last = self.last_field_id.last().copied().unwrap_or(0);
        self.last_field_id.push(last);
        Ok(())
    }

    fn write_struct_end(&mut self) -> Result<(), ProtocolError> {
        self.last_field_id.pop();
        Ok(())
    }

    fn write_field_begin(&mut self, field: &FieldBegin) -> Result<(), ProtocolError> {
        if field.field_type == TType::Bool {
            // Defer writing until write_bool — the value is encoded in the type nibble.
            self.bool_field_pending = Some(field.id);
            return Ok(());
        }
        self.write_field_begin_inner(field.field_type, field.id)?;
        Ok(())
    }

    fn write_field_end(&mut self) -> Result<(), ProtocolError> { Ok(()) }

    fn write_field_stop(&mut self) -> Result<(), ProtocolError> {
        self.writer.write_u8(CompactType::Stop as u8)?;
        Ok(())
    }

    fn write_bool(&mut self, value: bool) -> Result<(), ProtocolError> {
        if let Some(field_id) = self.bool_field_pending.take() {
            // Bool value is encoded directly in the field type nibble.
            let compact_type = if value { CompactType::BooleanTrue } else { CompactType::BooleanFalse };
            let last_id = *self.last_field_id.last().unwrap_or(&0);
            let delta = field_id - last_id;
            if delta > 0 && delta <= 15 {
                self.writer.write_u8(((delta as u8) << 4) | (compact_type as u8))?;
            } else {
                self.writer.write_u8(compact_type as u8)?;
                self.writer.write_i16::<BigEndian>(field_id)?;
            }
            *self.last_field_id.last_mut().unwrap() = field_id;
        } else {
            // Standalone bool (inside a list/set/map): encode as BooleanTrue/False byte.
            self.writer.write_u8(if value { CompactType::BooleanTrue as u8 } else { CompactType::BooleanFalse as u8 })?;
        }
        Ok(())
    }

    fn write_byte(&mut self, value: i8) -> Result<(), ProtocolError> { Ok(self.writer.write_i8(value)?) }
    fn write_i16(&mut self, value: i16) -> Result<(), ProtocolError> { self.write_varint32(value as i32) }
    fn write_i32(&mut self, value: i32) -> Result<(), ProtocolError> { self.write_varint32(value) }
    fn write_i64(&mut self, value: i64) -> Result<(), ProtocolError> { self.write_varint64(value) }
    fn write_double(&mut self, value: f64) -> Result<(), ProtocolError> { Ok(self.writer.write_f64::<BigEndian>(value)?) }
    fn write_string(&mut self, value: &str) -> Result<(), ProtocolError> {
        self.write_varint32(value.len() as i32)?;
        Ok(self.writer.write_all(value.as_bytes())?)
    }
    fn write_binary(&mut self, value: &[u8]) -> Result<(), ProtocolError> {
        self.write_varint32(value.len() as i32)?;
        Ok(self.writer.write_all(value)?)
    }
    fn write_map_begin(&mut self, identifier: &MapBegin) -> Result<(), ProtocolError> {
        if identifier.size == 0 {
            self.writer.write_u8(0)?;
        } else {
            self.write_varint32(identifier.size)?;
            let types = (CompactType::from_ttype(identifier.key_type) as u8) << 4 | (CompactType::from_ttype(identifier.value_type) as u8);
            self.writer.write_u8(types)?;
        }
        Ok(())
    }
    fn write_map_end(&mut self) -> Result<(), ProtocolError> { Ok(()) }
    fn write_list_begin(&mut self, identifier: &ListBegin) -> Result<(), ProtocolError> {
        if identifier.size <= 14 {
            self.writer.write_u8(((identifier.size as u8) << 4) | (CompactType::from_ttype(identifier.element_type) as u8))?;
        } else {
            self.writer.write_u8(0xf0 | (CompactType::from_ttype(identifier.element_type) as u8))?;
            self.write_varint32(identifier.size)?;
        }
        Ok(())
    }
    fn write_list_end(&mut self) -> Result<(), ProtocolError> { Ok(()) }
    fn write_set_begin(&mut self, identifier: &SetBegin) -> Result<(), ProtocolError> {
        self.write_list_begin(&ListBegin { element_type: identifier.element_type, size: identifier.size })
    }
    fn write_set_end(&mut self) -> Result<(), ProtocolError> { Ok(()) }
}

