use nom::character::complete::alpha1;
use nom::{
    branch::alt,
    bytes::complete::{tag, take_until, take_while},
    character::complete::{char, digit1, multispace1},
    combinator::{map, opt, recognize, value},
    multi::many0,
    sequence::{delimited, pair, preceded},
    IResult, Parser,
};

#[derive(Debug, Clone, PartialEq)]
pub enum Token {
    Keyword(String),
    Identifier(String),
    Number(i64),
    String(String),
    #[allow(dead_code)]
    Symbol(char),
    Comma,
    Semicolon,
    LeftParen,
    RightParen,
    LeftBrace,
    RightBrace,
    LeftBracket,
    RightBracket,
    LessThan,
    GreaterThan,
    Colon,
    Equal,
    Dot,
    Asterisk,
}

fn keyword_or_identifier(ident: String) -> Token {
    match ident.as_str() {
        "struct" | "service" | "required" | "optional" | "oneway" | "include" | "namespace"
        | "typedef" | "enum" | "const" | "throws" | "exception" => Token::Keyword(ident),
        _ => Token::Identifier(ident),
    }
}

pub fn parse_identifier(input: &str) -> IResult<&str, String> {
    map(
        recognize(pair(
            alt((tag("_"), alpha1)),
            take_while(|c: char| c.is_alphanumeric() || c == '_'),
        )),
        |s: &str| s.to_string(),
    )
    .parse(input)
}

pub fn parse_string(input: &str) -> IResult<&str, String> {
    delimited(
        char('"'),
        map(take_while(|c| c != '"'), |s: &str| s.to_string()),
        char('"'),
    )
    .parse(input)
}

pub fn parse_number(input: &str) -> IResult<&str, i64> {
    map(recognize(pair(opt(char('-')), digit1)), |s: &str| {
        s.parse::<i64>().unwrap()
    })
    .parse(input)
}

pub fn skip_comment(input: &str) -> IResult<&str, ()> {
    alt((
        // Line comment: // ...
        value((), pair(tag("//"), take_while(|c| c != '\n'))),
        // Block comment: /* ... */
        value((), (tag("/*"), take_until("*/"), tag("*/"))),
        // Hash comment: # ...
        value((), pair(tag("#"), take_while(|c| c != '\n'))),
    ))
    .parse(input)
}

pub fn skip_whitespace_and_comments(input: &str) -> IResult<&str, ()> {
    value((), many0(alt((value((), multispace1), skip_comment)))).parse(input)
}

pub fn parse_token(input: &str) -> IResult<&str, Token> {
    preceded(
        skip_whitespace_and_comments,
        alt((
            map(parse_string, Token::String),
            map(parse_number, Token::Number),
            map(parse_identifier, keyword_or_identifier),
            map(char(','), |_| Token::Comma),
            map(char(';'), |_| Token::Semicolon),
            map(char('('), |_| Token::LeftParen),
            map(char(')'), |_| Token::RightParen),
            map(char('{'), |_| Token::LeftBrace),
            map(char('}'), |_| Token::RightBrace),
            map(char('['), |_| Token::LeftBracket),
            map(char(']'), |_| Token::RightBracket),
            map(char('<'), |_| Token::LessThan),
            map(char('>'), |_| Token::GreaterThan),
            map(char(':'), |_| Token::Colon),
            map(char('='), |_| Token::Equal),
            map(char('.'), |_| Token::Dot),
            map(char('*'), |_| Token::Asterisk),
        )),
    )
    .parse(input)
}

pub fn tokenize(input: &str) -> IResult<&str, Vec<Token>> {
    many0(parse_token).parse(input)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_identifier_with_underscore() {
        let (remaining, ident) = parse_identifier("user_id ").unwrap();
        assert_eq!(ident, "user_id");
        assert_eq!(remaining, " ");
    }

    #[test]
    fn parses_negative_number() {
        let (remaining, number) = parse_number("-42,").unwrap();
        assert_eq!(number, -42);
        assert_eq!(remaining, ",");
    }

    #[test]
    fn parses_string_literal() {
        let (remaining, value) = parse_string("\"hello\";").unwrap();
        assert_eq!(value, "hello");
        assert_eq!(remaining, ";");
    }

    #[test]
    fn skips_line_block_and_hash_comments() {
        assert!(skip_comment("// hello\n").is_ok());
        assert!(skip_comment("/* hello */").is_ok());
        assert!(skip_comment("# hello\n").is_ok());
    }

    #[test]
    fn tokenizes_struct_definition() {
        let (_, tokens) = tokenize("struct User { 1: required i32 id; }").unwrap();
        assert_eq!(tokens[0], Token::Keyword("struct".to_string()));
        assert_eq!(tokens[1], Token::Identifier("User".to_string()));
        assert!(tokens.contains(&Token::Colon));
        assert!(tokens.contains(&Token::Semicolon));
    }

    #[test]
    fn tokenizes_container_symbols() {
        let (_, tokens) = tokenize("map<string, list<i32>> values").unwrap();
        assert!(tokens.contains(&Token::LessThan));
        assert!(tokens.contains(&Token::GreaterThan));
        assert!(tokens.contains(&Token::Comma));
    }

    #[test]
    fn tokenizes_oneway_as_keyword() {
        let (_, tokens) = tokenize("oneway void notify()").unwrap();
        assert_eq!(tokens[0], Token::Keyword("oneway".to_string()));
        assert_eq!(tokens[1], Token::Identifier("void".to_string()));
    }

    #[test]
    fn does_not_split_identifier_with_keyword_prefix() {
        let (_, tokens) =
            tokenize("optional_note required_field oneway_mode include_path").unwrap();
        assert_eq!(tokens[0], Token::Identifier("optional_note".to_string()));
        assert_eq!(tokens[1], Token::Identifier("required_field".to_string()));
        assert_eq!(tokens[2], Token::Identifier("oneway_mode".to_string()));
        assert_eq!(tokens[3], Token::Identifier("include_path".to_string()));
    }

    #[test]
    fn tokenizes_compatibility_keywords() {
        let (_, tokens) =
            tokenize("include namespace typedef enum const throws exception *").unwrap();
        assert_eq!(tokens[0], Token::Keyword("include".to_string()));
        assert_eq!(tokens[1], Token::Keyword("namespace".to_string()));
        assert_eq!(tokens[2], Token::Keyword("typedef".to_string()));
        assert_eq!(tokens[3], Token::Keyword("enum".to_string()));
        assert_eq!(tokens[4], Token::Keyword("const".to_string()));
        assert_eq!(tokens[5], Token::Keyword("throws".to_string()));
        assert_eq!(tokens[6], Token::Keyword("exception".to_string()));
        assert_eq!(tokens[7], Token::Asterisk);
    }

    #[test]
    fn leaves_unrecognized_input_for_parser_error() {
        let (remaining, tokens) = tokenize("struct Broken @").unwrap();
        assert_eq!(tokens.len(), 2);
        assert_eq!(remaining, " @");
    }
}
