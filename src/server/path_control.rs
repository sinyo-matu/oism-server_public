use std::collections::HashMap;

use crate::db::auth::UserRole;
use axum::http::Method;
use axum::Router;

use super::AppState;

pub trait ApplicationPath: Send + Sync + 'static {
    fn root_path(&self) -> String;
    fn inject_auth_router(self, router: Router<AppState>) -> Router<AppState>;
    fn get_matcher(&self) -> &matchit::Router<HashMap<Method, UserRole>>;
}
