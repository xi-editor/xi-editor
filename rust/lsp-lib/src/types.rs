use jsonrpc_lite::Error;
use serde_json::Value;
use language_server::LanguageServerClient;
use serde_json;
use std;

pub enum LSPHeader {
    ContentType,
    ContentLength(usize),
}

pub trait Callable: Send {
    fn call(self: Box<Self>, client: &mut LanguageServerClient, result: Result<Value, Error>);
}

impl<F: Send + FnOnce(&mut LanguageServerClient, Result<Value, Error>)> Callable for F {
    fn call(self: Box<F>, client: &mut LanguageServerClient, result: Result<Value, Error>) {
        (*self)(client, result)
    }
}

pub type Callback = Box<Callable>;

// Error Types
#[derive(Debug)]
pub enum ParseError {
    Io(std::io::Error),
    ParseInt(std::num::ParseIntError),
    Utf8(std::string::FromUtf8Error),
    Json(serde_json::Error),
    Unknown(String),
}

impl From<std::io::Error> for ParseError {
    fn from(err: std::io::Error) -> ParseError {
        ParseError::Io(err)
    }
}

impl From<std::string::FromUtf8Error> for ParseError {
    fn from(err: std::string::FromUtf8Error) -> ParseError {
        ParseError::Utf8(err)
    }
}

impl From<serde_json::Error> for ParseError {
    fn from(err: serde_json::Error) -> ParseError {
        ParseError::Json(err)
    }
}

impl From<std::num::ParseIntError> for ParseError {
    fn from(err: std::num::ParseIntError) -> ParseError {
        ParseError::ParseInt(err)
    }
}

impl From<String> for ParseError {
    fn from(s: String) -> ParseError {
        ParseError::Unknown(s)
    }
}
