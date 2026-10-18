//! Versioned VM-provider C boundary. Only fixed-width scalars, byte buffers,
//! JSON messages and opaque handles cross DLLs. Lua/Rust objects never do.
use crate::platform::Ecosystem;
use serde::{Deserialize, Serialize};

pub const ABI_VERSION: u32 = 1;
pub const MESSAGE_VERSION: u32 = 3;
pub const ENGINE_LUAJIT: u32 = 2;
pub const MAX_REQUEST_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_RESPONSE_BYTES: usize = 1024 * 1024;
pub const OK: i32 = 0;
pub const BAD_ARGUMENT: i32 = 1;
pub const BUFFER_TOO_SMALL: i32 = 2;
pub const UNKNOWN_RESPONSE: i32 = 3;
pub const CAPACITY_EXCEEDED: i32 = 4;
pub const PANIC: i32 = 5;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct ProviderApiV1 {
    pub struct_bytes: u32,
    pub abi_version: u32,
    pub pointer_bits: u32,
    pub engine: u32,
    pub request: Option<unsafe extern "system" fn(*const u8, u32, *mut u64) -> i32>,
    pub read_response: Option<unsafe extern "system" fn(u64, *mut u8, u32, *mut u32) -> i32>,
    pub release_response: Option<unsafe extern "system" fn(u64) -> i32>,
    pub counts: Option<unsafe extern "system" fn(*mut u32, *mut u32) -> i32>,
}

impl ProviderApiV1 {
    pub fn validate(&self) -> Result<(), String> {
        if self.struct_bytes != std::mem::size_of::<Self>() as u32
            || self.abi_version != ABI_VERSION
            || self.pointer_bits != usize::BITS
            || self.engine != ENGINE_LUAJIT
        {
            return Err("Incompatible VM provider ABI, engine or architecture".into());
        }
        if self.request.is_none()
            || self.read_response.is_none()
            || self.release_response.is_none()
            || self.counts.is_none()
        {
            return Err("VM provider has missing entry points".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum Value {
    Nil,
    Boolean(bool),
    Number(f64),
    Bytes(Vec<u8>),
    Unsupported(String),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub version: u32,
    pub operation: Operation,
}
impl Request {
    pub fn new(operation: Operation) -> Self {
        Self {
            version: MESSAGE_VERSION,
            operation,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub enum Operation {
    Create {
        name: String,
        runtime_path: String,
        addon_path: String,
        source: String,
        ecosystem: Ecosystem,
    },
    Emit {
        handle: u64,
        event: String,
        arguments: Vec<Value>,
    },
    EmitBlocking {
        handle: u64,
        event: String,
        arguments: Vec<Value>,
        blocked_at: usize,
        blocked: bool,
    },
    Frame {
        handle: u64,
        index: u32,
    },
    Poll {
        handle: u64,
        seconds: f64,
    },
    TakeActions {
        handle: u64,
    },
    Diagnostics {
        handle: u64,
    },
    Destroy {
        handle: u64,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Action {
    GameInput {
        origin: Ecosystem,
        mode: i32,
        bytes: Vec<u8>,
    },
    WindowerCommand {
        bytes: Vec<u8>,
    },
    NativeCommand {
        bytes: Vec<u8>,
    },
    AshitaCommand {
        mode: i32,
        bytes: Vec<u8>,
    },
    Chat {
        mode: i32,
        bytes: Vec<u8>,
    },
}
#[derive(Debug, Serialize, Deserialize)]
pub struct Callback {
    pub id: u32,
    pub values: Vec<Value>,
    pub error: Option<String>,
}
#[derive(Default, Debug, Serialize, Deserialize)]
pub struct Response {
    pub ok: bool,
    pub handle: Option<u64>,
    pub callbacks: Vec<Callback>,
    pub actions: Vec<Action>,
    pub errors: Vec<String>,
    pub blocked: bool,
    pub error: Option<String>,
    #[serde(default)]
    pub scheduled: usize,
}
impl Response {
    pub fn success() -> Self {
        Self {
            ok: true,
            ..Self::default()
        }
    }
    pub fn failure(error: impl ToString) -> Self {
        Self {
            error: Some(error.to_string()),
            ..Self::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn binary_arguments_survive_the_serialized_boundary() {
        let request = Request::new(Operation::Emit {
            handle: 9,
            event: "binary".into(),
            arguments: vec![Value::Bytes(vec![0, 255, 1]), Value::Nil],
        });
        let bytes = serde_json::to_vec(&request).unwrap();
        let restored: Request = serde_json::from_slice(&bytes).unwrap();
        match restored.operation {
            Operation::Emit { arguments, .. } => {
                assert_eq!(arguments, vec![Value::Bytes(vec![0, 255, 1]), Value::Nil])
            }
            _ => panic!("Wrong operation"),
        }
    }
    #[test]
    fn mismatched_or_incomplete_vtables_are_rejected() {
        let api = ProviderApiV1 {
            struct_bytes: std::mem::size_of::<ProviderApiV1>() as u32,
            abi_version: ABI_VERSION,
            pointer_bits: usize::BITS,
            engine: ENGINE_LUAJIT,
            request: None,
            read_response: None,
            release_response: None,
            counts: None,
        };
        assert!(api.validate().is_err());
        assert!(
            ProviderApiV1 {
                abi_version: 99,
                ..api
            }
            .validate()
            .is_err()
        );
    }
}
