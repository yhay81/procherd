use std::{io, process::ExitCode};

use schemars::JsonSchema;
use serde::Serialize;
use thiserror::Error;

#[derive(Debug, Error)]
#[error("{message}")]
pub struct AppError {
    pub code: u8,
    pub kind: &'static str,
    pub message: String,
}

impl AppError {
    pub fn usage(message: impl Into<String>) -> Self {
        Self {
            code: 2,
            kind: "usage",
            message: message.into(),
        }
    }

    pub fn operational(kind: &'static str, message: impl Into<String>) -> Self {
        Self {
            code: 1,
            kind,
            message: message.into(),
        }
    }

    pub fn timeout(message: impl Into<String>) -> Self {
        Self {
            code: 3,
            kind: "timeout",
            message: message.into(),
        }
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        Self {
            code: 4,
            kind: "not_found",
            message: message.into(),
        }
    }

    pub fn integrity(message: impl Into<String>) -> Self {
        Self {
            code: 5,
            kind: "integrity",
            message: message.into(),
        }
    }

    pub fn exit_code(&self) -> ExitCode {
        ExitCode::from(self.code)
    }
}

impl From<io::Error> for AppError {
    fn from(error: io::Error) -> Self {
        Self::operational("io", error.to_string())
    }
}

impl From<serde_json::Error> for AppError {
    fn from(error: serde_json::Error) -> Self {
        Self::integrity(error.to_string())
    }
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ErrorDocument<'a> {
    pub schema_version: &'static str,
    pub error: ErrorBody<'a>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ErrorBody<'a> {
    pub kind: &'a str,
    pub message: &'a str,
    pub exit_code: u8,
}

impl<'a> From<&'a AppError> for ErrorDocument<'a> {
    fn from(error: &'a AppError) -> Self {
        Self {
            schema_version: "procherd.error.v1",
            error: ErrorBody {
                kind: error.kind,
                message: &error.message,
                exit_code: error.code,
            },
        }
    }
}
