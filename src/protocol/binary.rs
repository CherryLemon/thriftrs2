use super::types::*;
use byteorder::{BigEndian, ReadBytesExt, WriteBytesExt};
use std::io::{Read, Write, Result as IoResult, Cursor};
use thiserror::Error;

pub const MESSAGE_TYPE_CALL: u8 = 1;
pub const MESSAGE_TYPE_REPLY: u8 = 2;
pub const MESSAGE_TYPE_EXCEPTION: u8 = 3;
pub const MESSAGE_TYPE_ONEWAY: u8 = 4;

pub const THRIFT_VERSION_1: u32 = 0x80010000;

#[derive(Debug, Clone)]
pub struct MessageBegin {
    pub name: String,
    pub message_type: u8,
    pub seq_id: i32,
}

#[derive(Error, Debug)]
pub enum ProtocolError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Invalid type: {0}")]
    InvalidType(u8),
    #[error("Invalid field type")]
    InvalidFieldType,
}

pub struct BinaryProtocolReader<R: Read> {
    reader: R,
}

impl<R: Read> BinaryProtocolReader<R> {
    pub fn new(reader: R) -> Self {
        Self { reader }
    }

    #[inline]
    pub fn read_field_begin(&mut self) -> Result<FieldBegin, ProtocolError> {
        let field_type = self.reader.read_u8()?;
        let field_type = TType::from_u8(field_type).ok_or(ProtocolError::InvalidType(field_type))?;

        if field_type == TType::Stop {
            return Ok(FieldBegin {
                name: None,
                field_type,
                id: 0,
            });
        }

        let id = self.reader.read_i16::<BigEndian>()?;

        Ok(FieldBegin {
            name: None,
            field_type,
            id,
        })
    }

    #[inline]
    pub fn read_bool(&mut self) -> IoResult<bool> {
        Ok(self.reader.read_u8()? != 0)
    }

    #[inline]
    pub fn read_byte(&mut self) -> IoResult<i8> {
        Ok(self.reader.read_i8()?)
    }

    #[inline]
    pub fn read_i16(&mut self) -> IoResult<i16> {
        self.reader.read_i16::<BigEndian>()
    }

    #[inline]
    pub fn read_i32(&mut self) -> IoResult<i32> {
        self.reader.read_i32::<BigEndian>()
    }

    #[inline]
    pub fn read_i64(&mut self) -> IoResult<i64> {
        self.reader.read_i64::<BigEndian>()
    }

    #[inline]
    pub fn read_double(&mut self) -> IoResult<f64> {
        self.reader.read_f64::<BigEndian>()
    }

    #[inline]
    pub fn read_string(&mut self) -> IoResult<String> {
        let length = self.reader.read_i32::<BigEndian>()? as usize;
        let mut buffer = vec![0u8; length];
        self.reader.read_exact(&mut buffer)?;
        String::from_utf8(buffer).map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
    }

    #[inline]
    pub fn read_binary(&mut self) -> IoResult<Vec<u8>> {
        let length = self.reader.read_i32::<BigEndian>()? as usize;
        let mut buffer = vec![0u8; length];
        self.reader.read_exact(&mut buffer)?;
        Ok(buffer)
    }

    /// Raw reads for skip_value — no allocation, no Python involvement.
    #[inline]
    pub fn read_u8_raw(&mut self) -> std::io::Result<u8> {
        self.reader.read_u8()
    }

    #[inline]
    pub fn read_i16_raw(&mut self) -> std::io::Result<i16> {
        self.reader.read_i16::<BigEndian>()
    }

    #[inline]
    pub fn read_i32_raw(&mut self) -> std::io::Result<i32> {
        self.reader.read_i32::<BigEndian>()
    }

    #[inline]
    pub fn read_i64_raw(&mut self) -> std::io::Result<i64> {
        self.reader.read_i64::<BigEndian>()
    }

    pub fn read_map_begin(&mut self) -> Result<MapBegin, ProtocolError> {
        let key_type = TType::from_u8(self.reader.read_u8()?)
            .ok_or(ProtocolError::InvalidFieldType)?;
        let value_type = TType::from_u8(self.reader.read_u8()?)
            .ok_or(ProtocolError::InvalidFieldType)?;
        let size = self.reader.read_i32::<BigEndian>()?;

        Ok(MapBegin {
            key_type,
            value_type,
            size,
        })
    }

    pub fn read_list_begin(&mut self) -> Result<ListBegin, ProtocolError> {
        let element_type = TType::from_u8(self.reader.read_u8()?)
            .ok_or(ProtocolError::InvalidFieldType)?;
        let size = self.reader.read_i32::<BigEndian>()?;

        Ok(ListBegin {
            element_type,
            size,
        })
    }

    pub fn read_set_begin(&mut self) -> Result<SetBegin, ProtocolError> {
        let element_type = TType::from_u8(self.reader.read_u8()?)
            .ok_or(ProtocolError::InvalidFieldType)?;
        let size = self.reader.read_i32::<BigEndian>()?;

        Ok(SetBegin {
            element_type,
            size,
        })
    }

    pub fn read_message_begin(&mut self) -> Result<MessageBegin, ProtocolError> {
        let version_and_type = self.reader.read_u32::<BigEndian>()?;
        if version_and_type & 0xffff0000 != THRIFT_VERSION_1 {
            return Err(ProtocolError::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("Bad version: 0x{:08x}", version_and_type),
            )));
        }
        let message_type = (version_and_type & 0x000000ff) as u8;
        let name_len = self.reader.read_i32::<BigEndian>()? as usize;
        let mut name_buf = vec![0u8; name_len];
        self.reader.read_exact(&mut name_buf)?;
        let name = String::from_utf8(name_buf)
            .map_err(|e| ProtocolError::Io(std::io::Error::new(std::io::ErrorKind::InvalidData, e)))?;
        let seq_id = self.reader.read_i32::<BigEndian>()?;
        Ok(MessageBegin { name, message_type, seq_id })
    }
}

pub struct BinaryProtocolWriter<W: Write> {
    writer: W,
}

impl<W: Write> BinaryProtocolWriter<W> {
    pub fn new(writer: W) -> Self {
        Self { writer }
    }

    pub fn write_field_begin(&mut self, field: &FieldBegin) -> IoResult<()> {
        self.writer.write_u8(field.field_type as u8)?;
        if field.field_type != TType::Stop {
            self.writer.write_i16::<BigEndian>(field.id)?;
        }
        Ok(())
    }

    pub fn write_field_stop(&mut self) -> IoResult<()> {
        self.writer.write_u8(TType::Stop as u8)
    }

    pub fn write_bool(&mut self, value: bool) -> IoResult<()> {
        self.writer.write_u8(if value { 1 } else { 0 })
    }

    pub fn write_byte(&mut self, value: i8) -> IoResult<()> {
        self.writer.write_i8(value)
    }

    pub fn write_i16(&mut self, value: i16) -> IoResult<()> {
        self.writer.write_i16::<BigEndian>(value)
    }

    pub fn write_i32(&mut self, value: i32) -> IoResult<()> {
        self.writer.write_i32::<BigEndian>(value)
    }

    pub fn write_i64(&mut self, value: i64) -> IoResult<()> {
        self.writer.write_i64::<BigEndian>(value)
    }

    pub fn write_double(&mut self, value: f64) -> IoResult<()> {
        self.writer.write_f64::<BigEndian>(value)
    }

    pub fn write_string(&mut self, value: &str) -> IoResult<()> {
        let bytes = value.as_bytes();
        self.writer.write_i32::<BigEndian>(bytes.len() as i32)?;
        self.writer.write_all(bytes)
    }

    pub fn write_binary(&mut self, value: &[u8]) -> IoResult<()> {
        self.writer.write_i32::<BigEndian>(value.len() as i32)?;
        self.writer.write_all(value)
    }

    pub fn write_map_begin(&mut self, map: &MapBegin) -> IoResult<()> {
        self.writer.write_u8(map.key_type as u8)?;
        self.writer.write_u8(map.value_type as u8)?;
        self.writer.write_i32::<BigEndian>(map.size)
    }

    pub fn write_list_begin(&mut self, list: &ListBegin) -> IoResult<()> {
        self.writer.write_u8(list.element_type as u8)?;
        self.writer.write_i32::<BigEndian>(list.size)
    }

    pub fn write_set_begin(&mut self, set: &SetBegin) -> IoResult<()> {
        self.writer.write_u8(set.element_type as u8)?;
        self.writer.write_i32::<BigEndian>(set.size)
    }

    pub fn write_map_end(&mut self) -> IoResult<()> {
        Ok(())
    }

    pub fn write_list_end(&mut self) -> IoResult<()> {
        Ok(())
    }

    pub fn write_set_end(&mut self) -> IoResult<()> {
        Ok(())
    }

    pub fn flush(&mut self) -> IoResult<()> {
        self.writer.flush()
    }

    pub fn write_message_begin(&mut self, name: &str, message_type: u8, seq_id: i32) -> IoResult<()> {
        let version_and_type = THRIFT_VERSION_1 | (message_type as u32);
        self.writer.write_u32::<BigEndian>(version_and_type)?;
        let name_bytes = name.as_bytes();
        self.writer.write_i32::<BigEndian>(name_bytes.len() as i32)?;
        self.writer.write_all(name_bytes)?;
        self.writer.write_i32::<BigEndian>(seq_id)?;
        Ok(())
    }
}
