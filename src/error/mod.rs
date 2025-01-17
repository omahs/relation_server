use lambda_http::http::StatusCode;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum Error {
    // general
    #[error("{0}")]
    General(String, StatusCode),
    // http
    #[error("Param missing: {0}")]
    ParamMissing(String),
    #[error("Param error: {0}")]
    ParamError(String),
    #[error("No body provided")]
    BodyMissing,
    #[error("No result")]
    NoResult,
    #[error("JSON parse error: {0}")]
    JSONParseError(#[from] serde_json::error::Error),
    #[error("HTTP general error")]
    HttpError(#[from] lambda_http::http::Error),
    #[error("Config error: {0}")]
    ConfigError(#[from] config::ConfigError),
    #[error("Database error: {0}")]
    SignatureValidationError(String),
    #[error("Hex parse error: {0}")]
    HttpClientError(#[from] hyper::Error),
    #[error("UUID parse error: {0}")]
    UuidError(#[from] uuid::Error),
    #[error("ArangoDB error: {0}")]
    ArangoDBError(#[from] aragog::Error),
    #[error("ArangoLiteDB error: {0}")]
    ArangoLiteDBError(#[from] arangors_lite::ClientError),
    #[error("Parse error: {0}")]
    EnumParseError(#[from] strum::ParseError),
    #[error("Parse Int error: {0}")]
    ParseIntError(#[from] std::num::ParseIntError),
    #[error("GraphQL error: {0}")]
    GraphQLError(String),
    #[error("PoolError error: {0}")]
    PoolError(String),
    #[error("ArangoConfigError error: {0}")]
    ArangoConfigError(#[from] crate::graph::arangopool::ArangoConfigError),
}

impl Error {
    pub fn http_status(&self) -> StatusCode {
        match self {
            Error::General(_, status) => *status,
            Error::ParamMissing(_) => StatusCode::BAD_REQUEST,
            Error::ParamError(_) => StatusCode::BAD_REQUEST,
            Error::BodyMissing => StatusCode::BAD_REQUEST,
            Error::JSONParseError(_) => StatusCode::BAD_REQUEST,
            Error::NoResult => StatusCode::BAD_REQUEST,
            Error::HttpError(_) => StatusCode::INTERNAL_SERVER_ERROR,
            Error::ConfigError(_) => StatusCode::INTERNAL_SERVER_ERROR,
            Error::HttpClientError(_) => StatusCode::INTERNAL_SERVER_ERROR,
            Error::SignatureValidationError(_) => StatusCode::BAD_REQUEST,
            Error::ArangoDBError(_) => StatusCode::INTERNAL_SERVER_ERROR,
            Error::EnumParseError(_) => StatusCode::BAD_REQUEST,
            Error::GraphQLError(_) => StatusCode::INTERNAL_SERVER_ERROR,
            Error::ParseIntError(_) => StatusCode::INTERNAL_SERVER_ERROR,
            Error::UuidError(_) => StatusCode::BAD_REQUEST,
            Error::ArangoLiteDBError(_) => StatusCode::INTERNAL_SERVER_ERROR,
            Error::PoolError(_) => StatusCode::INTERNAL_SERVER_ERROR,
            Error::ArangoConfigError(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

impl warp::reject::Reject for Error {}

unsafe impl Sync for Error {}
unsafe impl Send for Error {}

pub type Result<T> = std::result::Result<T, Error>;
