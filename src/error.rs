use error_stack::{Report, ResultExt};
use jj_cli::command_error::CommandError;
use reqwest::blocking::{RequestBuilder, Response};
use std::{error::Error, fmt::Display};

pub type Result<T> = core::result::Result<T, Report<CustomError>>;

#[derive(Debug)]
pub enum CustomError {
    RepoError,
    UrlError(String),
    RequestError,
    ResponseError {
        url: String,
        status: http::StatusCode,
        body: String,
    },
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
            Self::UrlError(msg) => write!(f, "URL Error: {msg}"),
            Self::RequestError => write!(f, "Request Error"),
            Self::ResponseError { url, status, body } => {
                write!(f, "Response Error ({url}): {status} -> {body}")
            }
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

pub trait SendChecked {
    fn send_checked(self) -> Result<Response>;
}

impl SendChecked for RequestBuilder {
    fn send_checked(self) -> Result<Response> {
        let res = self.send().change_context(CustomError::RequestError)?;
        if res.status().is_success() {
            Ok(res)
        } else {
            let url = res.url().as_str().to_string();
            let status = res.status();
            let body = res
                .text()
                .unwrap_or_else(|e| format!("failed to read body: {e}"));

            Err(CustomError::ResponseError { url, status, body }.into())
        }
    }
}
