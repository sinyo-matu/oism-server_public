use std::{collections::HashMap, fmt::Display};

use mongodb::bson::{doc, Bson, Document, Uuid};
use serde::{Deserialize, Serialize};

use crate::{
    db::mongo::USERS_COL,
    error_result::{AuthError, Error},
};
use crate::{error_result::Result, server::AppPrivateRoute};

use super::mongo::DbClient;

pub async fn check_is_username_occupied(db: &DbClient, username: &str) -> Result<bool> {
    let filter = doc! {"username":username};
    let res = db
        .ph_db
        .collection::<User>(USERS_COL)
        .find_one(filter, None)
        .await?;
    if res.is_some() {
        return Ok(true);
    }
    Ok(false)
}

pub async fn create_user(db: &DbClient, user: User) -> Result<()> {
    let mut sub_role_doc = Document::new();
    for (path, role) in user.sub_role.iter() {
        sub_role_doc.insert(path.to_string(), role);
    }
    let doc = doc! {
        "id":user.id,
        "username":user.username,
        "hash":user.hash,
        "role":user.role,
        "sub_role":sub_role_doc,
    };
    db.ph_db.collection(USERS_COL).insert_one(doc, None).await?;
    Ok(())
}

pub async fn find_user(db: &DbClient, id: Uuid) -> Result<User> {
    let filter = doc! {"id":id};
    let res = db
        .ph_db
        .collection::<User>(USERS_COL)
        .find_one(filter, None)
        .await?;
    if res.is_none() {
        return Err(Error::Auth(AuthError::UserNotFound));
    }
    Ok(res.unwrap())
}

pub async fn find_user_by_username(db: &DbClient, username: &str) -> Result<User> {
    let filter = doc! {"username":username};
    let res = db
        .ph_db
        .collection::<User>(USERS_COL)
        .find_one(filter, None)
        .await?;
    if res.is_none() {
        return Err(Error::Auth(AuthError::UserNotFound));
    }
    Ok(res.unwrap())
}
#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct User {
    pub id: Uuid,
    pub username: String,
    pub hash: String,
    pub role: UserRole,
    pub sub_role: HashMap<AppPrivateRoute, UserRole>,
}

impl User {
    pub fn new(
        username: String,
        hash: String,
        role: UserRole,
        sub_role: HashMap<AppPrivateRoute, UserRole>,
    ) -> Self {
        Self {
            id: Uuid::new(),
            username,
            hash,
            role,
            sub_role,
        }
    }
}

#[derive(Deserialize, Serialize, Debug, Clone, PartialEq, PartialOrd, Eq, Copy)]
#[serde(rename_all = "snake_case")]
pub enum UserRole {
    Full = 0,
    Editor = 1,
    Viewer = 2,
    Visitor = 3,
}

impl Display for UserRole {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Full => write!(f, "full"),
            Self::Editor => write!(f, "editor"),
            Self::Viewer => write!(f, "viewer"),
            Self::Visitor => write!(f, "visitor"),
        }
    }
}

impl From<UserRole> for Bson {
    fn from(u: UserRole) -> Self {
        match u {
            UserRole::Full => Bson::String(String::from("full")),
            UserRole::Viewer => Bson::String(String::from("viewer")),
            UserRole::Editor => Bson::String(String::from("editor")),
            UserRole::Visitor => Bson::String(String::from("visitor")),
        }
    }
}
