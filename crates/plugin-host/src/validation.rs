//! Structured manifest problems: a stable code, the JSON path of the value at fault and a
//! message for the author. The codes are specified in docs/extensions/specification.md;
//! a published code never changes meaning.
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
pub enum ValidationCode {
    #[serde(rename = "EXTENSION_INVALID_JSON")]
    InvalidJson,
    #[serde(rename = "EXTENSION_TOO_LARGE")]
    TooLarge,
    #[serde(rename = "EXTENSION_UNKNOWN_FIELD")]
    UnknownField,
    #[serde(rename = "EXTENSION_INVALID_FIELD")]
    InvalidField,
    #[serde(rename = "EXTENSION_INVALID_ID")]
    InvalidId,
    #[serde(rename = "EXTENSION_RESERVED_ID")]
    ReservedId,
    #[serde(rename = "EXTENSION_INVALID_SIGNATURE")]
    InvalidSignature,
    #[serde(rename = "EXTENSION_INVALID_VERSION")]
    InvalidVersion,
    #[serde(rename = "EXTENSION_API_INCOMPATIBLE")]
    ApiIncompatible,
    #[serde(rename = "EXTENSION_INVALID_VALUE")]
    InvalidValue,
    #[serde(rename = "EXTENSION_DUPLICATE_IDENTIFIER")]
    DuplicateIdentifier,
    #[serde(rename = "EXTENSION_PERMISSION_MISMATCH")]
    PermissionMismatch,
    #[serde(rename = "EXTENSION_UNSUPPORTED_TARGET")]
    UnsupportedTarget,
    #[serde(rename = "EXTENSION_INVALID_BINDING")]
    InvalidBinding,
    #[serde(rename = "EXTENSION_UNRESOLVED_CAPABILITY")]
    UnresolvedCapability,
    #[serde(rename = "EXTENSION_UNRESOLVED_PAGE")]
    UnresolvedPage,
    #[serde(rename = "EXTENSION_INVALID_KIND")]
    InvalidKind,
}

impl ValidationCode {
    pub const ALL: &'static [Self] = &[
        Self::InvalidJson,
        Self::TooLarge,
        Self::UnknownField,
        Self::InvalidField,
        Self::InvalidId,
        Self::ReservedId,
        Self::InvalidSignature,
        Self::InvalidVersion,
        Self::ApiIncompatible,
        Self::InvalidValue,
        Self::DuplicateIdentifier,
        Self::PermissionMismatch,
        Self::UnsupportedTarget,
        Self::InvalidBinding,
        Self::UnresolvedCapability,
        Self::UnresolvedPage,
        Self::InvalidKind,
    ];

    /// The wire name, as serialized.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::InvalidJson => "EXTENSION_INVALID_JSON",
            Self::TooLarge => "EXTENSION_TOO_LARGE",
            Self::UnknownField => "EXTENSION_UNKNOWN_FIELD",
            Self::InvalidField => "EXTENSION_INVALID_FIELD",
            Self::InvalidId => "EXTENSION_INVALID_ID",
            Self::ReservedId => "EXTENSION_RESERVED_ID",
            Self::InvalidSignature => "EXTENSION_INVALID_SIGNATURE",
            Self::InvalidVersion => "EXTENSION_INVALID_VERSION",
            Self::ApiIncompatible => "EXTENSION_API_INCOMPATIBLE",
            Self::InvalidValue => "EXTENSION_INVALID_VALUE",
            Self::DuplicateIdentifier => "EXTENSION_DUPLICATE_IDENTIFIER",
            Self::PermissionMismatch => "EXTENSION_PERMISSION_MISMATCH",
            Self::UnsupportedTarget => "EXTENSION_UNSUPPORTED_TARGET",
            Self::InvalidBinding => "EXTENSION_INVALID_BINDING",
            Self::UnresolvedCapability => "EXTENSION_UNRESOLVED_CAPABILITY",
            Self::UnresolvedPage => "EXTENSION_UNRESOLVED_PAGE",
            Self::InvalidKind => "EXTENSION_INVALID_KIND",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ValidationError {
    pub code: ValidationCode,
    /// From the manifest root: `.` between fields and `[i]` for array elements, for
    /// example `contributions.pages[2].capability`. Empty when the whole manifest is at fault.
    pub path: String,
    pub message: String,
}

impl ValidationError {
    pub fn new(code: ValidationCode, path: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code,
            path: path.into(),
            message: message.into(),
        }
    }
}

impl fmt::Display for ValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let path = if self.path.is_empty() {
            "manifest"
        } else {
            &self.path
        };
        write!(f, "{path}: {} ({})", self.message, self.code.as_str())
    }
}

/// Every problem found in one manifest, one per line when displayed.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ValidationErrors(pub Vec<ValidationError>);

impl ValidationErrors {
    pub fn push(
        &mut self,
        code: ValidationCode,
        path: impl Into<String>,
        message: impl Into<String>,
    ) {
        self.0.push(ValidationError::new(code, path, message));
    }

    /// `Ok` when nothing was found.
    pub fn into_result(self) -> Result<(), Self> {
        if self.0.is_empty() {
            Ok(())
        } else {
            Err(self)
        }
    }
}

impl fmt::Display for ValidationErrors {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (index, error) in self.0.iter().enumerate() {
            if index > 0 {
                writeln!(f)?;
            }
            write!(f, "{error}")?;
        }
        Ok(())
    }
}

impl std::error::Error for ValidationErrors {}

impl From<ValidationError> for ValidationErrors {
    fn from(error: ValidationError) -> Self {
        Self(vec![error])
    }
}

impl From<ValidationErrors> for String {
    fn from(errors: ValidationErrors) -> Self {
        errors.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_names_match_the_wire_names() {
        for &code in ValidationCode::ALL {
            assert_eq!(serde_json::to_value(code).unwrap(), code.as_str());
        }
    }
}
