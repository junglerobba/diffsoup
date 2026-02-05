use error_stack::Report;
use jj_cli::command_error::CommandError;
use std::{error::Error, fmt::Display};

pub type Result<T> = core::result::Result<T, Report<CustomError>>;

#[derive(Debug)]
pub enum CustomError {
    RepoError,
    UrlError,
    RequestError,
    ExprError,
    ConfigError,
    CommitError(String),
    ProcessError(String),
}

impl Error for CustomError {}

impl Display for CustomError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::RepoError => write!(f, "Repo Error"),
            Self::UrlError => write!(f, "URL Error"),
            Self::RequestError => write!(f, "Request Error"),
            Self::ExprError => write!(f, "Expr Error"),
            Self::ConfigError => write!(f, "Config Error"),
            Self::CommitError(msg) => write!(f, "Commit Error: {msg}"),
            Self::ProcessError(msg) => write!(f, "Process error: {msg}"),
        }
    }
}

impl From<CommandError> for CustomError {
    fn from(value: CommandError) -> Self {
        CustomError::ProcessError(format!("{:#?}", value))
    }
}
