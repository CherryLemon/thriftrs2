use super::types::*;
use byteorder::{BigEndian, ReadBytesExt, WriteBytesExt};
use std::io::{Read, Write};

pub const MESSAGE_TYPE_CALL: u8 = 1;
pub const MESSAGE_TYPE_REPLY: u8 = 2;
pub const MESSAGE_TYPE_EXCEPTION: u8 = 3;
#[allow(dead_code)]
pub const MESSAGE_TYPE_ONEWAY: u8 = 4;

pub const THRIFT_VERSION_1: u32 = 0x80010000;

pub struct BinaryProtocolReader<R: Read> {
    reader: R,
}

impl<R: Read> BinaryProtocolReader<R> {
    pub fn new(reader: R) -> Self {
        Self { reader }
    }
}

impl<R: Read> TInputProtocol for BinaryProtocolReader<R> {
    fn read_message_begin(&mut self) -> Result<MessageBegin, ProtocolError> {
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
        let name = String::from_utf8(name_buf).map_err(|e| {
            ProtocolError::Io(std::io::Error::new(std::io::ErrorKind::InvalidData, e))
        })?;
        let seq_id = self.reader.read_i32::<BigEndian>()?;
        Ok(MessageBegin {
            name,
            message_type,
            seq_id,
        })
    }

    fn read_message_end(&mut self) -> Result<(), ProtocolError> {
        Ok(())
    }

    fn read_struct_begin(&mut self) -> Result<(), ProtocolError> {
        Ok(())
    }

    fn read_struct_end(&mut self) -> Result<(), ProtocolError> {
        Ok(())
    }

    fn read_field_begin(&mut self) -> Result<FieldBegin, ProtocolError> {
        let field_type_byte = self.reader.read_u8()?;
        let field_type =
            TType::from_u8(field_type_byte).ok_or(ProtocolError::InvalidType(field_type_byte))?;

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

    fn read_field_end(&mut self) -> Result<(), ProtocolError> {
        Ok(())
    }

    fn read_bool(&mut self) -> Result<bool, ProtocolError> {
        Ok(self.reader.read_u8()? != 0)
    }

    fn read_byte(&mut self) -> Result<i8, ProtocolError> {
        Ok(self.reader.read_i8()?)
    }

    fn read_i16(&mut self) -> Result<i16, ProtocolError> {
        Ok(self.reader.read_i16::<BigEndian>()?)
    }

    fn read_i32(&mut self) -> Result<i32, ProtocolError> {
        Ok(self.reader.read_i32::<BigEndian>()?)
    }

    fn read_i64(&mut self) -> Result<i64, ProtocolError> {
        Ok(self.reader.read_i64::<BigEndian>()?)
    }

    fn read_double(&mut self) -> Result<f64, ProtocolError> {
        Ok(self.reader.read_f64::<BigEndian>()?)
    }

    fn read_string(&mut self) -> Result<String, ProtocolError> {
        let length = self.reader.read_i32::<BigEndian>()? as usize;
        let mut buffer = vec![0u8; length];
        self.reader.read_exact(&mut buffer)?;
        String::from_utf8(buffer)
            .map_err(|e| ProtocolError::Io(std::io::Error::new(std::io::ErrorKind::InvalidData, e)))
    }

    fn read_binary(&mut self) -> Result<Vec<u8>, ProtocolError> {
        let length = self.reader.read_i32::<BigEndian>()? as usize;
        let mut buffer = vec![0u8; length];
        self.reader.read_exact(&mut buffer)?;
        Ok(buffer)
    }

    fn read_map_begin(&mut self) -> Result<MapBegin, ProtocolError> {
        let key_type =
            TType::from_u8(self.reader.read_u8()?).ok_or(ProtocolError::InvalidFieldType)?;
        let value_type =
            TType::from_u8(self.reader.read_u8()?).ok_or(ProtocolError::InvalidFieldType)?;
        let size = self.reader.read_i32::<BigEndian>()?;

        Ok(MapBegin {
            key_type,
            value_type,
            size,
        })
    }

    fn read_map_end(&mut self) -> Result<(), ProtocolError> {
        Ok(())
    }

    fn read_list_begin(&mut self) -> Result<ListBegin, ProtocolError> {
        let element_type =
            TType::from_u8(self.reader.read_u8()?).ok_or(ProtocolError::InvalidFieldType)?;
        let size = self.reader.read_i32::<BigEndian>()?;

        Ok(ListBegin { element_type, size })
    }

    fn read_list_end(&mut self) -> Result<(), ProtocolError> {
        Ok(())
    }

    fn read_set_begin(&mut self) -> Result<SetBegin, ProtocolError> {
        let element_type =
            TType::from_u8(self.reader.read_u8()?).ok_or(ProtocolError::InvalidFieldType)?;
        let size = self.reader.read_i32::<BigEndian>()?;

        Ok(SetBegin { element_type, size })
    }

    fn read_set_end(&mut self) -> Result<(), ProtocolError> {
        Ok(())
    }
}

pub struct BinaryProtocolWriter<W: Write> {
    writer: W,
}

impl<W: Write> BinaryProtocolWriter<W> {
    pub fn new(writer: W) -> Self {
        Self { writer }
    }
}

impl<W: Write> TOutputProtocol for BinaryProtocolWriter<W> {
    fn write_message_begin(&mut self, identifier: &MessageBegin) -> Result<(), ProtocolError> {
        let version_and_type = THRIFT_VERSION_1 | (identifier.message_type as u32);
        self.writer.write_u32::<BigEndian>(version_and_type)?;
        self.writer
            .write_i32::<BigEndian>(identifier.name.len() as i32)?;
        self.writer.write_all(identifier.name.as_bytes())?;
        self.writer.write_i32::<BigEndian>(identifier.seq_id)?;
        Ok(())
    }

    fn write_message_end(&mut self) -> Result<(), ProtocolError> {
        Ok(())
    }

    fn write_struct_begin(&mut self, _name: &str) -> Result<(), ProtocolError> {
        Ok(())
    }

    fn write_struct_end(&mut self) -> Result<(), ProtocolError> {
        Ok(())
    }

    fn write_field_begin(&mut self, field: &FieldBegin) -> Result<(), ProtocolError> {
        self.writer.write_u8(field.field_type as u8)?;
        if field.field_type != TType::Stop {
            self.writer.write_i16::<BigEndian>(field.id)?;
        }
        Ok(())
    }

    fn write_field_end(&mut self) -> Result<(), ProtocolError> {
        Ok(())
    }

    fn write_field_stop(&mut self) -> Result<(), ProtocolError> {
        self.writer.write_u8(TType::Stop as u8)?;
        Ok(())
    }

    fn write_bool(&mut self, value: bool) -> Result<(), ProtocolError> {
        self.writer.write_u8(if value { 1 } else { 0 })?;
        Ok(())
    }

    fn write_byte(&mut self, value: i8) -> Result<(), ProtocolError> {
        self.writer.write_i8(value)?;
        Ok(())
    }

    fn write_i16(&mut self, value: i16) -> Result<(), ProtocolError> {
        self.writer.write_i16::<BigEndian>(value)?;
        Ok(())
    }

    fn write_i32(&mut self, value: i32) -> Result<(), ProtocolError> {
        self.writer.write_i32::<BigEndian>(value)?;
        Ok(())
    }

    fn write_i64(&mut self, value: i64) -> Result<(), ProtocolError> {
        self.writer.write_i64::<BigEndian>(value)?;
        Ok(())
    }

    fn write_double(&mut self, value: f64) -> Result<(), ProtocolError> {
        self.writer.write_f64::<BigEndian>(value)?;
        Ok(())
    }

    fn write_string(&mut self, value: &str) -> Result<(), ProtocolError> {
        self.writer.write_i32::<BigEndian>(value.len() as i32)?;
        self.writer.write_all(value.as_bytes())?;
        Ok(())
    }

    fn write_binary(&mut self, value: &[u8]) -> Result<(), ProtocolError> {
        self.writer.write_i32::<BigEndian>(value.len() as i32)?;
        self.writer.write_all(value)?;
        Ok(())
    }

    fn write_map_begin(&mut self, identifier: &MapBegin) -> Result<(), ProtocolError> {
        self.writer.write_u8(identifier.key_type as u8)?;
        self.writer.write_u8(identifier.value_type as u8)?;
        self.writer.write_i32::<BigEndian>(identifier.size)?;
        Ok(())
    }

    fn write_map_end(&mut self) -> Result<(), ProtocolError> {
        Ok(())
    }

    fn write_list_begin(&mut self, identifier: &ListBegin) -> Result<(), ProtocolError> {
        self.writer.write_u8(identifier.element_type as u8)?;
        self.writer.write_i32::<BigEndian>(identifier.size)?;
        Ok(())
    }

    fn write_list_end(&mut self) -> Result<(), ProtocolError> {
        Ok(())
    }

    fn write_set_begin(&mut self, identifier: &SetBegin) -> Result<(), ProtocolError> {
        self.writer.write_u8(identifier.element_type as u8)?;
        self.writer.write_i32::<BigEndian>(identifier.size)?;
        Ok(())
    }

    fn write_set_end(&mut self) -> Result<(), ProtocolError> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn round_trip<T>(
        write: impl FnOnce(&mut BinaryProtocolWriter<&mut Vec<u8>>),
        read: impl FnOnce(&mut BinaryProtocolReader<Cursor<Vec<u8>>>) -> T,
    ) -> T {
        let mut bytes = Vec::new();
        write(&mut BinaryProtocolWriter::new(&mut bytes));
        read(&mut BinaryProtocolReader::new(Cursor::new(bytes)))
    }

    #[test]
    fn round_trips_message_begin() {
        let mut bytes = Vec::new();
        let mut writer = BinaryProtocolWriter::new(&mut bytes);
        writer
            .write_message_begin(&MessageBegin {
                name: "ping".to_string(),
                message_type: MESSAGE_TYPE_CALL,
                seq_id: 42,
            })
            .unwrap();
        let mut reader = BinaryProtocolReader::new(Cursor::new(bytes));
        let msg = reader.read_message_begin().unwrap();
        assert_eq!(msg.name, "ping");
        assert_eq!(msg.message_type, MESSAGE_TYPE_CALL);
        assert_eq!(msg.seq_id, 42);
    }

    #[test]
    fn rejects_bad_message_version() {
        let mut reader = BinaryProtocolReader::new(Cursor::new(vec![0, 0, 0, 1]));
        assert!(reader.read_message_begin().is_err());
    }

    #[test]
    fn round_trips_bool() {
        assert!(round_trip(
            |writer| writer.write_bool(true).unwrap(),
            |reader| reader.read_bool().unwrap()
        ));
    }

    #[test]
    fn round_trips_numeric_values() {
        let mut bytes = Vec::new();
        let mut writer = BinaryProtocolWriter::new(&mut bytes);
        writer.write_byte(-12).unwrap();
        writer.write_i16(-1234).unwrap();
        writer.write_i32(123456).unwrap();
        writer.write_i64(9_000_000_000).unwrap();
        writer.write_double(1.25).unwrap();
        let mut reader = BinaryProtocolReader::new(Cursor::new(bytes));
        assert_eq!(reader.read_byte().unwrap(), -12);
        assert_eq!(reader.read_i16().unwrap(), -1234);
        assert_eq!(reader.read_i32().unwrap(), 123456);
        assert_eq!(reader.read_i64().unwrap(), 9_000_000_000);
        assert_eq!(reader.read_double().unwrap(), 1.25);
    }

    #[test]
    fn round_trips_string() {
        let value = round_trip(
            |writer| writer.write_string("hello").unwrap(),
            |reader| reader.read_string().unwrap(),
        );
        assert_eq!(value, "hello");
    }

    #[test]
    fn round_trips_binary() {
        let value = round_trip(
            |writer| writer.write_binary(&[0, 1, 2, 255]).unwrap(),
            |reader| reader.read_binary().unwrap(),
        );
        assert_eq!(value, vec![0, 1, 2, 255]);
    }

    #[test]
    fn round_trips_field_header_and_stop() {
        let mut bytes = Vec::new();
        let mut writer = BinaryProtocolWriter::new(&mut bytes);
        writer
            .write_field_begin(&FieldBegin {
                name: None,
                field_type: TType::I32,
                id: 7,
            })
            .unwrap();
        writer.write_field_stop().unwrap();
        let mut reader = BinaryProtocolReader::new(Cursor::new(bytes));
        let field = reader.read_field_begin().unwrap();
        assert_eq!(field.field_type, TType::I32);
        assert_eq!(field.id, 7);
        assert_eq!(reader.read_field_begin().unwrap().field_type, TType::Stop);
    }

    #[test]
    fn round_trips_list_header() {
        let mut bytes = Vec::new();
        let mut writer = BinaryProtocolWriter::new(&mut bytes);
        writer
            .write_list_begin(&ListBegin {
                element_type: TType::I16,
                size: 3,
            })
            .unwrap();
        let mut reader = BinaryProtocolReader::new(Cursor::new(bytes));
        let list = reader.read_list_begin().unwrap();
        assert_eq!(list.element_type, TType::I16);
        assert_eq!(list.size, 3);
    }

    #[test]
    fn round_trips_set_header() {
        let mut bytes = Vec::new();
        let mut writer = BinaryProtocolWriter::new(&mut bytes);
        writer
            .write_set_begin(&SetBegin {
                element_type: TType::String,
                size: 2,
            })
            .unwrap();
        let mut reader = BinaryProtocolReader::new(Cursor::new(bytes));
        let set = reader.read_set_begin().unwrap();
        assert_eq!(set.element_type, TType::String);
        assert_eq!(set.size, 2);
    }

    #[test]
    fn round_trips_map_header() {
        let mut bytes = Vec::new();
        let mut writer = BinaryProtocolWriter::new(&mut bytes);
        writer
            .write_map_begin(&MapBegin {
                key_type: TType::String,
                value_type: TType::I64,
                size: 1,
            })
            .unwrap();
        let mut reader = BinaryProtocolReader::new(Cursor::new(bytes));
        let map = reader.read_map_begin().unwrap();
        assert_eq!(map.key_type, TType::String);
        assert_eq!(map.value_type, TType::I64);
        assert_eq!(map.size, 1);
    }
}
