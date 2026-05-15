use serde::Serialize;

#[derive(Debug, thiserror::Error, Serialize)]
pub enum CommandError {
    #[error("Ej inloggad")]
    NotAuthenticated,

    #[error("Åtkomst nekad")]
    Forbidden,

    #[error("Valideringsfel: {0}")]
    Validation(String),

    #[error("Ej funnet: {0}")]
    NotFound(String),

    #[error("Databasfel: {0}")]
    Database(String),

    #[error("Internt fel: {0}")]
    Internal(String),
}

impl From<sqlx::Error> for CommandError {
    fn from(e: sqlx::Error) -> Self {
        CommandError::Database(e.to_string())
    }
}
