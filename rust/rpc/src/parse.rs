// Copyright 2017 Google Inc. All rights reserved.
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

//! Parsing of raw JSON messages into RPC objects.

use serde_json::Value;
use serde::de::{Deserializer, Deserialize};

use error::RemoteError;


/// A unique identifier attached to request RPCs.
type RequestId = u64;

#[derive(Debug, Clone, PartialEq)]
/// An internal type, used to represent the various possible outcomes
/// of attempting to parse a new message.
///
/// Because of the generality of the failure messages we get from serde,
/// we handle various failure cases on our own.
pub enum ParseResult<N, R> {
    /// An id and an RPC Request
    Request(RequestId, R),
    /// An RPC Notification
    Notification(N),
    /// A response from the peer
    Response(RequestId, Response),
    /// The JSON was not a correctly formed request. The peer will
    /// receive an error response, with the id if present.
    InvalidRequest(Option<RequestId>, RemoteError),
    /// The JSON contained an ID but was not a valid Response.
    InvalidResponse(RequestId),
    /// The message could not be parsed for some other reason.
    OtherError(String),
}

pub type Response = Result<Value, RemoteError>;

//TODO: revisit this if https://github.com/arcnmx/serde-value/issues/15 happens
impl<'de, N, R> Deserialize<'de> for ParseResult<N, R>
    where N: Deserialize<'de>,
          R: Deserialize<'de>
{
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
        where D: Deserializer<'de>
    {
        #[derive(Deserialize)]
        struct Helper {
            id: Option<u64>,
            method: Option<String>,
        }

        #[derive(Deserialize)]
        struct ResponseHelper {
            result: Option<Value>,
            error: Option<RemoteError>,
        }

        let v = match Value::deserialize(deserializer) {
            Ok(v) => v,
            Err(err) => return Ok(ParseResult::OtherError(err.to_string())),
        };

        let helper =  match Helper::deserialize(&v) {
            Ok(h) => h,
            Err(e) => return Ok(ParseResult::OtherError(e.to_string())),
        };

        let is_method = helper.method.is_some();
        match (is_method, helper.id) {
            (true, Some(id)) => {
                match R::deserialize(v) {
                    Ok(r) => Ok(ParseResult::Request(id, r)),
                    Err(e) => {
                        //TODO: if serde-json error messages improve, we should send
                        // 'method not found' or 'invalid params' where appropriate
                        let err = RemoteError::InvalidRequest(Some(json!(e.to_string())));
                        Ok(ParseResult::InvalidRequest(Some(id), err))
                    }
                }
            }
            (true, None) => {
                match N::deserialize(v) {
                    Ok(n) => Ok(ParseResult::Notification(n)),
                    Err(e) => Ok(ParseResult::OtherError(e.to_string())),
                }
            }
            (false, Some(id)) => {
                match ResponseHelper::deserialize(v) {
                    Ok(resp) => {
                        assert!(resp.result.is_some() != resp.error.is_some());
                        if let Some(r) = resp.result {
                            Ok(ParseResult::Response(id, Ok(r)))
                        } else {
                            Ok(ParseResult::Response(id, Err(resp.error.unwrap())))
                        }
                    }
                    Err(_) => Ok(ParseResult::InvalidResponse(id)),
                }
            }
            // if 'id' and 'method' fields are missing, msg is malformed
            _ => Ok(ParseResult::OtherError("RPC is missing 'id' or 'method' field".into())),
        }
    }
}

#[cfg(test)]
mod tests {

    use serde_json;
    use super::*;

    #[derive(Serialize, Deserialize, Debug, PartialEq)]
    #[serde(rename_all = "snake_case")]
    #[serde(tag = "method", content = "params")]
    enum TestR {
        NewView { file_path: Option<String> },
        OldView { file_path: usize },
    }

    #[derive(Serialize, Deserialize, Debug, PartialEq)]
    #[serde(rename_all = "snake_case")]
    #[serde(tag = "method", content = "params")]
    enum TestN {
        CloseView { view_id: String },
        Save { view_id: String, file_path: String },
    }

    type RpcType = ParseResult<TestN, TestR>;

    #[test]
    fn request_success() {
        let json = r#"{"id":0,"method":"new_view","params":{}}"#;
        let p = serde_json::from_str::<RpcType>(json).unwrap();
        assert_eq!(p, ParseResult::Request(0, TestR::NewView { file_path: None }));
    }

    #[test]
    fn request_failure() {
        // method does not exist
        let json = r#"{"id":0,"method":"new_truth","params":{}}"#;
        let p = serde_json::from_str::<RpcType>(json).unwrap();
        let is_ok = match p {
            ParseResult::InvalidRequest(Some(0), _) => true,
            _ => false,
        };
        if !is_ok {
            panic!("{:?}", p);
        }
    }

    #[test]
    fn notif_with_id() {
        // method is a notification, should not have ID
        let json = r#"{"id":0,"method":"close_view","params":{"view_id": "view-id-1"}}"#;
        let p = serde_json::from_str::<RpcType>(json).unwrap();
        let is_ok = match p {
            ParseResult::InvalidRequest(Some(0), _) => true,
            _ => false,
        };
        if !is_ok {
            panic!("{:?}", p);
        }
    }

    #[test]
    fn test_resp_err() {
        let json = r#"{"id":5,"error":{"code":420, "message":"chill out"}}"#;
        let p = serde_json::from_str::<RpcType>(json).unwrap();
        assert_eq!(p, ParseResult::Response(5, Err(RemoteError::custom(420, "chill out", None))));
    }

    #[test]
    fn test_resp_result() {
        let json = r#"{"id":5,"result":"success!"}"#;
        let p = serde_json::from_str::<RpcType>(json).unwrap();
        assert_eq!(p, ParseResult::Response(5, Ok(json!("success!"))));
    }

    #[test]
    fn test_err() {
        let json = r#"{"code": -32600, "message": "Invalid Request"}"#;
        let e = serde_json::from_str::<RemoteError>(json).unwrap();
        assert_eq!(e, RemoteError::InvalidRequest(None));
    }
}
