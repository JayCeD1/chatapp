// A typed application error carried across the Tauri command boundary. It serializes to
// `{ "code": "<snake_case>", "message": "<human text>" }`, so the frontend can switch on a
// stable `code` (e.g. show a specific hint for "conflict") while still having a message to
// display. `From` conversions let existing `?`-based code keep working: a bare String or a
// sqlx error becomes an `Internal`/`Db` error with its text preserved.

use serde::Serialize;

pub type AppResult<T> = Result<T, AppError>;

#[derive(Debug, thiserror::Error, Serialize)]
#[serde(tag = "code", content = "message", rename_all = "snake_case")]
pub enum AppError {
    /// Bad input (length/format/empty) — the user should correct and retry.
    #[error("{0}")]
    Validation(String),
    /// A uniqueness/duplicate collision (e.g. a channel name already taken).
    #[error("{0}")]
    Conflict(String),
    /// Authentication / authorization failure (wrong password, not a member).
    #[error("{0}")]
    Auth(String),
    /// A networking/transport failure (couldn't connect, send, or discover).
    #[error("{0}")]
    Network(String),
    /// A database error.
    #[error("{0}")]
    Db(String),
    /// Anything else / unclassified.
    #[error("{0}")]
    Internal(String),
}

impl From<String> for AppError {
    fn from(s: String) -> Self {
        AppError::Internal(s)
    }
}

impl From<&str> for AppError {
    fn from(s: &str) -> Self {
        AppError::Internal(s.to_string())
    }
}

impl From<sqlx::Error> for AppError {
    fn from(e: sqlx::Error) -> Self {
        AppError::Db(e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serializes_as_code_and_message() {
        let json = serde_json::to_string(&AppError::Conflict("taken".into())).unwrap();
        assert_eq!(json, r#"{"code":"conflict","message":"taken"}"#);
        let json = serde_json::to_string(&AppError::Validation("too long".into())).unwrap();
        assert_eq!(json, r#"{"code":"validation","message":"too long"}"#);
    }

    #[test]
    fn string_converts_to_internal() {
        let e: AppError = "boom".to_string().into();
        assert!(matches!(e, AppError::Internal(_)));
    }
}
