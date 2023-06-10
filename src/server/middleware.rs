use axum::{
    async_trait,
    extract::{FromRequestParts, State, TypedHeader},
    headers::{authorization::Bearer, Authorization, Cookie},
    http::{request::Parts, Request},
    middleware::Next,
    response::{IntoResponse, Response},
    Extension,
};
use tracing::error;

use crate::{
    db::SMTAuthDataBase,
    error_result::{AuthError, Error, Result},
};
use std::{result::Result as StdResult, sync::Arc};

use super::{
    auth::{
        parse_access_token, parse_refresh_token, RefreshAuthInfo, UserInfo, ACCESS_COOKIE_NAME,
        REFRESH_COOKIE_NAME,
    },
    path_control::ApplicationPath,
    AppState,
};

#[async_trait]
impl<S> FromRequestParts<S> for UserInfo
where
    S: Send + Sync,
{
    type Rejection = Response;
    async fn from_request_parts(req: &mut Parts, state: &S) -> StdResult<Self, Self::Rejection> {
        let Extension(state) = Extension::<AppState>::from_request_parts(req, &state)
            .await
            .unwrap();
        let user_info = match TypedHeader::<Cookie>::from_request_parts(req, &state).await {
            Ok(TypedHeader(cookie)) => {
                let token = cookie
                    .get(ACCESS_COOKIE_NAME)
                    .ok_or(Error::Auth(AuthError::JWTTokenNotFound))
                    .map_err(|e| e.into_response())?;
                let id = parse_access_token(token, req.uri.to_string(), false)
                    .map_err(|e| e.into_response())?;
                let user = state
                    .db_client
                    .find_user(id.into())
                    .await
                    .map_err(|e| e.into_response())?;
                UserInfo {
                    user_id: user.id.into(),
                    role: user.role,
                    sub_role: user.sub_role,
                }
            }
            Err(_) => {
                if let Ok(TypedHeader(authorization)) =
                    TypedHeader::<Authorization<Bearer>>::from_request_parts(req, &state).await
                {
                    let id = parse_access_token(authorization.token(), None, true)
                        .map_err(|e| e.into_response())?;
                    let user = state
                        .db_client
                        .find_user(id.into())
                        .await
                        .map_err(|e| e.into_response())?;
                    UserInfo {
                        user_id: user.id.into(),
                        role: user.role,
                        sub_role: user.sub_role,
                    }
                } else {
                    error!("not found cookie and auth header either!");
                    return Err(Error::Auth(AuthError::CookieHeaderNotFound).into_response());
                }
            }
        };
        Ok(user_info)
    }
}

#[async_trait]
impl<S> FromRequestParts<S> for RefreshAuthInfo
where
    S: Send + Sync,
{
    type Rejection = Response;

    async fn from_request_parts(req: &mut Parts, state: &S) -> StdResult<Self, Self::Rejection> {
        match TypedHeader::<Cookie>::from_request_parts(req, state).await {
            Ok(TypedHeader(cookie)) => {
                let token = cookie
                    .get(REFRESH_COOKIE_NAME)
                    .ok_or(Error::Auth(AuthError::JWTTokenNotFound))
                    .map_err(|e| e.into_response())?;
                let user_id = parse_refresh_token(token).map_err(|e| e.into_response())?;
                Ok(Self(user_id))
            }
            Err(_) => {
                if let Ok(TypedHeader(authorization)) =
                    TypedHeader::<Authorization<Bearer>>::from_request_parts(req, state).await
                {
                    let user_id = parse_refresh_token(authorization.token())
                        .map_err(|e| e.into_response())?;
                    return Ok(Self(user_id));
                }
                error!("not found cookie and auth header either!");
                Err(Error::Auth(AuthError::CookieHeaderNotFound).into_response())
            }
        }
    }
}

// pub fn check_permission<B>(
//     user_info: &UserInfo,
//     route: AppPrivateRoute,
//     role: UserRole,
// ) -> Result<Response> {
//     if user_info.role <= role {
//         return Err(Error::Changed.into());
//     }
//     let sub_role = user_info.sub_role.get(&route);
//     if sub_role.is_none() {
//         return Err(Error::Auth(AuthError::PermissionNotEnough {
//             got: None,
//             need: role,
//         }));
//     }
//     if sub_role.unwrap() > &role {
//         return Err(Error::Auth(AuthError::PermissionNotEnough {
//             got: Some(*sub_role.unwrap()),
//             need: role,
//         }));
//     }
//     Ok()
// }

pub async fn auth<B>(
    State(state): State<Arc<dyn ApplicationPath>>,
    user_info: UserInfo,
    req: Request<B>,
    next: Next<B>,
) -> Result<Response> {
    let matcher = state.get_matcher();
    let matched = matcher.at(req.uri().path()).ok();
    if matched.is_none() {
        return Err(Error::PathNotFound);
    }
    let role = matched.unwrap().value.get(req.method());
    if role.is_none() {
        return Err(Error::PathNotFound);
    }
    let role = role.unwrap();
    if user_info.role <= *role {
        return Ok(next.run(req).await);
    }
    let sub_role = user_info.sub_role.get(&state.root_path().into());
    if sub_role.is_none() {
        return Err(Error::Auth(AuthError::PermissionNotEnough {
            got: None,
            need: *role,
        }));
    }
    if sub_role.unwrap() > role {
        return Err(Error::Auth(AuthError::PermissionNotEnough {
            got: Some(*sub_role.unwrap()),
            need: *role,
        }));
    }
    Ok(next.run(req).await)
}
