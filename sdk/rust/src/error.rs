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
#[non_exhaustive]
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

/// Why a call to srelens failed.
///
/// `#[non_exhaustive]`: a new variant here should not break every match an
/// author wrote against this enum, so matching it from outside this crate
/// needs a wildcard arm.
///
/// ```compile_fail
/// use srelens_sidecar::HostError;
///
/// fn describe(error: HostError) -> &'static str {
///     match error {
///         HostError::ConsentDenied(_) => "denied",
///         HostError::CapabilityFailed(_) => "failed",
///         HostError::InvalidParams(_) => "invalid",
///         HostError::InvalidCall(_) => "not sent",
///         HostError::Cancelled => "cancelled",
///         HostError::Rpc(_) => "rpc",
///         HostError::TooLarge(_) => "too large",
///         HostError::Disconnected => "disconnected",
///     }
/// }
/// ```
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum HostError {
    /// A person declined, or no one could be asked (`-32002`). Nothing ran.
    ConsentDenied(String),
    /// srelens or the cluster refused the call or failed it (`-32003`). The
    /// message is theirs.
    CapabilityFailed(String),
    /// srelens refused the call's params (`-32602`).
    InvalidParams(String),
    /// A field of the call is one srelens would refuse, so the call was not
    /// sent: no slot was taken and srelens did not see it. The message names
    /// the field and the shape it must have. A handler that passes this on
    /// with `?` fails with `-32603`: a call the sidecar built wrong is the
    /// sidecar's bug, not a refusal from srelens ([`HostError::InvalidParams`]).
    InvalidCall(String),
    /// The call was cancelled (`-32800`).
    Cancelled,
    /// Any other error srelens answered with.
    Rpc(RpcError),
    /// The call serialized to this many bytes, over the message limit, so it
    /// was never sent. srelens did not see it, and the session goes on.
    TooLarge(usize),
    /// The session with srelens ended before it answered.
    Disconnected,
}

impl HostError {
    pub(crate) fn from_rpc(error: RpcError) -> HostError {
        match error.code {
            code::CONSENT_DENIED => HostError::ConsentDenied(error.message),
            code::CAPABILITY_FAILED => HostError::CapabilityFailed(error.message),
            code::INVALID_PARAMS => HostError::InvalidParams(error.message),
            code::REQUEST_CANCELLED => HostError::Cancelled,
            _ => HostError::Rpc(error),
        }
    }
}

impl fmt::Display for HostError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            HostError::ConsentDenied(why) => write!(f, "srelens was not given consent: {why}"),
            HostError::CapabilityFailed(why) => write!(f, "srelens refused the call: {why}"),
            HostError::InvalidParams(why) => write!(f, "srelens refused the call's params: {why}"),
            HostError::InvalidCall(why) => write!(f, "the call was not sent: {why}"),
            HostError::Cancelled => f.write_str("the call to srelens was cancelled"),
            HostError::Rpc(error) => write!(f, "srelens answered with an error: {error}"),
            HostError::TooLarge(bytes) => write!(
                f,
                "the call is {bytes} bytes, over the {} a message may be, so it was not sent",
                crate::outbox::limit_text()
            ),
            HostError::Disconnected => f.write_str("the session with srelens ended"),
        }
    }
}

impl std::error::Error for HostError {}

/// A handler's host call failed: the handler fails with srelens's words.
impl From<HostError> for Error {
    fn from(error: HostError) -> Error {
        Error::internal(error.to_string())
    }
}

/// Why [`crate::Frames::send`] did not send.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum StreamClosed {
    /// srelens cancelled the stream.
    Cancelled,
    /// The frame is this many bytes, over the message limit.
    TooLarge(usize),
    /// The stream's handler already returned, and its terminal frame was
    /// sent: a clone of [`crate::Frames`] kept past the handler sends nothing.
    Finished,
    /// The session ended.
    Ended,
    /// The frame could not be serialized; the serializer's message.
    Invalid(String),
}

impl fmt::Display for StreamClosed {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StreamClosed::Cancelled => f.write_str("srelens cancelled the stream"),
            StreamClosed::TooLarge(bytes) => write!(
                f,
                "the frame is {bytes} bytes, over the {} a message may be",
                crate::outbox::limit_text()
            ),
            StreamClosed::Finished => f.write_str("the stream already ended: its handler returned"),
            StreamClosed::Ended => f.write_str("the session with srelens ended"),
            StreamClosed::Invalid(why) => write!(f, "the frame could not be serialized: {why}"),
        }
    }
}

impl std::error::Error for StreamClosed {}

impl From<StreamClosed> for Error {
    fn from(closed: StreamClosed) -> Error {
        Error::internal(closed.to_string())
    }
}
