use std::sync::Arc;

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{get, patch, put},
    Json, Router,
};
use chrono::prelude::*;
use chrono::serde::ts_seconds;
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast::Sender;
use tracing::instrument;
use uuid::Uuid;

use crate::{
    cache::OrderCache,
    db::{mongo::DbClient, order::ITEMS_PER_PAGE, Order, OrderItem, OrderRepo, RegisterItem},
    services::google_service::GoogleService,
};
use crate::{db::order::OrderItemStatus, error_result::Result};

use super::{
    auth::{UserInfo, SETTINGS},
    ws::{send_control_message, send_control_messages, ControlMessage},
    AppState, OrderRegisterInput, PagedResponse,
};

pub fn get_router() -> Router<AppState> {
    Router::new()
        .route("/", get(query_orders).post(create_new_order))
        .route("/:id", get(get_order_by_id).delete(delete_order))
        .route("/taobao_no/:taobao_no", get(get_order_by_taobao_no))
        .route("/:id/note", patch(update_order_note))
        .route("/check_then_update", put(check_then_update_order_status))
}

pub fn get_items_router() -> Router<AppState> {
    Router::new()
        .route("/", get(query_order_items))
        .route("/:id", get(get_order_item_by_id).delete(conceal_order_item))
        .route("/:id/rate", patch(update_order_items_rate))
}

#[instrument(name="create new order",skip(user_info,message,db,cache,sender),fields(
    request_id = %Uuid::new_v4(),
    action_by = %user_info.user_id,
))]
pub async fn create_new_order(
    user_info: UserInfo,
    State(db): State<Arc<DbClient>>,
    State(cache): State<Arc<dyn OrderCache>>,
    State(sender): State<Arc<Sender<ControlMessage>>>,
    Json(message): Json<OrderRegisterInput>,
) -> Result<impl IntoResponse> {
    db.create_order(message).await?;
    let messages = &[
        ControlMessage::RefreshOrderList,
        ControlMessage::RefreshInventory,
        ControlMessage::RefreshInventoryItemQuantity,
        ControlMessage::RefreshWaitForShipmentItemList,
    ];
    send_control_messages(sender, messages);
    cache.clear_orders();
    Ok(StatusCode::CREATED)
}

#[derive(Deserialize, Serialize, Debug, Clone, PartialEq, Eq, Hash)]
#[serde(rename_all = "camelCase")]
pub struct QueryOrdersMessage {
    pub keyword: String,
    pub status: String,
    #[serde(with = "ts_seconds")]
    pub from: DateTime<Utc>,
    #[serde(with = "ts_seconds")]
    pub to: DateTime<Utc>,
    pub page: Option<u32>,
}

pub async fn query_orders(
    Query(message): Query<QueryOrdersMessage>,
    State(db): State<Arc<DbClient>>,
    State(cache): State<Arc<dyn OrderCache>>,
) -> Result<Json<PagedResponse<Order>>> {
    let current_page = message.page.unwrap_or(0);
    if let Some(output) = cache.get_orders(&message) {
        let res = PagedResponse {
            has_next: (output.len() as u32) == ITEMS_PER_PAGE,
            data: output.into_iter().map(|m| m.into()).collect(),
            next: current_page + 1,
        };
        return Ok(res.into());
    }
    let (has_next, output) = db
        .query_orders(
            &message.keyword,
            &message.status,
            message.from,
            message.to,
            message.page,
        )
        .await?;
    if !cache.contains_orders(&message) {
        cache.set_orders(message, output.clone());
    }
    let res = PagedResponse {
        data: output.into_iter().map(|m| m.into()).collect(),
        next: current_page + 1,
        has_next,
    };
    Ok(res.into())
}

pub async fn get_order_by_id(
    Path(id): Path<Uuid>,
    State(db): State<Arc<DbClient>>,
) -> Result<Json<Order>> {
    let output = db.get_order_by_id(id.into()).await?;
    let reply: Order = output.into();
    Ok(reply.into())
}

pub async fn get_order_by_taobao_no(
    Path(taobao_order_no): Path<String>,
    State(db): State<Arc<DbClient>>,
) -> Result<Json<Vec<Order>>> {
    let output = db.get_order_by_taobao_no(&taobao_order_no).await?;
    Ok(Json(output.into_iter().map(|o| o.into()).collect()))
}

#[instrument(name="delete order request",skip(user_info,db,cache,sender),fields(
    request_id=%Uuid::new_v4(),
    action_by=%user_info.user_id,
))]
pub async fn delete_order(
    user_info: UserInfo,
    Path(order_id): Path<Uuid>,
    State(db): State<Arc<DbClient>>,
    State(cache): State<Arc<dyn OrderCache>>,
    State(sender): State<Arc<Sender<ControlMessage>>>,
    State(google_service): State<Arc<GoogleService>>,
) -> Result<impl IntoResponse> {
    let output = db.delete_order(order_id.into()).await?;
    let messages = &[
        ControlMessage::RefreshOrderList,
        ControlMessage::RefreshInventory,
        ControlMessage::RefreshInventoryItemQuantity,
        ControlMessage::RefreshWaitForShipmentItemList,
    ];
    send_control_messages(sender.clone(), messages);
    for item in output.deleted_items {
        if output.item_is_shipped_ids.contains(&item.id) {
            google_service
                .call_notify(
                    SETTINGS.google_service.target_user_ex_id,
                    SETTINGS.google_service.task_list_name.clone(),
                    item.item_code_ext,
                    format!("顧客名:{},メモ:{}", item.customer_id, item.note),
                )
                .await;
        }
        send_control_message(
            &sender,
            ControlMessage::RefreshNewShipmentBucket(item.id.into()),
        );
    }
    cache.clear_orders();
    Ok(StatusCode::OK)
}

pub async fn get_order_item_by_id(
    Path(order_item_id): Path<Uuid>,
    State(db): State<Arc<DbClient>>,
) -> Result<Json<OrderItem>> {
    let res: OrderItem = db.get_order_item_by_id(order_item_id.into()).await?.into();
    Ok(res.into())
}

#[instrument(name="conceal order item request",skip(user_info,db,cache,sender),fields(
    request_id=%Uuid::new_v4(),
    action_by=%user_info.user_id,
))]
pub async fn conceal_order_item(
    user_info: UserInfo,
    Path(order_item_id): Path<Uuid>,
    State(db): State<Arc<DbClient>>,
    State(cache): State<Arc<dyn OrderCache>>,
    State(sender): State<Arc<Sender<ControlMessage>>>,
    State(google_service): State<Arc<GoogleService>>,
) -> Result<impl IntoResponse> {
    let output = db.conceal_order_item(order_item_id.into()).await?;
    if output.is_shipped {
        google_service
            .call_notify(
                SETTINGS.google_service.target_user_ex_id,
                SETTINGS.google_service.task_list_name.clone(),
                output.concealed_item.item_code_ext,
                format!(
                    "顧客名:{},メモ:{}",
                    output.concealed_item.customer_id, output.concealed_item.note
                ),
            )
            .await;
    }
    send_control_message(&sender, ControlMessage::RefreshOrderItem(order_item_id));
    send_control_message(&sender, ControlMessage::RefreshInventory);
    send_control_message(&sender, ControlMessage::RefreshInventoryItemQuantity);
    send_control_message(
        &sender,
        ControlMessage::RefreshNewShipmentBucket(order_item_id),
    );
    send_control_message(&sender, ControlMessage::RefreshWaitForShipmentItemList);
    cache.clear_orders();
    Ok(StatusCode::OK)
}

#[derive(Deserialize, Serialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct UpdateOrderNoteMessage {
    pub note: String,
}

#[instrument(name="update note",skip(user_info,db,cache,message),fields(
    request_id=%Uuid::new_v4(),
    action_by=%user_info.user_id,
))]
pub async fn update_order_note(
    user_info: UserInfo,
    State(db): State<Arc<DbClient>>,
    State(cache): State<Arc<dyn OrderCache>>,
    Path(order_id): Path<Uuid>,
    Json(message): Json<UpdateOrderNoteMessage>,
) -> Result<impl IntoResponse> {
    db.update_order_note(order_id.into(), &message.note).await?;
    cache.clear_orders();
    Ok(StatusCode::OK)
}

#[derive(Deserialize, Serialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct CheckThenUpdateOrderStatusMessage {
    items: Vec<RegisterItem>,
}

#[instrument(name="check and update order status",skip(message,db,cache,sender),fields(
    request_id=%Uuid::new_v4()
))]
pub async fn check_then_update_order_status(
    State(db): State<Arc<DbClient>>,
    State(cache): State<Arc<dyn OrderCache>>,
    State(sender): State<Arc<Sender<ControlMessage>>>,
    Json(message): Json<CheckThenUpdateOrderStatusMessage>,
) -> Result<Json<Vec<String>>> {
    let res = db.check_then_update_order_status(message.items).await?;
    if !res.is_empty() {
        for item in res.iter() {
            send_control_message(&sender, ControlMessage::RefreshOrderItem(item.id.into()));
        }
        //update client's order item id for re-render validating state to a random id
        //for preventing unnecessary re-renders
        send_control_message(&sender, ControlMessage::RefreshOrderItem(Uuid::new_v4()));
        send_control_message(&sender, ControlMessage::RefreshInventory);
        send_control_message(&sender, ControlMessage::RefreshWaitForShipmentItemList);
        cache.clear_orders();
    }
    Ok(res
        .into_iter()
        .map(|oi| oi.customer_id)
        .collect::<Vec<_>>()
        .into())
}

#[derive(Deserialize, Serialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct QueryOrderItemsMessage {
    keyword: String,
    status: OrderItemStatus,
}

pub async fn query_order_items(
    Query(message): Query<QueryOrderItemsMessage>,
    State(db): State<Arc<DbClient>>,
) -> Result<Json<Vec<OrderItem>>> {
    let outputs = db
        .query_order_items(&message.keyword, &message.status)
        .await?;
    Ok(outputs
        .into_iter()
        .map(|o| o.into())
        .collect::<Vec<_>>()
        .into())
}

#[derive(Deserialize, Serialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct UpdateOrderItemRateMessage {
    rate: f64,
}

#[instrument(name="update order item rate",skip(user_info,db,cache,sender),fields(
    request_id=%Uuid::new_v4(),
    action_by=%user_info.user_id
))]
pub async fn update_order_items_rate(
    user_info: UserInfo,
    State(db): State<Arc<DbClient>>,
    State(cache): State<Arc<dyn OrderCache>>,
    State(sender): State<Arc<Sender<ControlMessage>>>,
    Path(order_item_id): Path<Uuid>,
    Json(message): Json<UpdateOrderItemRateMessage>,
) -> Result<impl IntoResponse> {
    db.update_order_item_rate(order_item_id.into(), message.rate)
        .await?;
    send_control_message(&sender, ControlMessage::RefreshOrderItem(order_item_id));
    send_control_message(&sender, ControlMessage::RefreshOrderList);
    send_control_message(
        &sender,
        ControlMessage::RefreshNewShipmentBucket(order_item_id),
    );
    send_control_message(&sender, ControlMessage::RefreshWaitForShipmentItemList);
    cache.clear_orders();
    Ok(StatusCode::OK)
}
