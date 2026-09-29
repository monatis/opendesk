use serde::{Deserialize, Serialize};
use std::collections::HashMap;

pub const PROTOCOL_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    CapabilityUnsupported,
    PermissionDenied,
    InvalidArgument,
    NotFound,
    Timeout,
    Cancelled,
    Internal,
    Protocol,
    Busy,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct ErrorInfo {
    pub code: String,
    #[serde(default)]
    pub message: String,
    #[serde(default)]
    pub details: HashMap<String, serde_json::Value>,
}

impl std::fmt::Display for ErrorInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.message.is_empty() {
            write!(f, "{}", self.code)
        } else {
            write!(f, "{}: {}", self.code, self.message)
        }
    }
}

impl std::error::Error for ErrorInfo {}

impl ErrorInfo {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            details: HashMap::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Frame {
    Hello(HelloFrame),
    Req(ReqFrame),
    Res(ResFrame),
    Cancel(CancelFrame),
    Push(PushFrame),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HelloFrame {
    #[serde(default = "default_protocol_version")]
    pub v: u32,
    pub role: String,
    #[serde(default)]
    pub principal: String,
    #[serde(default)]
    pub auth: HashMap<String, serde_json::Value>,
    #[serde(default)]
    pub capabilities: HashMap<String, serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<ErrorInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ReqFrame {
    #[serde(default = "default_protocol_version")]
    pub v: u32,
    pub id: u64,
    pub method: String,
    #[serde(default)]
    pub stream: bool,
    #[serde(default)]
    pub params: HashMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ResFrame {
    #[serde(default = "default_protocol_version")]
    pub v: u32,
    pub id: u64,
    #[serde(default)]
    pub seq: u64,
    #[serde(default = "default_true")]
    pub end: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<ErrorInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CancelFrame {
    #[serde(default = "default_protocol_version")]
    pub v: u32,
    pub id: u64,
    #[serde(default)]
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PushFrame {
    #[serde(default = "default_protocol_version")]
    pub v: u32,
    pub topic: String,
    #[serde(default)]
    pub payload: HashMap<String, serde_json::Value>,
}

impl PushFrame {
    pub fn new(topic: &str, payload: HashMap<String, serde_json::Value>) -> Self {
        Self {
            v: PROTOCOL_VERSION,
            topic: topic.to_string(),
            payload,
        }
    }
}

fn default_protocol_version() -> u32 {
    PROTOCOL_VERSION
}

fn default_true() -> bool {
    true
}

impl HelloFrame {
    pub fn server(capabilities: HashMap<String, serde_json::Value>) -> Self {
        Self {
            v: PROTOCOL_VERSION,
            role: "server".to_string(),
            principal: String::new(),
            auth: HashMap::new(),
            capabilities,
            error: None,
        }
    }

    pub fn client() -> Self {
        Self {
            v: PROTOCOL_VERSION,
            role: "client".to_string(),
            principal: String::new(),
            auth: HashMap::new(),
            capabilities: HashMap::new(),
            error: None,
        }
    }
}

impl ReqFrame {
    pub fn new(
        id: u64,
        method: impl Into<String>,
        params: HashMap<String, serde_json::Value>,
    ) -> Self {
        Self {
            v: PROTOCOL_VERSION,
            id,
            method: method.into(),
            stream: false,
            params,
        }
    }
}

impl ResFrame {
    pub fn ok(id: u64, result: serde_json::Value) -> Self {
        Self {
            v: PROTOCOL_VERSION,
            id,
            seq: 0,
            end: true,
            result: Some(result),
            error: None,
        }
    }

    pub fn error(id: u64, code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            v: PROTOCOL_VERSION,
            id,
            seq: 0,
            end: true,
            result: None,
            error: Some(ErrorInfo::new(code, message)),
        }
    }
}

fn rmpv_to_json(val: rmpv::Value) -> serde_json::Value {
    match val {
        rmpv::Value::Nil => serde_json::Value::Null,
        rmpv::Value::Boolean(b) => serde_json::Value::Bool(b),
        rmpv::Value::Integer(i) => {
            if let Some(v) = i.as_i64() {
                serde_json::Value::Number(v.into())
            } else if let Some(v) = i.as_u64() {
                serde_json::Value::Number(v.into())
            } else {
                serde_json::Value::Null
            }
        }
        rmpv::Value::F32(f) => serde_json::Number::from_f64(f as f64)
            .map(serde_json::Value::Number)
            .unwrap_or(serde_json::Value::Null),
        rmpv::Value::F64(f) => serde_json::Number::from_f64(f)
            .map(serde_json::Value::Number)
            .unwrap_or(serde_json::Value::Null),
        rmpv::Value::String(s) => match s.as_str() {
            Some(st) => serde_json::Value::String(st.to_string()),
            None => serde_json::Value::String(data_encoding::BASE64.encode(s.as_bytes())),
        },
        rmpv::Value::Binary(bytes) => {
            serde_json::Value::String(data_encoding::BASE64.encode(&bytes))
        }
        rmpv::Value::Array(items) => {
            serde_json::Value::Array(items.into_iter().map(rmpv_to_json).collect())
        }
        rmpv::Value::Map(entries) => {
            let mut map = serde_json::Map::new();
            for (k, v) in entries {
                let key_str = match &k {
                    rmpv::Value::String(s) => s
                        .as_str()
                        .map(|str_val| str_val.to_string())
                        .unwrap_or_else(|| k.to_string()),
                    _ => k.to_string(),
                };
                map.insert(key_str, rmpv_to_json(v));
            }
            serde_json::Value::Object(map)
        }
        rmpv::Value::Ext(_, bytes) => {
            serde_json::Value::String(data_encoding::BASE64.encode(&bytes))
        }
    }
}

impl Frame {
    pub fn to_msgpack(&self) -> Result<Vec<u8>, rmp_serde::encode::Error> {
        rmp_serde::to_vec_named(self)
    }

    pub fn from_msgpack(bytes: &[u8]) -> anyhow::Result<Self> {
        let rmp_val = rmpv::decode::read_value(&mut &bytes[..])
            .map_err(|e| anyhow::anyhow!("failed to read msgpack value: {}", e))?;
        let json_val = rmpv_to_json(rmp_val);
        serde_json::from_value(json_val)
            .map_err(|e| anyhow::anyhow!("failed to deserialize frame from value: {}", e))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hello_frame_roundtrip() {
        let frame = Frame::Hello(HelloFrame {
            v: 1,
            role: "client".to_string(),
            principal: "test-principal".to_string(),
            auth: HashMap::new(),
            capabilities: HashMap::new(),
            error: None,
        });

        let bytes = frame.to_msgpack().expect("pack failed");
        let decoded: Frame = Frame::from_msgpack(&bytes).expect("unpack failed");
        assert_eq!(frame, decoded);
    }

    #[test]
    fn test_python_wire_interop() {
        // Hex output generated by Python's msgpack.packb for HelloFrame:
        let hex_str = "86a17601a474797065a568656c6c6fa4726f6c65a6636c69656e74a97072696e636970616ca3616263a46175746880ac6361706162696c697469657380";
        let bytes: Vec<u8> = (0..hex_str.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&hex_str[i..i + 2], 16).unwrap())
            .collect();

        let frame = Frame::from_msgpack(&bytes).expect("unpack python wire bytes failed");
        match frame {
            Frame::Hello(hello) => {
                assert_eq!(hello.v, 1);
                assert_eq!(hello.role, "client");
                assert_eq!(hello.principal, "abc");
            }
            _ => panic!("Expected Hello frame"),
        }
    }

    #[test]
    fn test_python_res_with_binary() {
        let hex_str = "86a474797065a3726573a17601a2696401a373657100a3656e64c3a6726573756c7482a464617461c40489504e47a6666f726d6174a3706e67";
        let bytes: Vec<u8> = (0..hex_str.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&hex_str[i..i + 2], 16).unwrap())
            .collect();

        let frame_res = Frame::from_msgpack(&bytes);
        println!("Result: {:?}", frame_res);
        assert!(
            frame_res.is_ok(),
            "Failed to unpack python res frame with binary: {:?}",
            frame_res.err()
        );
    }
}
