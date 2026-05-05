use thiserror::Error;

// ──────────────────────────────────────────────────────────────────────────────
// Wire type tags
// ──────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TType {
    Stop = 0,
    Void = 1,
    Bool = 2,
    Byte = 3,
    Double = 4,
    I16 = 6,
    I32 = 8,
    I64 = 10,
    String = 11,
    Struct = 12,
    Map = 13,
    Set = 14,
    List = 15,
}

impl TType {
    pub fn from_u8(value: u8) -> Option<Self> {
        match value {
            0 => Some(TType::Stop),
            1 => Some(TType::Void),
            2 => Some(TType::Bool),
            3 => Some(TType::Byte),
            4 => Some(TType::Double),
            6 => Some(TType::I16),
            8 => Some(TType::I32),
            10 => Some(TType::I64),
            11 => Some(TType::String),
            12 => Some(TType::Struct),
            13 => Some(TType::Map),
            14 => Some(TType::Set),
            15 => Some(TType::List),
            _ => None,
        }
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// Shared message / field / container descriptors
// ──────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct MessageBegin {
    pub name: String,
    pub message_type: u8,
    pub seq_id: i32,
}

#[derive(Debug, Clone)]
pub struct FieldBegin {
    #[allow(dead_code)]
    pub name: Option<String>,
    pub field_type: TType,
    pub id: i16,
}

#[derive(Debug, Clone)]
pub struct MapBegin {
    pub key_type: TType,
    pub value_type: TType,
    pub size: i32,
}

#[derive(Debug, Clone)]
pub struct ListBegin {
    pub element_type: TType,
    pub size: i32,
}

#[derive(Debug, Clone)]
pub struct SetBegin {
    pub element_type: TType,
    pub size: i32,
}

// ──────────────────────────────────────────────────────────────────────────────
// Error type
// ──────────────────────────────────────────────────────────────────────────────

#[derive(Error, Debug)]
pub enum ProtocolError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Invalid type: {0}")]
    InvalidType(u8),
    #[error("Invalid field type")]
    InvalidFieldType,
}

impl From<ProtocolError> for std::io::Error {
    fn from(e: ProtocolError) -> Self {
        match e {
            ProtocolError::Io(io_err) => io_err,
            other => std::io::Error::new(std::io::ErrorKind::InvalidData, other.to_string()),
        }
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// Protocol traits
// ──────────────────────────────────────────────────────────────────────────────

pub trait TInputProtocol {
    fn read_message_begin(&mut self) -> Result<MessageBegin, ProtocolError>;
    #[allow(dead_code)]
    fn read_message_end(&mut self) -> Result<(), ProtocolError>;
    fn read_struct_begin(&mut self) -> Result<(), ProtocolError>;
    fn read_struct_end(&mut self) -> Result<(), ProtocolError>;
    fn read_field_begin(&mut self) -> Result<FieldBegin, ProtocolError>;
    fn read_field_end(&mut self) -> Result<(), ProtocolError>;
    fn read_bool(&mut self) -> Result<bool, ProtocolError>;
    fn read_byte(&mut self) -> Result<i8, ProtocolError>;
    fn read_i16(&mut self) -> Result<i16, ProtocolError>;
    fn read_i32(&mut self) -> Result<i32, ProtocolError>;
    fn read_i64(&mut self) -> Result<i64, ProtocolError>;
    fn read_double(&mut self) -> Result<f64, ProtocolError>;
    fn read_string(&mut self) -> Result<String, ProtocolError>;
    fn read_binary(&mut self) -> Result<Vec<u8>, ProtocolError>;
    fn read_map_begin(&mut self) -> Result<MapBegin, ProtocolError>;
    fn read_map_end(&mut self) -> Result<(), ProtocolError>;
    fn read_list_begin(&mut self) -> Result<ListBegin, ProtocolError>;
    fn read_list_end(&mut self) -> Result<(), ProtocolError>;
    fn read_set_begin(&mut self) -> Result<SetBegin, ProtocolError>;
    fn read_set_end(&mut self) -> Result<(), ProtocolError>;
}

pub trait TOutputProtocol {
    fn write_message_begin(&mut self, identifier: &MessageBegin) -> Result<(), ProtocolError>;
    #[allow(dead_code)]
    fn write_message_end(&mut self) -> Result<(), ProtocolError>;
    fn write_struct_begin(&mut self, name: &str) -> Result<(), ProtocolError>;
    fn write_struct_end(&mut self) -> Result<(), ProtocolError>;
    fn write_field_begin(&mut self, field: &FieldBegin) -> Result<(), ProtocolError>;
    fn write_field_end(&mut self) -> Result<(), ProtocolError>;
    fn write_field_stop(&mut self) -> Result<(), ProtocolError>;
    fn write_bool(&mut self, value: bool) -> Result<(), ProtocolError>;
    fn write_byte(&mut self, value: i8) -> Result<(), ProtocolError>;
    fn write_i16(&mut self, value: i16) -> Result<(), ProtocolError>;
    fn write_i32(&mut self, value: i32) -> Result<(), ProtocolError>;
    fn write_i64(&mut self, value: i64) -> Result<(), ProtocolError>;
    fn write_double(&mut self, value: f64) -> Result<(), ProtocolError>;
    fn write_string(&mut self, value: &str) -> Result<(), ProtocolError>;
    fn write_binary(&mut self, value: &[u8]) -> Result<(), ProtocolError>;
    fn write_map_begin(&mut self, identifier: &MapBegin) -> Result<(), ProtocolError>;
    fn write_map_end(&mut self) -> Result<(), ProtocolError>;
    fn write_list_begin(&mut self, identifier: &ListBegin) -> Result<(), ProtocolError>;
    #[allow(dead_code)]
    fn write_list_end(&mut self) -> Result<(), ProtocolError>;
    fn write_set_begin(&mut self, identifier: &SetBegin) -> Result<(), ProtocolError>;
    #[allow(dead_code)]
    fn write_set_end(&mut self) -> Result<(), ProtocolError>;
}
