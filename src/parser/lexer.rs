use nom::{
    branch::alt,
    bytes::complete::{tag, take_while, take_while1, take_until},
    character::complete::{char, digit1, multispace0, multispace1},
    combinator::{map, opt, recognize, value},
    multi::{many0, separated_list0},
    sequence::{delimited, pair, preceded, terminated, tuple},
    IResult,
};
use nom::character::complete::alpha1;

#[derive(Debug, Clone, PartialEq)]
pub enum Token {
    Keyword(String),
    Identifier(String),
    Number(i64),
    String(String),
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
}

pub fn parse_identifier(input: &str) -> IResult<&str, String> {
    map(
        recognize(pair(
            alt((tag("_"), alpha1)),
            take_while(|c: char| c.is_alphanumeric() || c == '_'),
        )),
        |s: &str| s.to_string(),
    )(input)
}

pub fn parse_number(input: &str) -> IResult<&str, i64> {
    map(
        recognize(pair(opt(char('-')), digit1)),
        |s: &str| s.parse().unwrap(),
    )(input)
}

pub fn parse_string(input: &str) -> IResult<&str, String> {
    delimited(
        char('"'),
        map(take_while(|c| c != '"'), |s: &str| s.to_string()),
        char('"'),
    )(input)
}

pub fn skip_comment(input: &str) -> IResult<&str, ()> {
    alt((
        // Line comment: // ...
        value((), pair(tag("//"), take_while(|c| c != '\n'))),
        // Block comment: /* ... */
        value((), tuple((tag("/*"), take_until("*/"), tag("*/")))),
        // Hash comment: # ...
        value((), pair(tag("#"), take_while(|c| c != '\n'))),
    ))(input)
}

pub fn skip_whitespace_and_comments(input: &str) -> IResult<&str, ()> {
    value((), many0(alt((
        value((), multispace1),
        skip_comment,
    ))))(input)
}

pub fn parse_token(input: &str) -> IResult<&str, Token> {
    preceded(
        skip_whitespace_and_comments,
        alt((
            map(parse_string, Token::String),
            map(parse_number, Token::Number),
            map(tag("struct"), |_| Token::Keyword("struct".to_string())),
            map(tag("service"), |_| Token::Keyword("service".to_string())),
            map(tag("required"), |_| Token::Keyword("required".to_string())),
            map(tag("optional"), |_| Token::Keyword("optional".to_string())),
            map(parse_identifier, Token::Identifier),
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
        )),
    )(input)
}

pub fn tokenize(input: &str) -> IResult<&str, Vec<Token>> {
    many0(parse_token)(input)
}
