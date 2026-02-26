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

        Ok(Parser { tokens, position: 0 })
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
            includes: Vec::new(),
            namespaces: HashMap::new(),
        };

        while self.position < self.tokens.len() {
            match self.current_token()? {
                Token::Keyword(keyword) if keyword == "struct" => {
                    let struct_def = self.parse_struct()?;
                    document.structs.insert(struct_def.name.clone(), struct_def);
                }
                Token::Keyword(keyword) if keyword == "service" => {
                    let service_def = self.parse_service()?;
                    document.services.insert(service_def.name.clone(), service_def);
                }
                _ => {
                    self.consume_token()?; // Skip unknown tokens
                }
            }
        }

        Ok(document)
    }

    fn parse_struct(&mut self) -> Result<ThriftStruct, ParseError> {
        self.expect_token(Token::Keyword("struct".to_string()))?;

        let name = match self.consume_token()? {
            Token::Identifier(name) => name,
            token => return Err(ParseError::UnexpectedToken(token)),
        };

        self.expect_token(Token::LeftBrace)?;

        let mut fields = Vec::new();
        while let Ok(token) = self.current_token() {
            if let Token::RightBrace = token {
                break;
            }
            fields.push(self.parse_field()?);
        }

        self.expect_token(Token::RightBrace)?;

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

        let name = match self.consume_token()? {
            Token::Identifier(name) => name,
            token => return Err(ParseError::UnexpectedToken(token)),
        };

        // Skip optional default value and semicolon
        if let Ok(Token::Equal) = self.current_token() {
            self.consume_token()?; // consume '='
            self.consume_token()?; // consume default value (simplified)
        }

        if let Ok(Token::Semicolon) = self.current_token() {
            self.consume_token()?;
        }

        Ok(ThriftField {
            id,
            name,
            field_type,
            required,
            default_value: None,
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
                _ => Ok(ThriftType::Struct(type_name)),
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

        self.expect_token(Token::LeftBrace)?;

        let mut methods = Vec::new();
        while let Ok(token) = self.current_token() {
            if let Token::RightBrace = token {
                break;
            }
            methods.push(self.parse_method()?);
        }

        self.expect_token(Token::RightBrace)?;

        Ok(ThriftService { name, methods })
    }

    fn parse_method(&mut self) -> Result<ThriftMethod, ParseError> {
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

        // Skip optional throws clause
        if let Ok(Token::Semicolon) = self.current_token() {
            self.consume_token()?;
        }

        Ok(ThriftMethod {
            name,
            return_type,
            arguments,
            exceptions: Vec::new(),
        })
    }
}
