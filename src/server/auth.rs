use std::{collections::HashMap, sync::Arc};

use axum::{
    extract::{Query, State},
    headers::HeaderName,
    http::{header::SET_COOKIE, StatusCode},
    response::{AppendHeaders, IntoResponse, Redirect, Response},
    Json,
};
use base64::{engine::general_purpose, Engine as _};
use chrono::prelude::*;
use jsonwebtoken::{
    decode, encode, errors::ErrorKind as JWTErrorKind, Algorithm, DecodingKey, EncodingKey, Header,
    Validation,
};
use once_cell::sync::Lazy;
use pbkdf2::password_hash::{
    rand_core::OsRng, PasswordHash, PasswordHasher, PasswordVerifier, SaltString,
};
use pbkdf2::Pbkdf2;
use reqwest::header::CONTENT_TYPE;
use secrecy::{ExposeSecret, Secret};
use serde::{Deserialize, Serialize};
use std::str;
use tracing::{info, instrument};
use uuid::Uuid;

use crate::{
    configuration::{get_configuration, Settings},
    db::{
        auth::{User, UserRole},
        mongo::DbClient,
    },
    error_result::Result,
};
use crate::{
    db::SMTAuthDataBase,
    error_result::{AuthError, Error},
};

use super::AppPrivateRoute;

pub const ACCESS_COOKIE_NAME: &str = "smt_token";
pub const REFRESH_COOKIE_NAME: &str = "smt_id";

pub static COOKIE_ATTRIBUTE: Lazy<&'static str> = Lazy::new(|| match std::env::var("IS_TEST") {
    Ok(_) => " SameSite=Strict; Path=/; HttpOnly; Max-Age=1814400",
    Err(_) => " SameSite=Strict; Path=/; Domain=oism.app; Secure; HttpOnly; Max-Age=1814400",
});

pub static SETTINGS: Lazy<Settings> =
    Lazy::new(|| get_configuration().expect("Failed to load configuration"));

#[derive(Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct SignUpMessage {
    username: String,
    password: Secret<String>,
    role: UserRole,
    sub_role: HashMap<AppPrivateRoute, UserRole>,
    secret: String,
}

#[instrument(name = "sign up new user", skip(message, db),fields(
    request_id=%Uuid::new_v4(),
    username=%message.username,
))]
pub async fn sign_up(
    State(db): State<Arc<DbClient>>,
    Json(message): Json<SignUpMessage>,
) -> Result<impl IntoResponse> {
    if message.secret != *SETTINGS.signup_secret.expose_secret() {
        info!("{secret} is incorrect", secret = message.secret);
        return Err(Error::Auth(AuthError::InvalidSignupSecret));
    }
    if db.check_is_username_occupied(&message.username).await? {
        info!("{} is occupied", message.username);
        return Err(Error::Auth(AuthError::UsernameOccupied));
    }
    let handler = tokio::task::spawn_blocking(move || {
        generate_password_hash(message.password.expose_secret())
    });
    let password_hash = handler.await??;
    info!("create new user :{}", message.username);
    let user = User::new(
        message.username,
        password_hash,
        message.role,
        message.sub_role,
    );
    db.create_user(user).await?;
    Ok(StatusCode::CREATED)
}

#[derive(Serialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct GetUserInfoResponse {
    id: Uuid,
    username: String,
    role: UserRole,
}

pub async fn get_user_info_handler(
    user_info: UserInfo,
    State(db): State<Arc<DbClient>>,
) -> Result<Json<GetUserInfoResponse>> {
    let user = db.find_user(user_info.user_id.into()).await?;
    Ok(Json(GetUserInfoResponse {
        username: user.username,
        id: user.id.to_uuid_1(),
        role: user.role,
    }))
}

#[derive(Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct LoginMessage {
    username: String,
    password: Secret<String>,
}

#[instrument(name = "login in user", skip(message, db),fields(
    request_id=%Uuid::new_v4(),
    username=%message.username,
))]
pub async fn login(
    State(db): State<Arc<DbClient>>,
    Json(message): Json<LoginMessage>,
) -> Result<Response> {
    let user = db.find_user_by_username(&message.username).await?;
    verify_password(message.password.expose_secret(), &user.hash)?;
    info!("login {}", user.username);
    let access_token = generate_access_token(user.id.into())?;
    let refresh_token = generate_refresh_token(user.id.into())?;
    Ok(get_cookie_headers(&access_token, &refresh_token).into_response())
}

pub struct RefreshAuthInfo(pub Uuid);

#[derive(Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct RefreshTokenQuery {
    uri: Option<String>,
}

#[instrument(name = "refresh access token", skip(query, db,auth_info),fields(
    request_id=%Uuid::new_v4(),
    use_id=%auth_info.0,
))]
pub async fn token_refresh_handler(
    Query(query): Query<RefreshTokenQuery>,
    auth_info: RefreshAuthInfo,
    State(db): State<Arc<DbClient>>,
) -> Result<Response> {
    let user = db.find_user(auth_info.0.into()).await?;
    info!("user is {}", user.username);
    let access_token = generate_access_token(user.id.into())?;
    let refresh_token = generate_refresh_token(user.id.into())?;
    if let Some(uri_str) = query.uri {
        let decoded_bytes = general_purpose::URL_SAFE_NO_PAD
            .decode(uri_str.as_bytes())
            .unwrap();
        let decoded = str::from_utf8(&decoded_bytes).unwrap();
        let redirect_to_str = format!("/api/v1/private{decoded}");
        info!("redirect to provided uri :{}", uri_str);
        return Ok((
            get_cookie_headers(&access_token, &refresh_token),
            Redirect::temporary(&redirect_to_str),
        )
            .into_response());
    }
    Ok((
        get_cookie_headers(&access_token, &refresh_token),
        [(CONTENT_TYPE, "application/json")],
        serde_json::json!(BearerTokenResponse {
            access_token,
            refresh_token
        })
        .to_string(),
    )
        .into_response())
}

#[derive(Serialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct BearerTokenResponse {
    access_token: String,
    refresh_token: String,
}

#[derive(Debug, Deserialize, Serialize)]
struct Claims {
    user_id: Uuid,
    role: Option<UserRole>,
    sub_role: Option<HashMap<AppPrivateRoute, UserRole>>,
    exp: i64,
}

#[derive(Debug, Deserialize, Serialize)]
struct RefreshClaims {
    user_id: Uuid,
    exp: i64,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct UserInfo {
    pub user_id: Uuid,
    pub role: UserRole,
    pub sub_role: HashMap<AppPrivateRoute, UserRole>,
}

#[inline]
fn generate_password_hash(password: &str) -> Result<String> {
    let salt = SaltString::generate(&mut OsRng);

    let hash = Pbkdf2
        .hash_password(password.as_bytes(), &salt)
        .map_err(|e| Error::Auth(e.into()))?
        .to_string();
    verify_password(password, &hash)?;
    Ok(hash)
}

#[inline]
fn verify_password(password: &str, password_hash: &str) -> Result<()> {
    let parsed = PasswordHash::new(password_hash).map_err(|e| Error::Auth(e.into()))?;
    Pbkdf2
        .verify_password(password.as_bytes(), &parsed)
        .map_err(|_| Error::Auth(AuthError::InvalidPassword))
}

#[inline]
pub fn parse_access_token(
    token: &str,
    uri_opt: impl Into<Option<String>>,
    is_auth_token: bool,
) -> Result<Uuid> {
    let decoded = decode::<Claims>(
        token,
        &DecodingKey::from_secret(SETTINGS.access_token_secret.expose_secret().as_bytes()),
        &Validation::new(Algorithm::HS512),
    )
    .map_err(|e| {
        if e.kind() == &JWTErrorKind::ExpiredSignature {
            if is_auth_token {
                return Error::Auth(AuthError::TokenNeedRefresh);
            }
            if let Some(uri) = uri_opt.into() {
                return Error::Auth(AuthError::JWTTokenNeedRefresh(uri));
            }
            return Error::Auth(AuthError::JWTError(e));
        }
        Error::Auth(AuthError::JWTError(e))
    })?;
    Ok(decoded.claims.user_id)
}

#[inline]
pub fn parse_refresh_token(token: &str) -> Result<Uuid> {
    let decoded = decode::<RefreshClaims>(
        token,
        &DecodingKey::from_secret(SETTINGS.refresh_token_secret.expose_secret().as_bytes()),
        &Validation::new(Algorithm::HS512),
    )
    .map_err(|e| Error::Auth(e.into()))?;
    Ok(decoded.claims.user_id)
}

#[inline]
pub fn generate_access_token(user_id: Uuid) -> Result<String> {
    let expiration = Utc::now()
        .checked_add_signed(chrono::Duration::seconds(SETTINGS.access_expiration.into()))
        .expect("valid timestamp")
        .timestamp();

    let claims = Claims {
        user_id,
        role: None,
        exp: expiration,
        sub_role: None,
    };
    let header = Header::new(Algorithm::HS512);
    encode(
        &header,
        &claims,
        &EncodingKey::from_secret(SETTINGS.access_token_secret.expose_secret().as_bytes()),
    )
    .map_err(|e| Error::Auth(AuthError::JWTError(e)))
}

#[inline]
pub fn generate_refresh_token(user_id: Uuid) -> Result<String> {
    let expiration = Utc::now()
        .checked_add_signed(chrono::Duration::seconds(
            SETTINGS.refresh_expiration.into(),
        ))
        .expect("valid timestamp")
        .timestamp();
    let claims = RefreshClaims {
        user_id,
        exp: expiration,
    };
    let header = Header::new(Algorithm::HS512);
    encode(
        &header,
        &claims,
        &EncodingKey::from_secret(SETTINGS.refresh_token_secret.expose_secret().as_bytes()),
    )
    .map_err(|e| Error::Auth(AuthError::JWTError(e)))
}

#[inline]
pub fn get_cookie_headers(
    access_token: &str,
    refresh_token: &str,
) -> AppendHeaders<[(HeaderName, String); 2]> {
    AppendHeaders([
        (
            SET_COOKIE,
            format!(
                "{ACCESS_COOKIE_NAME}={access_token}; {}",
                COOKIE_ATTRIBUTE.trim(),
            ),
        ),
        (
            SET_COOKIE,
            format!(
                "{REFRESH_COOKIE_NAME}={refresh_token}; {}",
                COOKIE_ATTRIBUTE.trim(),
            ),
        ),
    ])
}
