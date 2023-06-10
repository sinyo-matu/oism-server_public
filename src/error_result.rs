use axum::{
    http::{uri::InvalidUri, StatusCode},
    response::{IntoResponse, Redirect},
};
use base64::{engine::general_purpose, Engine as _};
use reqwest::Response;
use serde::de::DeserializeOwned;
use thiserror::Error;
use tokio::task::JoinError;
use tracing::{error, instrument, warn};

use crate::db::{auth::UserRole, order::OrderValidateError};

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Error)]
pub enum Error {
    #[error(transparent)]
    StdIo(#[from] std::io::Error),
    #[error(transparent)]
    Mongodb(#[from] mongodb::error::Error),
    #[error("can not find inventory item {0}")]
    InventoryItemNotFound(String),
    #[error("can not find order {0}")]
    OrderNotFound(String),
    #[error("can not find transfer {0}")]
    TransferNotFound(String),
    #[error("can not find order item {0}")]
    OrderItemNotFound(String),
    #[error("OrderItemIsConcealed")]
    OrderItemIsConcealed,
    #[error("clearance vendor should match a paid location")]
    VenderLocationNotMatch,
    #[error("requested backward count is large than inventory operation for backward:{0} operation count {1}")]
    PartialBackwardCountOver(u32, u32),
    #[error("can not find inventory operation {0}")]
    CanNotFindOperation(String),
    #[error("RegisterCanNotDelete")]
    RegisterCanNotDelete,
    #[error("Changed")]
    Changed,
    #[error("OrderCanNotDelete")]
    OrderCanNotDelete,
    #[error("InventoryNotFound")]
    InventoryNotFound,
    #[error(transparent)]
    Uuid(#[from] mongodb::bson::uuid::Error),
    #[error(transparent)]
    SerdeJsonBody(#[from] serde_json::Error),
    #[error(transparent)]
    BsonDe(#[from] mongodb::bson::de::Error),
    #[error("ItemTypeNotPrepared")]
    ItemTypeNotPrepared,
    #[error(transparent)]
    Auth(AuthError),
    #[error("tokio handler error")]
    TokioHandler(#[from] JoinError),
    #[error("filename not show in a file path")]
    FilenameNotShow,
    #[error(transparent)]
    InvalidUri(#[from] InvalidUri),
    #[error(transparent)]
    OrderValidate(#[from] OrderValidateError),
    #[error(transparent)]
    HttpRequest(#[from] reqwest::Error),
    #[error("http response error : {0}")]
    HttpResponse(String),
    #[error("InvalidOperation")]
    InvalidOperation,
    #[error("Path not found")]
    PathNotFound,
}

impl IntoResponse for Error {
    #[instrument(name = "change error into response", skip(self))]
    fn into_response(self) -> axum::response::Response {
        error!("got error raw : {self:?}, message:{self}");
        let (status, message) = match self {
            Error::TransferNotFound(transfer) => (
                StatusCode::NOT_FOUND,
                format!("transfer id: {transfer} not found"),
            ),
            Error::InventoryItemNotFound(item) => (
                StatusCode::NOT_FOUND,
                format!("inventory item {} not found", item),
            ),
            Error::OrderItemIsConcealed => (
                StatusCode::FORBIDDEN,
                String::from("order item has been changed"),
            ),
            Error::Changed => (
                StatusCode::BAD_REQUEST,
                String::from("requested has been changed"),
            ),
            Error::InvalidOperation => (StatusCode::BAD_REQUEST, String::from("InvalidOperation")),
            Error::OrderValidate(e) => (StatusCode::BAD_REQUEST, format!("{e}")),
            Error::VenderLocationNotMatch => (StatusCode::BAD_REQUEST, format!("{self}")),
            Error::PathNotFound => (StatusCode::NOT_FOUND, format!("{self}")),
            Error::Auth(e) => match e {
                AuthError::CookieHeaderNotFound => (
                    StatusCode::UNAUTHORIZED,
                    String::from("CookieHeaderNotFound"),
                ),
                AuthError::InvalidSignupSecret => (
                    StatusCode::BAD_REQUEST,
                    String::from("invalid signup secret"),
                ),
                AuthError::UsernameOccupied => (
                    StatusCode::BAD_REQUEST,
                    String::from("username is occupied"),
                ),
                AuthError::UserNotFound => (
                    StatusCode::UNAUTHORIZED,
                    String::from("username is not found"),
                ),
                AuthError::InvalidPassword => {
                    (StatusCode::UNAUTHORIZED, String::from("invalid password"))
                }
                AuthError::JWTError(e) => (StatusCode::UNAUTHORIZED, format!("InvalidToken:{e}")),
                AuthError::JWTTokenNotFound => {
                    (StatusCode::UNAUTHORIZED, String::from("TokenNotFound"))
                }
                AuthError::TokenNeedRefresh => {
                    (StatusCode::UNAUTHORIZED, String::from("TokenNeedRefresh"))
                }
                AuthError::JWTTokenNeedRefresh(uri) => {
                    let encoded = general_purpose::STANDARD_NO_PAD.encode(uri.as_bytes());
                    let path = format!("/api/v1/public/refresh_token?uri={}", encoded);
                    return Redirect::temporary(&path).into_response();
                }
                AuthError::PermissionNotEnough { got, need } => {
                    error!(
                        "Got permission Error user got :{:?} but need :{}",
                        got, need
                    );
                    (StatusCode::FORBIDDEN, String::from("PermissionNotEnough"))
                }
                _ => (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    String::from("Internal server error"),
                ),
            },
            _ => (
                StatusCode::INTERNAL_SERVER_ERROR,
                String::from("Internal server error"),
            ),
        };
        error!("returning error message:{message}");

        (status, message).into_response()
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum AuthError {
    #[error("invalid signup secret")]
    InvalidSignupSecret,
    #[error("cookie header is not found")]
    CookieHeaderNotFound,
    #[error("username is occupied")]
    UsernameOccupied,
    #[error("user not found")]
    UserNotFound,
    #[error("invalid password")]
    InvalidPassword,
    #[error("password process got error: {0:?}")]
    PasswordHashProcess(pbkdf2::password_hash::Error),
    #[error(transparent)]
    JWTError(#[from] jsonwebtoken::errors::Error),
    #[error("token not found")]
    JWTTokenNotFound,
    #[error("token need refresh")]
    JWTTokenNeedRefresh(String),
    #[error("TokenNeedRefresh")]
    TokenNeedRefresh,
    #[error("PermissionNotEnough")]
    PermissionNotEnough {
        got: Option<UserRole>,
        need: UserRole,
    },
}
impl From<pbkdf2::password_hash::Error> for AuthError {
    fn from(e: pbkdf2::password_hash::Error) -> Self {
        Self::PasswordHashProcess(e)
    }
}

pub async fn validate_http_response<T: DeserializeOwned>(resp: Response) -> Result<T> {
    if resp.status() != 200 {
        let text = resp.text().await?;
        return Err(Error::HttpResponse(text));
    }
    Ok(resp.json().await?)
}
