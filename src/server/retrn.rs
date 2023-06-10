use std::sync::Arc;

use crate::db::{inventory::Quantity, mongo::DbClient, Return, ReturnRepo};
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{delete, post},
    Json, Router,
};
use chrono::prelude::*;
use chrono::serde::ts_seconds;
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast::Sender;
use uuid::Uuid;

use crate::error_result::Result;

use super::{
    ws::{send_control_message, ControlMessage},
    AppState,
};

pub fn get_return_router() -> Router<AppState> {
    Router::new()
        .route("/", post(create_new_return).get(query_returns))
        .route("/:id", delete(delete_return_by_id).get(get_return_by_id))
}

#[derive(Deserialize, Serialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct NewReturnInputItem {
    pub item_code_ext: String,
    pub quantity: Vec<Quantity>,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct NewReturnInput {
    pub return_no: String,
    #[serde(with = "ts_seconds")]
    pub return_date: DateTime<Utc>,
    pub note: String,
    pub items: Vec<NewReturnInputItem>,
}

pub async fn create_new_return(
    State(db): State<Arc<DbClient>>,
    State(sender): State<Arc<Sender<ControlMessage>>>,
    Json(input): Json<NewReturnInput>,
) -> Result<impl IntoResponse> {
    db.create_new_return(
        &input.return_no,
        input.return_date,
        &input.note,
        input.items,
    )
    .await?;
    send_control_message(&sender, ControlMessage::RefreshReturnList);
    send_control_message(&sender, ControlMessage::RefreshInventory);
    send_control_message(&sender, ControlMessage::RefreshInventoryItemQuantity);
    Ok(StatusCode::CREATED)
}

#[derive(Deserialize, Serialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct QueryReturnMessage {
    #[serde(with = "ts_seconds")]
    from: DateTime<Utc>,
    #[serde(with = "ts_seconds")]
    to: DateTime<Utc>,
    keyword: Option<String>,
}

pub async fn query_returns(
    Query(message): Query<QueryReturnMessage>,
    State(db): State<Arc<DbClient>>,
) -> Result<Json<Vec<Return>>> {
    let res = db
        .query_returns(message.from, message.to, message.keyword)
        .await?;
    Ok(res
        .into_iter()
        .map(|item| item.into())
        .collect::<Vec<_>>()
        .into())
}

pub async fn get_return_by_id(
    Path(id): Path<Uuid>,
    State(db): State<Arc<DbClient>>,
) -> Result<Json<Return>> {
    let output: Return = db.get_return_by_id(id.into()).await?.into();
    Ok(output.into())
}

pub async fn delete_return_by_id(
    Path(id): Path<Uuid>,
    State(db): State<Arc<DbClient>>,
    State(sender): State<Arc<Sender<ControlMessage>>>,
) -> Result<impl IntoResponse> {
    db.delete_return_by_id(id.into()).await?;
    send_control_message(&sender, ControlMessage::RefreshReturnList);
    send_control_message(&sender, ControlMessage::RefreshInventory);
    send_control_message(&sender, ControlMessage::RefreshInventoryItemQuantity);
    Ok(StatusCode::OK)
}
