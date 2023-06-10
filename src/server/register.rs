use std::sync::Arc;

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
use tracing::instrument;
use uuid::Uuid;

use crate::db::{mongo::DbClient, Register, RegisterRepo, StockRegisterInput};
use crate::error_result::Result;

use super::{
    ws::{send_control_message, ControlMessage},
    AppState, PagedResponse,
};

pub fn get_router() -> Router<AppState> {
    Router::new()
        .route("/", post(create_new_register).get(query_registers))
        .route(
            "/:id",
            delete(delete_stock_register).get(get_register_by_id),
        )
}

pub async fn create_new_register(
    State(db): State<Arc<DbClient>>,
    State(sender): State<Arc<Sender<ControlMessage>>>,
    Json(message): Json<StockRegisterInput>,
) -> Result<impl IntoResponse> {
    db.insert_stock_register(&message).await?;
    send_control_message(&sender, ControlMessage::RefreshRegisterList);
    send_control_message(&sender, ControlMessage::RefreshInventory);
    send_control_message(&sender, ControlMessage::RefreshInventoryItemQuantity);
    Ok(StatusCode::CREATED)
}

#[derive(Deserialize, Serialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct QueryRegistersMessage {
    #[serde(with = "ts_seconds")]
    from: DateTime<Utc>,
    #[serde(with = "ts_seconds")]
    to: DateTime<Utc>,
    keyword: Option<String>,
    page: Option<u32>,
}

pub async fn query_registers(
    Query(message): Query<QueryRegistersMessage>,
    State(db): State<Arc<DbClient>>,
) -> Result<Json<PagedResponse<Register>>> {
    let (has_next, res) = db
        .query_registers(message.from, message.to, message.keyword, message.page)
        .await?;
    let current_page = message.page.unwrap_or(0);
    let res = PagedResponse {
        data: res.into_iter().map(|i| i.into()).collect::<Vec<_>>(),
        has_next,
        next: current_page + 1,
    };
    Ok(res.into())
}

pub async fn get_register_by_id(
    Path(id): Path<Uuid>,
    State(db): State<Arc<DbClient>>,
) -> Result<Json<Register>> {
    let output: Register = db.get_register_by_id(id.into()).await?.into();
    Ok(output.into())
}

#[instrument(name="delete register",skip(db,sender),fields(
    request_id=%Uuid::new_v4()
))]
pub async fn delete_stock_register(
    Path(id): Path<Uuid>,
    State(db): State<Arc<DbClient>>,
    State(sender): State<Arc<Sender<ControlMessage>>>,
) -> Result<impl IntoResponse> {
    db.delete_stock_register(id.into()).await?;
    send_control_message(&sender, ControlMessage::RefreshRegisterList);
    send_control_message(&sender, ControlMessage::RefreshInventory);
    send_control_message(&sender, ControlMessage::RefreshInventoryItemQuantity);
    Ok(StatusCode::OK)
}
