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

#[derive(Debug, Clone)]
pub struct FieldBegin {
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
