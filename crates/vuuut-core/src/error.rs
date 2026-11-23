use thiserror::Error;

pub type Result<T> = std::result::Result<T, GhostError>;

/// Ghost error taxonomy, mirroring the Node error classes that the
/// JSON API exposes (`errors: [{message, errorType, context}]`).
#[derive(Debug, Error)]
pub enum GhostError {
    #[error("{message}")]
    ValidationError { message: String, context: Option<String> },
    #[error("{message}")]
    NotFoundError { message: String, context: Option<String> },
    #[error("{message}")]
    UnauthorizedError { message: String, context: Option<String> },
    #[error("{message}")]
    NoPermissionError { message: String, context: Option<String> },
    #[error("{0}")]
    BadRequest(String),
    /// A limit the *host* has placed on this site has been reached — see
    /// [`crate::limits`]. Kept separate from `NoPermissionError` because the
    /// answer is different: nothing about who is asking would change it, and
    /// the only way through is to change the plan, which is what `help`
    /// points at. `details` carries Ghost's `errorDetails`: the limit's name,
    /// its ceiling and the current total, which is what lets a client say
    /// "47 of 50 staff users" rather than just refusing.
    #[error("{message}")]
    HostLimitError {
        message: String,
        details: serde_json::Value,
        help: Option<String>,
    },
    #[error("Incorrect data passed to {name}")]
    IncorrectUsage { name: String, message: String },
    #[error("Theme error: {0}")]
    ThemeError(String),
    #[error("Config error: {0}")]
    ConfigError(String),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Database error: {0}")]
    Db(String),
    #[error("{0}")]
    Render(String),
    #[error("{0}")]
    Internal(String),
    #[error("{0}")]
    EmailError(String),
    #[error("Data transfer error: {0}")]
    DataImportError(String),
}

impl GhostError {
    /// HTTP status code matching Ghost's error semantics.
    pub fn status_code(&self) -> u16 {
        match self {
            GhostError::ValidationError { .. }
            | GhostError::IncorrectUsage { .. }
            | GhostError::BadRequest(_) => 422,
            GhostError::NotFoundError { .. } => 404,
            GhostError::UnauthorizedError { .. } => 401,
            GhostError::NoPermissionError { .. } | GhostError::HostLimitError { .. } => 403,
            _ => 500,
        }
    }

    pub fn error_type(&self) -> &'static str {
        match self {
            GhostError::ValidationError { .. } => "ValidationError",
            GhostError::NotFoundError { .. } => "NotFoundError",
            GhostError::UnauthorizedError { .. } => "UnauthorizedError",
            GhostError::NoPermissionError { .. } => "NoPermissionError",
            GhostError::IncorrectUsage { .. } => "IncorrectUsageError",
            GhostError::BadRequest(_) => "BadRequestError",
            GhostError::HostLimitError { .. } => "HostLimitError",
            GhostError::ThemeError(_) => "ThemeError",
            GhostError::ConfigError(_) => "ConfigurationError",
            GhostError::Io(_) | GhostError::Db(_) | GhostError::Internal(_) => "InternalServerError",
            GhostError::Render(_) => "TemplateError",
            GhostError::EmailError(_) => "EmailError",
            GhostError::DataImportError(_) => "DataImportError",
        }
    }

    pub fn message(&self) -> String {
        match self {
            GhostError::ValidationError { message, .. } => message.clone(),
            GhostError::NotFoundError { message, .. } => message.clone(),
            GhostError::UnauthorizedError { message, .. } => message.clone(),
            GhostError::NoPermissionError { message, .. } => message.clone(),
            GhostError::BadRequest(m) => m.clone(),
            GhostError::HostLimitError { message, .. } => message.clone(),
            GhostError::IncorrectUsage { message, .. } => message.clone(),
            GhostError::ThemeError(m) => m.clone(),
            GhostError::ConfigError(m) => m.clone(),
            GhostError::Db(m) => m.clone(),
            GhostError::Render(m) => m.clone(),
            GhostError::Internal(m) => m.clone(),
            GhostError::EmailError(m) => m.clone(),
            GhostError::DataImportError(m) => m.clone(),
            GhostError::Io(e) => e.to_string(),
        }
    }

    pub fn context(&self) -> Option<&str> {
        match self {
            GhostError::ValidationError { context, .. }
            | GhostError::NotFoundError { context, .. }
            | GhostError::UnauthorizedError { context, .. }
            | GhostError::NoPermissionError { context, .. } => context.as_deref(),
            _ => None,
        }
    }

    /// Serialize to Ghost's API error shape.
    pub fn to_api_json(&self) -> serde_json::Value {
        let mut body = serde_json::json!({
            "message": self.message(),
            "errorType": self.error_type(),
            "context": self.context(),
        });
        // A host limit carries two extra fields Ghost's clients read: what the
        // limit was, and where to go about it.
        if let GhostError::HostLimitError { details, help, .. } = self {
            body["errorDetails"] = details.clone();
            if let Some(help) = help {
                body["help"] = serde_json::json!(help);
            }
        }
        body
    }
}

impl serde::Serialize for GhostError {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        self.to_api_json().serialize(serializer)
    }
}

impl From<serde_json::Error> for GhostError {
    fn from(e: serde_json::Error) -> Self {
        GhostError::BadRequest(format!("JSON error: {e}"))
    }
}

impl From<serde_yaml::Error> for GhostError {
    fn from(e: serde_yaml::Error) -> Self {
        GhostError::BadRequest(format!("YAML error: {e}"))
    }
}

impl From<hex::FromHexError> for GhostError {
    fn from(e: hex::FromHexError) -> Self {
        GhostError::BadRequest(format!("hex error: {e}"))
    }
}
