use std;
use std::io::{BufRead};
use serde_json::value::Value;
use serde_json;

use types::{LSPHeader, ParseError};

const HEADER_CONTENT_LENGTH: &'static str = "content-length";
const HEADER_CONTENT_TYPE: &'static str = "content-type";

// To Parse header from the incoming input string
fn parse_header(s: &str) -> Result<LSPHeader, ParseError> {
    let split: Vec<String> = s.split(": ").map(|s| s.trim().to_lowercase()).collect();
    if split.len() != 2 { return Err(ParseError::Unknown("Malformed".to_string())) };
    match split[0].as_ref() {
        HEADER_CONTENT_TYPE => Ok(LSPHeader::ContentType),
        HEADER_CONTENT_LENGTH => Ok(LSPHeader::ContentLength(usize::from_str_radix(&split[1], 10)?)),
        _ => Err(ParseError::Unknown("Unknown parse error occured".to_string()))
    }
}

// Blocking call to read a message from the provided Buffered Reader
pub fn read_message<T: BufRead>(reader: &mut T) -> Result<String, ParseError> {
    let mut buffer = String::new();
    let mut content_length : Option<usize> = None;

    loop {
        buffer.clear();
        reader.read_line(&mut buffer);

        match &buffer {
            s if s.trim().len() == 0 => { break },
            s => {
                match parse_header(s)? {
                    LSPHeader::ContentLength(len) => content_length = Some(len),
                    LSPHeader::ContentType => ()
                };
            }
        };
    }

    let content_length = content_length.ok_or(format!("missing content-length header: {}", buffer))?;

    let mut body_buffer = vec![0; content_length];
    reader.read_exact(&mut body_buffer)?;

    let body = String::from_utf8(body_buffer)?;
    Ok(body)
}