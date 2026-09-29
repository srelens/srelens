use serde_json::Value;
use srelens_sidecar_protocol::{code, ContextError, RpcError};
use std::fmt;

/// What a handler fails with: the JSON-RPC error srelens is answered with.
#[derive(Debug, Clone, PartialEq)]
pub struct Error(RpcError);

impl Error {
    pub fn new(code: i64, message: impl Into<String>) -> Error {
        Error(RpcError::new(code, message))
    }

    /// The input was wrong: `-32602`.
    pub fn invalid_params(message: impl Into<String>) -> Error {
        Error::new(code::INVALID_PARAMS, message)
    }

    /// Anything else that went wrong: `-32603`.
    pub fn internal(message: impl Into<String>) -> Error {
        Error::new(code::INTERNAL_ERROR, message)
    }

    pub fn with_data(mut self, data: Value) -> Error {
        self.0.data = Some(data);
        self
    }

    pub fn code(&self) -> i64 {
        self.0.code
    }

    pub fn message(&self) -> &str {
        &self.0.message
    }

    pub(crate) fn into_rpc(self) -> RpcError {
        self.0
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl std::error::Error for Error {}

impl From<serde_json::Error> for Error {
    fn from(error: serde_json::Error) -> Error {
        Error::internal(error.to_string())
    }
}

/// An operation's own input named a cluster srelens would refuse.
impl From<ContextError> for Error {
    fn from(error: ContextError) -> Error {
        Error::invalid_params(error.to_string())
    }
}

/// Why a session with srelens ended other than as it should.
#[derive(Debug)]
pub enum SidecarError {
    /// Reading stdin or writing stdout failed.
    Io(std::io::Error),
    /// srelens wrote a line that is not a JSON-RPC message.
    Protocol(String),
}

impl fmt::Display for SidecarError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SidecarError::Io(e) => write!(f, "the pipe to srelens failed: {e}"),
            SidecarError::Protocol(why) => {
                write!(f, "srelens wrote a line that is not JSON-RPC: {why}")
            }
        }
    }
}

impl std::error::Error for SidecarError {}
