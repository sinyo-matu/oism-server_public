use std::sync::Arc;

use crate::{
    db::{
        inventory::{InventoryLocation, Quantity},
        mongo::DbClient,
        shipment::ShipmentVendor,
        transfer::{MongoTransferItem, MongoTransferOutput},
    },
    error_result::Error,
};
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{delete, get, post, put},
    Json, Router,
};
use chrono::prelude::*;
use chrono::serde::ts_seconds;
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast::Sender;
use tracing::info;
use uuid::Uuid;

use crate::db::TransferRepo;
use crate::error_result::Result;

use super::{
    shipment::ShipmentLite,
    ws::{send_control_message, ControlMessage},
    AppState,
};

pub fn get_transfer_router() -> Router<AppState> {
    Router::new()
        .route("/", post(create_new_transfer).get(query_transfers))
        .route(
            "/:id",
            delete(delete_transfer_by_id).get(find_transfer_by_id),
        )
        .route("/:id/shipments", get(find_shipments_by_id))
        .route("/:id/shipment_no", put(update_transfer_shipment_no))
        .route(
            "/shipment_no/:shipment_no",
            get(find_transfer_by_shipment_no),
        )
        .route(
            "/by_shipment_id/:shipment_id",
            get(get_transfers_by_shipment_id),
        )
}

#[derive(Deserialize, Serialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct NewTransferInputItem {
    pub item_code_ext: String,
    pub quantity: Vec<Quantity>,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct NewTransferMessage {
    pub shipment_no: String,
    pub note: String,
    #[serde(with = "ts_seconds")]
    pub transfer_date: DateTime<Utc>,
    pub shipment_vendor: ShipmentVendor,
    pub to_location: InventoryLocation,
    pub items: Vec<NewTransferInputItem>,
}

pub async fn create_new_transfer(
    State(db): State<Arc<DbClient>>,
    State(sender): State<Arc<Sender<ControlMessage>>>,
    Json(message): Json<NewTransferMessage>,
) -> Result<impl IntoResponse> {
    if message.shipment_vendor.is_clearance_vendor() && !message.to_location.is_paid() {
        return Err(Error::VenderLocationNotMatch);
    }
    db.create_new_transfer(
        &message.shipment_no,
        &message.note,
        message.transfer_date,
        message.shipment_vendor,
        message.items,
    )
    .await?;
    send_control_message(&sender, ControlMessage::RefreshTransferList);
    send_control_message(&sender, ControlMessage::RefreshInventory);
    send_control_message(&sender, ControlMessage::RefreshInventoryItemQuantity);
    Ok(StatusCode::CREATED)
}

#[derive(Deserialize, Serialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Transfer {
    pub id: Uuid,
    #[serde(with = "ts_seconds")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "ts_seconds")]
    pub update_at: DateTime<Utc>,
    pub shipment_no: String,
    pub shipment_id: Option<Uuid>,
    #[serde(with = "ts_seconds")]
    pub transfer_date: DateTime<Utc>,
    pub shipment_vendor: ShipmentVendor,
    pub note: String,
    pub items: Vec<TransferItem>,
}

impl From<MongoTransferOutput> for Transfer {
    fn from(m: MongoTransferOutput) -> Self {
        Self {
            id: m.id.into(),
            created_at: m.created_at.to_chrono(),
            update_at: m.update_at.to_chrono(),
            shipment_no: m.shipment_no,
            shipment_id: m.shipment_id.map(|i| i.into()),
            transfer_date: m.transfer_date.to_chrono(),
            shipment_vendor: m.shipment_vendor,
            note: m.note,
            items: m
                .items
                .into_iter()
                .filter(|item| item.count > 0)
                .map(|i| i.into())
                .collect(),
        }
    }
}

#[derive(Deserialize, Serialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct TransferItem {
    pub item_code_ext: String,
    pub count: i32,
    pub location: InventoryLocation,
}

impl From<MongoTransferItem> for TransferItem {
    fn from(m: MongoTransferItem) -> Self {
        Self {
            item_code_ext: m.item_code_ext,
            count: m.count,
            location: m.location,
        }
    }
}
pub async fn find_transfer_by_id(
    Path(id): Path<Uuid>,
    State(db): State<Arc<DbClient>>,
) -> Result<Json<Transfer>> {
    let transfer: Transfer = db.find_transfer_by_id(id.into()).await?.into();
    Ok(transfer.into())
}

#[derive(Deserialize, Serialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct QueryTransferMessage {
    #[serde(with = "ts_seconds")]
    from: DateTime<Utc>,
    #[serde(with = "ts_seconds")]
    to: DateTime<Utc>,
    keyword: Option<String>,
}

pub async fn delete_transfer_by_id(
    Path(id): Path<Uuid>,
    State(db): State<Arc<DbClient>>,
    State(sender): State<Arc<Sender<ControlMessage>>>,
) -> Result<impl IntoResponse> {
    db.delete_transfer_by_id(id.into()).await?;
    send_control_message(&sender, ControlMessage::RefreshTransferList);
    send_control_message(&sender, ControlMessage::RefreshInventory);
    send_control_message(&sender, ControlMessage::RefreshInventoryItemQuantity);
    Ok(StatusCode::OK)
}
pub async fn query_transfers(
    Query(message): Query<QueryTransferMessage>,
    State(db): State<Arc<DbClient>>,
) -> Result<Json<Vec<Transfer>>> {
    let res = db
        .query_transfers(message.from, message.to, message.keyword)
        .await?;
    Ok(res
        .into_iter()
        .map(|item| item.into())
        .collect::<Vec<_>>()
        .into())
}

pub async fn find_shipments_by_id(
    Path(id): Path<Uuid>,
    State(db): State<Arc<DbClient>>,
) -> Result<Json<Vec<ShipmentLite>>> {
    let outputs = db.find_shipment_by_transfer_id(id.into()).await?;
    Ok(Json(outputs.into_iter().map(|s| s.into()).collect()))
}

pub async fn get_transfers_by_shipment_id(
    Path(shipment_id): Path<Uuid>,
    State(db): State<Arc<DbClient>>,
) -> Result<Json<Vec<Transfer>>> {
    let transfers: Option<Vec<Transfer>> = db
        .find_transfer_by_shipment_id(shipment_id.into())
        .await?
        .map(|transfers| {
            transfers
                .into_iter()
                .map(|transfer| transfer.into())
                .collect()
        });
    if transfers.is_none() {
        return Ok(Json(Vec::new()));
    };
    Ok(Json(transfers.unwrap()))
}

pub async fn find_transfer_by_shipment_no(
    Path(shipment_no): Path<String>,
    State(db): State<Arc<DbClient>>,
) -> Result<Json<Vec<Transfer>>> {
    let transfers: Vec<Transfer> = db
        .find_transfer_by_shipment_no(&shipment_no)
        .await?
        .into_iter()
        .map(|transfer| transfer.into())
        .collect();
    Ok(Json(transfers))
}

#[derive(Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct UpdateShipmentNoMessage {
    shipment_no: String,
    update_related_transfers: bool,
}
pub async fn update_transfer_shipment_no(
    State(db): State<Arc<DbClient>>,
    State(sender): State<Arc<Sender<ControlMessage>>>,
    Path(transfer_id): Path<Uuid>,
    Json(message): Json<UpdateShipmentNoMessage>,
) -> Result<impl IntoResponse> {
    info!("got request of update transfer's shipment no");
    let transfer = db.find_transfer_by_id(transfer_id.into()).await?;
    match message.update_related_transfers {
        true => {
            db.update_transfers_shipment_no(&transfer.shipment_no, &message.shipment_no)
                .await?
        }
        false => {
            db.update_transfer_shipment_no_by_id(transfer_id.into(), &message.shipment_no)
                .await?
        }
    }
    info!("done request!");
    send_control_message(&sender, ControlMessage::RefreshTransferList);
    Ok(StatusCode::OK)
}
