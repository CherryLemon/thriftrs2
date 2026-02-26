use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ThriftType {
    Bool,
    Byte,
    I16,
    I32,
    I64,
    Double,
    String,
    Binary,
    List(Box<ThriftType>),
    Set(Box<ThriftType>),
    Map(Box<ThriftType>, Box<ThriftType>),
    Struct(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThriftField {
    pub id: i16,
    pub name: String,
    pub field_type: ThriftType,
    pub required: bool,
    pub default_value: Option<ThriftValue>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThriftStruct {
    pub name: String,
    pub fields: Vec<ThriftField>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThriftService {
    pub name: String,
    pub methods: Vec<ThriftMethod>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThriftMethod {
    pub name: String,
    pub return_type: ThriftType,
    pub arguments: Vec<ThriftField>,
    pub exceptions: Vec<ThriftField>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ThriftValue {
    Bool(bool),
    Byte(i8),
    I16(i16),
    I32(i32),
    I64(i64),
    Double(f64),
    String(String),
    Binary(Vec<u8>),
    List(Vec<ThriftValue>),
    Set(Vec<ThriftValue>),
    Map(Vec<(ThriftValue, ThriftValue)>),
    Struct(HashMap<String, ThriftValue>),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThriftDocument {
    pub structs: HashMap<String, ThriftStruct>,
    pub services: HashMap<String, ThriftService>,
    pub includes: Vec<String>,
    pub namespaces: HashMap<String, String>,
}
