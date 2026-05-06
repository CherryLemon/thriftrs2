pub mod ast;
pub mod lexer;

use crate::parser::ast::*;
use crate::parser::lexer::*;
use std::collections::HashMap;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum ParseError {
    #[error("Syntax error: {0}")]
    SyntaxError(String),
    #[error("Unexpected token: {0:?}")]
    UnexpectedToken(Token),
    #[error("End of input")]
    EndOfInput,
}

pub struct Parser {
    tokens: Vec<Token>,
    position: usize,
    type_aliases: HashMap<String, ThriftType>,
    enum_values: HashMap<String, HashMap<String, i32>>,
    parsed_structs: HashMap<String, ThriftStruct>,
    parsed_services: HashMap<String, ThriftService>,
}

impl Parser {
    pub fn new(input: &str) -> Result<Self, ParseError> {
        let (remaining, tokens) = tokenize(input)
            .map_err(|e| ParseError::SyntaxError(format!("Tokenization failed: {:?}", e)))?;

        // Skip any trailing whitespace/comments then check for unconsumed input
        let remaining = remaining.trim();
        if !remaining.is_empty() {
            return Err(ParseError::SyntaxError(format!(
                "Unexpected input: {:?}",
                &remaining[..remaining.len().min(40)]
            )));
        }

        Ok(Parser {
            tokens,
            position: 0,
            type_aliases: HashMap::new(),
            enum_values: HashMap::new(),
            parsed_structs: HashMap::new(),
            parsed_services: HashMap::new(),
        })
    }

    fn current_token(&self) -> Result<&Token, ParseError> {
        self.tokens.get(self.position).ok_or(ParseError::EndOfInput)
    }

    fn consume_token(&mut self) -> Result<Token, ParseError> {
        if self.position < self.tokens.len() {
            let token = self.tokens[self.position].clone();
            self.position += 1;
            Ok(token)
        } else {
            Err(ParseError::EndOfInput)
        }
    }

    fn expect_token(&mut self, expected: Token) -> Result<(), ParseError> {
        let token = self.consume_token()?;
        if std::mem::discriminant(&token) == std::mem::discriminant(&expected) {
            Ok(())
        } else {
            Err(ParseError::UnexpectedToken(token))
        }
    }

    pub fn parse_document(&mut self) -> Result<ThriftDocument, ParseError> {
        let mut document = ThriftDocument {
            structs: HashMap::new(),
            services: HashMap::new(),
            enums: HashMap::new(),
            includes: Vec::new(),
            namespaces: HashMap::new(),
        };

        while self.position < self.tokens.len() {
            match self.current_token()? {
                Token::Keyword(keyword) if keyword == "struct" => {
                    let struct_def = self.parse_struct("struct")?;
                    self.parsed_structs
                        .insert(struct_def.name.clone(), struct_def.clone());
                    document.structs.insert(struct_def.name.clone(), struct_def);
                }
                Token::Keyword(keyword) if keyword == "exception" => {
                    let struct_def = self.parse_struct("exception")?;
                    self.parsed_structs
                        .insert(struct_def.name.clone(), struct_def.clone());
                    document.structs.insert(struct_def.name.clone(), struct_def);
                }
                Token::Keyword(keyword) if keyword == "union" => {
                    let struct_def = self.parse_struct("union")?;
                    self.parsed_structs
                        .insert(struct_def.name.clone(), struct_def.clone());
                    document.structs.insert(struct_def.name.clone(), struct_def);
                }
                Token::Keyword(keyword) if keyword == "service" => {
                    let service_def = self.parse_service()?;
                    self.parsed_services
                        .insert(service_def.name.clone(), service_def.clone());
                    document
                        .services
                        .insert(service_def.name.clone(), service_def);
                }
                Token::Keyword(keyword) if keyword == "include" => {
                    let include = self.parse_include()?;
                    document.includes.push(include);
                }
                Token::Keyword(keyword) if keyword == "namespace" => {
                    let (scope, namespace) = self.parse_namespace()?;
                    document.namespaces.insert(scope, namespace);
                }
                Token::Keyword(keyword) if keyword == "typedef" => {
                    let (alias, aliased_type) = self.parse_typedef()?;
                    self.type_aliases.insert(alias, aliased_type);
                }
                Token::Keyword(keyword) if keyword == "enum" => {
                    let enum_def = self.parse_enum()?;
                    self.type_aliases
                        .insert(enum_def.name.clone(), ThriftType::I32);
                    self.enum_values
                        .insert(enum_def.name.clone(), enum_def.variants.clone());
                    document.enums.insert(enum_def.name.clone(), enum_def);
                }
                Token::Keyword(keyword) if keyword == "const" => {
                    self.parse_const()?;
                }
                _ => {
                    self.consume_token()?; // Skip unknown tokens
                }
            }
        }

        Ok(document)
    }

    fn parse_struct(&mut self, keyword: &str) -> Result<ThriftStruct, ParseError> {
        self.expect_token(Token::Keyword(keyword.to_string()))?;

        let name = match self.consume_token()? {
            Token::Identifier(name) => name,
            token => return Err(ParseError::UnexpectedToken(token)),
        };

        let extends = self.consume_optional_extends()?;
        self.skip_annotations()?;

        self.expect_token(Token::LeftBrace)?;

        let mut fields = Vec::new();
        while let Ok(token) = self.current_token() {
            if let Token::RightBrace = token {
                break;
            }
            fields.push(self.parse_field()?);
        }

        self.expect_token(Token::RightBrace)?;
        self.skip_annotations()?;

        let fields = self.merge_parent_fields(extends.as_deref(), fields)?;
        Ok(ThriftStruct { name, fields })
    }

    fn parse_field(&mut self) -> Result<ThriftField, ParseError> {
        let id = match self.consume_token()? {
            Token::Number(id) => id as i16,
            token => return Err(ParseError::UnexpectedToken(token)),
        };

        self.expect_token(Token::Colon)?;

        let required = match self.current_token()? {
            Token::Keyword(keyword) if keyword == "required" => {
                self.consume_token()?;
                true
            }
            Token::Keyword(keyword) if keyword == "optional" => {
                self.consume_token()?;
                false
            }
            _ => false,
        };

        let field_type = self.parse_type()?;
        self.skip_annotations()?;

        let name = match self.consume_token()? {
            Token::Identifier(name) => name,
            token => return Err(ParseError::UnexpectedToken(token)),
        };

        let mut default_value = None;
        if let Ok(Token::Equal) = self.current_token() {
            self.consume_token()?; // consume '='
            default_value = match self.parse_default_value(&field_type) {
                Ok(value) => Some(value),
                Err(_) => {
                    self.skip_default_value()?;
                    None
                }
            };
        }

        self.skip_annotations()?;

        if matches!(self.current_token(), Ok(Token::Semicolon | Token::Comma)) {
            self.consume_token()?;
        }

        Ok(ThriftField {
            id,
            name,
            field_type,
            required,
            default_value,
        })
    }

    fn parse_type(&mut self) -> Result<ThriftType, ParseError> {
        match self.consume_token()? {
            Token::Identifier(type_name) => match type_name.as_str() {
                "bool" => Ok(ThriftType::Bool),
                "byte" => Ok(ThriftType::Byte),
                "i16" => Ok(ThriftType::I16),
                "i32" => Ok(ThriftType::I32),
                "i64" => Ok(ThriftType::I64),
                "double" => Ok(ThriftType::Double),
                "string" => Ok(ThriftType::String),
                "binary" => Ok(ThriftType::Binary),
                "list" => {
                    self.expect_token(Token::LessThan)?;
                    let element_type = self.parse_type()?;
                    self.expect_token(Token::GreaterThan)?;
                    Ok(ThriftType::List(Box::new(element_type)))
                }
                "set" => {
                    self.expect_token(Token::LessThan)?;
                    let element_type = self.parse_type()?;
                    self.expect_token(Token::GreaterThan)?;
                    Ok(ThriftType::Set(Box::new(element_type)))
                }
                "map" => {
                    self.expect_token(Token::LessThan)?;
                    let key_type = self.parse_type()?;
                    self.expect_token(Token::Comma)?;
                    let value_type = self.parse_type()?;
                    self.expect_token(Token::GreaterThan)?;
                    Ok(ThriftType::Map(Box::new(key_type), Box::new(value_type)))
                }
                _ => {
                    let type_name = self.finish_qualified_identifier(type_name)?;
                    let local_type_name = type_name.rsplit('.').next().unwrap_or(&type_name);
                    if let Some(alias) = self
                        .type_aliases
                        .get(&type_name)
                        .or_else(|| self.type_aliases.get(local_type_name))
                    {
                        Ok(alias.clone())
                    } else {
                        Ok(ThriftType::Struct(type_name))
                    }
                }
            },
            token => Err(ParseError::UnexpectedToken(token)),
        }
    }

    fn parse_service(&mut self) -> Result<ThriftService, ParseError> {
        self.expect_token(Token::Keyword("service".to_string()))?;

        let name = match self.consume_token()? {
            Token::Identifier(name) => name,
            token => return Err(ParseError::UnexpectedToken(token)),
        };

        let extends = self.consume_optional_extends()?;
        self.skip_annotations()?;

        self.expect_token(Token::LeftBrace)?;

        let mut methods = Vec::new();
        while let Ok(token) = self.current_token() {
            if let Token::RightBrace = token {
                break;
            }
            methods.push(self.parse_method()?);
        }

        self.expect_token(Token::RightBrace)?;
        self.skip_annotations()?;

        let methods = self.merge_parent_methods(extends.as_deref(), methods)?;
        Ok(ThriftService { name, methods })
    }

    fn parse_method(&mut self) -> Result<ThriftMethod, ParseError> {
        let oneway = match self.current_token()? {
            Token::Keyword(keyword) if keyword == "oneway" => {
                self.consume_token()?;
                true
            }
            _ => false,
        };

        let return_type = self.parse_type()?;

        let name = match self.consume_token()? {
            Token::Identifier(name) => name,
            token => return Err(ParseError::UnexpectedToken(token)),
        };

        self.expect_token(Token::LeftParen)?;

        let mut arguments = Vec::new();
        while let Ok(token) = self.current_token() {
            if let Token::RightParen = token {
                break;
            }
            arguments.push(self.parse_field()?);
            if let Ok(Token::Comma) = self.current_token() {
                self.consume_token()?;
            }
        }

        self.expect_token(Token::RightParen)?;

        let exceptions = if matches!(self.current_token(), Ok(Token::Keyword(keyword)) if keyword == "throws")
        {
            self.consume_token()?;
            self.expect_token(Token::LeftParen)?;
            let mut exceptions = Vec::new();
            while let Ok(token) = self.current_token() {
                if let Token::RightParen = token {
                    break;
                }
                exceptions.push(self.parse_field()?);
                if let Ok(Token::Comma) = self.current_token() {
                    self.consume_token()?;
                }
            }
            self.expect_token(Token::RightParen)?;
            exceptions
        } else {
            Vec::new()
        };

        self.skip_annotations()?;

        if matches!(self.current_token(), Ok(Token::Semicolon | Token::Comma)) {
            self.consume_token()?;
        }

        Ok(ThriftMethod {
            name,
            return_type,
            arguments,
            exceptions,
            oneway,
        })
    }

    fn parse_include(&mut self) -> Result<String, ParseError> {
        self.expect_token(Token::Keyword("include".to_string()))?;
        let include = match self.consume_token()? {
            Token::String(include) => include,
            token => return Err(ParseError::UnexpectedToken(token)),
        };
        if let Ok(Token::Semicolon) = self.current_token() {
            self.consume_token()?;
        }
        Ok(include)
    }

    fn parse_namespace(&mut self) -> Result<(String, String), ParseError> {
        self.expect_token(Token::Keyword("namespace".to_string()))?;
        let scope = match self.consume_token()? {
            Token::Asterisk => "*".to_string(),
            Token::Identifier(scope) | Token::Keyword(scope) => scope,
            token => return Err(ParseError::UnexpectedToken(token)),
        };
        let namespace = match self.consume_token()? {
            Token::Identifier(namespace) | Token::Keyword(namespace) | Token::String(namespace) => {
                self.finish_qualified_identifier(namespace)?
            }
            token => return Err(ParseError::UnexpectedToken(token)),
        };
        if let Ok(Token::Semicolon) = self.current_token() {
            self.consume_token()?;
        }
        Ok((scope, namespace))
    }

    fn parse_typedef(&mut self) -> Result<(String, ThriftType), ParseError> {
        self.expect_token(Token::Keyword("typedef".to_string()))?;
        let aliased_type = self.parse_type()?;
        let alias = match self.consume_token()? {
            Token::Identifier(alias) => alias,
            token => return Err(ParseError::UnexpectedToken(token)),
        };
        if let Ok(Token::Semicolon) = self.current_token() {
            self.consume_token()?;
        }
        Ok((alias, aliased_type))
    }

    fn parse_enum(&mut self) -> Result<ThriftEnum, ParseError> {
        self.expect_token(Token::Keyword("enum".to_string()))?;
        let name = match self.consume_token()? {
            Token::Identifier(name) => name,
            token => return Err(ParseError::UnexpectedToken(token)),
        };
        self.expect_token(Token::LeftBrace)?;
        let mut variants = HashMap::new();
        let mut next_value = 0i32;
        while let Ok(token) = self.current_token() {
            if let Token::RightBrace = token {
                break;
            }
            let variant_name = match self.consume_token()? {
                Token::Identifier(variant_name) => variant_name,
                Token::Comma | Token::Semicolon => continue,
                token => return Err(ParseError::UnexpectedToken(token)),
            };
            let value = if let Ok(Token::Equal) = self.current_token() {
                self.consume_token()?;
                match self.consume_token()? {
                    Token::Number(value) => value as i32,
                    token => return Err(ParseError::UnexpectedToken(token)),
                }
            } else {
                next_value
            };
            variants.insert(variant_name, value);
            next_value = value + 1;
            if matches!(self.current_token(), Ok(Token::Comma | Token::Semicolon)) {
                self.consume_token()?;
            }
        }
        self.expect_token(Token::RightBrace)?;
        self.skip_annotations()?;
        if let Ok(Token::Semicolon) = self.current_token() {
            self.consume_token()?;
        }
        Ok(ThriftEnum { name, variants })
    }

    fn parse_const(&mut self) -> Result<(), ParseError> {
        self.expect_token(Token::Keyword("const".to_string()))?;
        let const_type = self.parse_type()?;
        match self.consume_token()? {
            Token::Identifier(_) => {}
            token => return Err(ParseError::UnexpectedToken(token)),
        }
        if let Ok(Token::Equal) = self.current_token() {
            self.consume_token()?;
            if self.parse_default_value(&const_type).is_err() {
                self.skip_default_value()?;
            }
        }
        if let Ok(Token::Semicolon) = self.current_token() {
            self.consume_token()?;
        }
        Ok(())
    }

    fn consume_optional_extends(&mut self) -> Result<Option<String>, ParseError> {
        if matches!(self.current_token(), Ok(Token::Keyword(keyword)) if keyword == "extends") {
            self.consume_token()?;
            let name = match self.consume_token()? {
                Token::Identifier(name) | Token::Keyword(name) => {
                    self.finish_identifier_path(name)?
                }
                token => return Err(ParseError::UnexpectedToken(token)),
            };
            Ok(Some(name))
        } else {
            Ok(None)
        }
    }

    fn merge_parent_fields(
        &self,
        parent_name: Option<&str>,
        fields: Vec<ThriftField>,
    ) -> Result<Vec<ThriftField>, ParseError> {
        let Some(parent_name) = parent_name else {
            return Ok(fields);
        };
        let Some(parent) = self.parsed_structs.get(parent_name) else {
            return Err(ParseError::SyntaxError(format!(
                "Unknown parent struct: {}",
                parent_name
            )));
        };

        let mut merged = parent.fields.clone();
        for field in fields {
            if merged
                .iter()
                .any(|existing| existing.id == field.id || existing.name == field.name)
            {
                return Err(ParseError::SyntaxError(format!(
                    "Duplicate inherited field '{}' in extends {}",
                    field.name, parent_name
                )));
            }
            merged.push(field);
        }
        Ok(merged)
    }

    fn merge_parent_methods(
        &self,
        parent_name: Option<&str>,
        methods: Vec<ThriftMethod>,
    ) -> Result<Vec<ThriftMethod>, ParseError> {
        let Some(parent_name) = parent_name else {
            return Ok(methods);
        };
        let Some(parent) = self.parsed_services.get(parent_name) else {
            return Err(ParseError::SyntaxError(format!(
                "Unknown parent service: {}",
                parent_name
            )));
        };

        let mut merged = parent.methods.clone();
        for method in methods {
            if merged.iter().any(|existing| existing.name == method.name) {
                return Err(ParseError::SyntaxError(format!(
                    "Duplicate inherited method '{}' in extends {}",
                    method.name, parent_name
                )));
            }
            merged.push(method);
        }
        Ok(merged)
    }

    fn skip_annotations(&mut self) -> Result<(), ParseError> {
        while matches!(self.current_token(), Ok(Token::LeftParen)) {
            self.consume_token()?;
            self.skip_balanced_block(Token::LeftParen, Token::RightParen)?;
        }
        Ok(())
    }

    fn parse_default_value(&mut self, thrift_type: &ThriftType) -> Result<ThriftValue, ParseError> {
        match thrift_type {
            ThriftType::Bool => self.parse_bool_default(),
            ThriftType::Byte => Ok(ThriftValue::Byte(self.parse_i64_default()? as i8)),
            ThriftType::I16 => Ok(ThriftValue::I16(self.parse_i64_default()? as i16)),
            ThriftType::I32 => Ok(ThriftValue::I32(self.parse_i64_default()? as i32)),
            ThriftType::I64 => Ok(ThriftValue::I64(self.parse_i64_default()?)),
            ThriftType::Double => Ok(ThriftValue::Double(self.parse_i64_default()? as f64)),
            ThriftType::String => match self.consume_token()? {
                Token::String(value) | Token::Identifier(value) | Token::Keyword(value) => {
                    Ok(ThriftValue::String(value))
                }
                token => Err(ParseError::UnexpectedToken(token)),
            },
            ThriftType::Binary => match self.consume_token()? {
                Token::String(value) => Ok(ThriftValue::Binary(value.into_bytes())),
                token => Err(ParseError::UnexpectedToken(token)),
            },
            ThriftType::List(element_type) => {
                let values = self.parse_default_sequence(element_type)?;
                Ok(ThriftValue::List(values))
            }
            ThriftType::Set(element_type) => {
                let values = self.parse_default_sequence(element_type)?;
                Ok(ThriftValue::Set(values))
            }
            ThriftType::Map(key_type, value_type) => {
                self.expect_token(Token::LeftBrace)?;
                let mut pairs = Vec::new();
                while let Ok(token) = self.current_token() {
                    if let Token::RightBrace = token {
                        break;
                    }
                    let key = self.parse_default_value(key_type)?;
                    self.expect_token(Token::Colon)?;
                    let value = self.parse_default_value(value_type)?;
                    pairs.push((key, value));
                    if matches!(self.current_token(), Ok(Token::Comma | Token::Semicolon)) {
                        self.consume_token()?;
                    }
                }
                self.expect_token(Token::RightBrace)?;
                Ok(ThriftValue::Map(pairs))
            }
            ThriftType::Struct(name) => {
                self.skip_default_value()?;
                Ok(ThriftValue::Struct {
                    name: Some(name.clone()),
                    fields: HashMap::new(),
                })
            }
        }
    }

    fn parse_default_sequence(
        &mut self,
        element_type: &ThriftType,
    ) -> Result<Vec<ThriftValue>, ParseError> {
        self.expect_token(Token::LeftBracket)?;
        let mut values = Vec::new();
        while let Ok(token) = self.current_token() {
            if let Token::RightBracket = token {
                break;
            }
            values.push(self.parse_default_value(element_type)?);
            if matches!(self.current_token(), Ok(Token::Comma | Token::Semicolon)) {
                self.consume_token()?;
            }
        }
        self.expect_token(Token::RightBracket)?;
        Ok(values)
    }

    fn parse_bool_default(&mut self) -> Result<ThriftValue, ParseError> {
        match self.consume_token()? {
            Token::Identifier(value) | Token::Keyword(value) if value == "true" => {
                Ok(ThriftValue::Bool(true))
            }
            Token::Identifier(value) | Token::Keyword(value) if value == "false" => {
                Ok(ThriftValue::Bool(false))
            }
            Token::Number(value) => Ok(ThriftValue::Bool(value != 0)),
            token => Err(ParseError::UnexpectedToken(token)),
        }
    }

    fn parse_i64_default(&mut self) -> Result<i64, ParseError> {
        match self.consume_token()? {
            Token::Number(value) => Ok(value),
            Token::Identifier(first) | Token::Keyword(first) => {
                let parts = self.finish_identifier_path_parts(first)?;
                if parts.len() == 2 {
                    if let Some(value) = self
                        .enum_values
                        .get(&parts[0])
                        .and_then(|variants| variants.get(&parts[1]))
                    {
                        return Ok(*value as i64);
                    }
                }
                Err(ParseError::SyntaxError(format!(
                    "Unknown numeric default value: {}",
                    parts.join(".")
                )))
            }
            token => Err(ParseError::UnexpectedToken(token)),
        }
    }

    fn finish_identifier_path(&mut self, first: String) -> Result<String, ParseError> {
        Ok(self
            .finish_identifier_path_parts(first)?
            .into_iter()
            .last()
            .unwrap_or_default())
    }

    fn finish_qualified_identifier(&mut self, first: String) -> Result<String, ParseError> {
        Ok(self.finish_identifier_path_parts(first)?.join("."))
    }

    fn finish_identifier_path_parts(&mut self, first: String) -> Result<Vec<String>, ParseError> {
        let mut parts = vec![first];
        while let Ok(Token::Dot) = self.current_token() {
            self.consume_token()?;
            let part = match self.consume_token()? {
                Token::Identifier(part) | Token::Keyword(part) => part,
                token => return Err(ParseError::UnexpectedToken(token)),
            };
            parts.push(part);
        }
        Ok(parts)
    }

    fn skip_default_value(&mut self) -> Result<(), ParseError> {
        let mut paren_depth = 0usize;
        let mut brace_depth = 0usize;
        let mut bracket_depth = 0usize;
        let mut consumed_any = false;

        while self.position < self.tokens.len() {
            let token = self.current_token()?.clone();
            let at_top_level = paren_depth == 0 && brace_depth == 0 && bracket_depth == 0;
            if at_top_level
                && matches!(
                    token,
                    Token::Semicolon | Token::Comma | Token::RightParen | Token::RightBrace
                )
            {
                break;
            }
            if consumed_any && at_top_level && matches!(token, Token::Number(_) | Token::Keyword(_))
            {
                break;
            }

            match token {
                Token::LeftParen => paren_depth += 1,
                Token::RightParen => paren_depth = paren_depth.saturating_sub(1),
                Token::LeftBrace => brace_depth += 1,
                Token::RightBrace => brace_depth = brace_depth.saturating_sub(1),
                Token::LeftBracket => bracket_depth += 1,
                Token::RightBracket => bracket_depth = bracket_depth.saturating_sub(1),
                _ => {}
            }
            self.consume_token()?;
            consumed_any = true;
        }
        Ok(())
    }

    fn skip_balanced_block(&mut self, left: Token, right: Token) -> Result<(), ParseError> {
        let mut depth = 1usize;
        while depth > 0 {
            let token = self.consume_token()?;
            if std::mem::discriminant(&token) == std::mem::discriminant(&left) {
                depth += 1;
            } else if std::mem::discriminant(&token) == std::mem::discriminant(&right) {
                depth -= 1;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(input: &str) -> ThriftDocument {
        let mut parser = Parser::new(input).unwrap();
        parser.parse_document().unwrap()
    }

    #[test]
    fn parses_empty_document() {
        let doc = parse("");
        assert!(doc.structs.is_empty());
        assert!(doc.services.is_empty());
    }

    #[test]
    fn parses_struct_name_and_fields() {
        let doc = parse("struct User { 1: required i32 id; 2: optional string name; }");
        let user = doc.structs.get("User").unwrap();
        assert_eq!(user.fields.len(), 2);
        assert_eq!(user.fields[0].name, "id");
        assert!(matches!(user.fields[0].field_type, ThriftType::I32));
        assert!(user.fields[0].required);
        assert!(!user.fields[1].required);
    }

    #[test]
    fn parses_struct_fields_with_comma_separators() {
        let doc = parse("struct User { 1: required i32 id, 2: optional string name, }");
        let user = doc.structs.get("User").unwrap();
        assert_eq!(user.fields.len(), 2);
        assert_eq!(user.fields[0].name, "id");
        assert_eq!(user.fields[1].name, "name");
    }

    #[test]
    fn skips_default_values() {
        let doc = parse("struct User { 1: required string name = \"Ada\"; }");
        let field = &doc.structs.get("User").unwrap().fields[0];
        assert_eq!(field.name, "name");
        assert!(
            matches!(field.default_value, Some(ThriftValue::String(ref value)) if value == "Ada")
        );
    }

    #[test]
    fn skips_container_default_values() {
        let doc = parse(
            "struct User { 1: optional list<i32> ids = [1, 2, 3]; 2: string name = \"Ada\"; }",
        );
        let fields = &doc.structs.get("User").unwrap().fields;
        assert_eq!(fields.len(), 2);
        assert_eq!(fields[0].name, "ids");
        assert_eq!(fields[1].name, "name");
    }

    #[test]
    fn parses_list_set_and_map_types() {
        let doc = parse(
            "struct Data { 1: list<i32> nums; 2: set<string> tags; 3: map<string, i64> counts; }",
        );
        let fields = &doc.structs.get("Data").unwrap().fields;
        assert!(matches!(fields[0].field_type, ThriftType::List(_)));
        assert!(matches!(fields[1].field_type, ThriftType::Set(_)));
        assert!(matches!(fields[2].field_type, ThriftType::Map(_, _)));
    }

    #[test]
    fn parses_nested_container_type() {
        let doc = parse("struct Data { 1: map<string, list<i32>> grouped; }");
        let field = &doc.structs.get("Data").unwrap().fields[0];
        assert!(matches!(field.field_type, ThriftType::Map(_, _)));
    }

    #[test]
    fn parses_custom_struct_type() {
        let doc = parse("struct Child { 1: string name; } struct Parent { 1: Child child; }");
        let field = &doc.structs.get("Parent").unwrap().fields[0];
        assert!(matches!(&field.field_type, ThriftType::Struct(name) if name == "Child"));
    }

    #[test]
    fn parses_include_namespace_typedef_enum_and_const() {
        let doc = parse(
            r#"
            include "common.thrift"
            namespace py thrift.example
            typedef i64 Timestamp
            enum Status { OK = 1, FAILED = 2 }
            const map<string, string> DEFAULT_LABELS = {"source": "test"}
            struct Event {
                1: Timestamp created_at;
                2: Status status = Status.OK;
            }
            "#,
        );

        assert_eq!(doc.includes, vec!["common.thrift".to_string()]);
        assert_eq!(doc.namespaces.get("py").unwrap(), "thrift.example");
        assert_eq!(
            doc.enums.get("Status").unwrap().variants.get("OK"),
            Some(&1)
        );
        let fields = &doc.structs.get("Event").unwrap().fields;
        assert!(matches!(fields[0].field_type, ThriftType::I64));
        assert!(matches!(fields[1].field_type, ThriftType::I32));
        assert!(matches!(fields[1].default_value, Some(ThriftValue::I32(1))));
    }

    #[test]
    fn parses_union_extends_annotations_and_defaults() {
        let doc = parse(
            r#"
            struct Base { 1: string id; }
            union Choice extends Base (scope="test") {
                2: string name = "fallback" (ui.hidden="true");
                3: list<i32> ids = [1, 2, 3];
                4: map<string, i32> counts = {"a": 1, "b": 2};
            }
            service Parent { void base_ping(); }
            service Child extends Parent (owner="bench") { void ping(); }
            "#,
        );
        let fields: HashMap<_, _> = doc
            .structs
            .get("Choice")
            .unwrap()
            .fields
            .iter()
            .map(|field| (field.name.as_str(), field))
            .collect();
        assert!(fields.contains_key("id"));
        assert!(
            matches!(fields["name"].default_value, Some(ThriftValue::String(ref value)) if value == "fallback")
        );
        assert!(
            matches!(fields["ids"].default_value, Some(ThriftValue::List(ref values)) if values.len() == 3)
        );
        assert!(
            matches!(fields["counts"].default_value, Some(ThriftValue::Map(ref pairs)) if pairs.len() == 2)
        );
        let methods: Vec<_> = doc
            .services
            .get("Child")
            .unwrap()
            .methods
            .iter()
            .map(|method| method.name.as_str())
            .collect();
        assert_eq!(methods, vec!["base_ping", "ping"]);
    }

    #[test]
    fn parses_exception_and_method_throws() {
        let doc = parse(
            "exception NotFound { 1: string message; } service Users { string get(1: i32 id) throws (1: NotFound err); }",
        );
        assert!(doc.structs.contains_key("NotFound"));
        let method = &doc.services.get("Users").unwrap().methods[0];
        assert_eq!(method.exceptions.len(), 1);
        assert!(
            matches!(&method.exceptions[0].field_type, ThriftType::Struct(name) if name == "NotFound")
        );
    }

    #[test]
    fn preserves_qualified_type_name() {
        let doc =
            parse("struct Shared { 1: string name; } struct Holder { 1: common.Shared value; }");
        let field = &doc.structs.get("Holder").unwrap().fields[0];
        assert!(matches!(&field.field_type, ThriftType::Struct(name) if name == "common.Shared"));
    }

    #[test]
    fn parses_service_methods() {
        let doc = parse("service UserService { bool save(1: i32 id); list<i32> all(); }");
        let service = doc.services.get("UserService").unwrap();
        assert_eq!(service.methods.len(), 2);
        assert_eq!(service.methods[0].name, "save");
        assert!(matches!(service.methods[0].return_type, ThriftType::Bool));
        assert_eq!(service.methods[0].arguments[0].name, "id");
    }

    #[test]
    fn parses_oneway_method() {
        let doc = parse("service Events { oneway void notify(1: string message); }");
        let method = &doc.services.get("Events").unwrap().methods[0];
        assert!(method.oneway);
        assert!(matches!(&method.return_type, ThriftType::Struct(name) if name == "void"));
    }

    #[test]
    fn skips_unknown_top_level_tokens_before_struct() {
        let doc = parse("namespace py ignored include \"x.thrift\" struct User { 1: i32 id; }");
        assert!(doc.structs.contains_key("User"));
    }

    #[test]
    fn invalid_field_id_returns_error() {
        let mut parser = Parser::new("struct Bad { id: string name; }").unwrap();
        assert!(parser.parse_document().is_err());
    }

    #[test]
    fn unclosed_struct_returns_error() {
        let mut parser = Parser::new("struct Bad { 1: string name;").unwrap();
        assert!(parser.parse_document().is_err());
    }

    #[test]
    fn tokenization_remainder_returns_new_error() {
        assert!(Parser::new("struct Bad @").is_err());
    }
}
