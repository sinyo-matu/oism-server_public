use std::sync::Arc;

use crate::{
    cache::OrderCache,
    db::{inventory::InventoryLocation, mongo::DbClient, shipment::ShipmentStatus, TransferRepo},
    error_result::Result,
};
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{delete, get, patch, post, put},
    Json, Router,
};
use chrono::prelude::*;
use chrono::serde::ts_seconds;
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast::Sender;
use tracing::{info, instrument};
use uuid::Uuid;

use crate::db::{
    shipment::{MongoShipment, ShipmentVendor},
    Shipment, ShipmentRepo,
};

use super::{
    export::{export_shipment_by_id_except_color_no, export_shipment_ordered, export_shipments},
    ws::{send_control_message, ControlMessage},
    AppState, NewShipmentInput, PagedResponse,
};

pub fn get_shipment_router() -> Router<AppState> {
    Router::new()
        .route("/", post(create_new_shipment).get(query_shipments))
        .route("/:id", delete(delete_shipment).get(get_shipment_by_id))
        .route("/:id/note", patch(update_shipment_note))
        .route("/:id/status", put(update_shipment_status))
        .route("/:id/no", put(update_shipment_no))
        .route("/:id/vendor", put(update_shipment_vendor))
        .route("/:id/export", get(export_shipment_by_id_except_color_no))
        .route("/:id/export_ordered", get(export_shipment_ordered))
        .route("/by_no/:no", get(find_shipment_by_no))
        .route("/export", get(export_shipments))
}

pub async fn create_new_shipment(
    State(db): State<Arc<DbClient>>,
    State(cache): State<Arc<dyn OrderCache>>,
    State(sender): State<Arc<Sender<ControlMessage>>>,
    Json(input): Json<NewShipmentInput>,
) -> Result<impl IntoResponse> {
    db.create_new_shipment(input.clone()).await?;
    send_control_message(&sender, ControlMessage::RefreshShipmentList);
    send_control_message(&sender, ControlMessage::RefreshWaitForShipmentItemList);
    for id in input.item_ids {
        send_control_message(&sender, ControlMessage::RefreshOrderItem(id));
    }
    send_control_message(&sender, ControlMessage::RefreshOrderItem(Uuid::new_v4()));
    cache.clear_orders();
    Ok(StatusCode::CREATED)
}

#[derive(Deserialize, Serialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct QueryShipmentMessage {
    pub keyword: String,
    #[serde(with = "ts_seconds")]
    pub from: DateTime<Utc>,
    #[serde(with = "ts_seconds")]
    pub to: DateTime<Utc>,
    pub status: String,
    pub vendor: String,
    pub page: Option<u32>,
}

pub async fn query_shipments(
    Query(message): Query<QueryShipmentMessage>,
    State(db): State<Arc<DbClient>>,
) -> Result<Json<PagedResponse<Shipment>>> {
    let (has_next, outputs) = db
        .query_shipments(
            &message.keyword,
            message.from,
            message.to,
            &message.status,
            &message.vendor,
            message.page,
        )
        .await?;
    let current_page = message.page.unwrap_or(0);
    let res = PagedResponse {
        data: outputs
            .into_iter()
            .map(|mut shipment| {
                shipment
                    .items
                    .sort_by(|a, b| a.customer_id.cmp(&b.customer_id));
                shipment.into()
            })
            .collect::<Vec<_>>(),
        has_next,
        next: current_page + 1,
    };
    Ok(res.into())
}

#[derive(Deserialize, Serialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct GetShipmentByIdMessage {
    pub id: Uuid,
}

pub async fn get_shipment_by_id(
    Path(id): Path<Uuid>,
    State(db): State<Arc<DbClient>>,
) -> Result<Json<Shipment>> {
    let mut output = db.get_shipment_by_id(id.into()).await?;
    output
        .items
        .sort_by(|a, b| a.customer_id.cmp(&b.customer_id));
    Ok(Json(output.into()))
}

#[instrument(name = "delete shipment", skip(id, db, cache, sender))]
pub async fn delete_shipment(
    Path(id): Path<Uuid>,
    State(db): State<Arc<DbClient>>,
    State(cache): State<Arc<dyn OrderCache>>,
    State(sender): State<Arc<Sender<ControlMessage>>>,
) -> Result<impl IntoResponse> {
    info!(
        "got delete shipment request will delete shipment_id: {}",
        id
    );
    let item_ids = db.delete_shipment(id.into()).await?;
    send_control_message(&sender, ControlMessage::RefreshShipmentList);
    send_control_message(&sender, ControlMessage::RefreshWaitForShipmentItemList);
    for id in item_ids {
        send_control_message(&sender, ControlMessage::RefreshOrderItem(id.into()));
    }
    send_control_message(&sender, ControlMessage::RefreshOrderItem(Uuid::new_v4()));
    cache.clear_orders();
    Ok(StatusCode::OK)
}

#[derive(Deserialize, Serialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct UpdateShipmentNoteMessage {
    note: String,
}

#[instrument(name="update shipment note",skip(shipment_id,message,db),fields(
    request_id = %Uuid::new_v4(),
))]
pub async fn update_shipment_note(
    State(db): State<Arc<DbClient>>,
    Path(shipment_id): Path<Uuid>,
    Json(message): Json<UpdateShipmentNoteMessage>,
) -> Result<impl IntoResponse> {
    db.update_shipment_note(shipment_id.into(), &message.note)
        .await?;
    Ok(StatusCode::OK)
}

pub async fn find_shipment_by_no(
    Path(shipment_no): Path<String>,
    State(db): State<Arc<DbClient>>,
) -> Result<Response> {
    let res: Vec<ShipmentLite> = db
        .find_shipment_by_no(&shipment_no)
        .await?
        .into_iter()
        .map(|s| s.into())
        .collect();
    if res.is_empty() {
        return Ok(StatusCode::NOT_FOUND.into_response());
    }
    Ok(Json(res).into_response())
}

#[derive(Deserialize, Serialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct UpdateShipmentStatusMessage {
    status: String,
}

pub async fn update_shipment_status(
    State(db): State<Arc<DbClient>>,
    State(sender): State<Arc<Sender<ControlMessage>>>,
    Path(shipment_id): Path<Uuid>,
    Json(message): Json<UpdateShipmentStatusMessage>,
) -> Result<impl IntoResponse> {
    db.update_shipment_status(shipment_id.into(), &message.status)
        .await?;
    send_control_message(&sender, ControlMessage::RefreshShipmentItem(shipment_id));
    send_control_message(&sender, ControlMessage::RefreshTransferList);
    Ok(StatusCode::OK)
}

#[derive(Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct UpdateShipmentNoMessage {
    shipment_no: String,
    update_related_transfers: bool,
}

#[instrument(name="update shipment no",skip(shipment_id,message,db),fields(
    request_id = %Uuid::new_v4(),
))]
pub async fn update_shipment_no(
    State(db): State<Arc<DbClient>>,
    State(sender): State<Arc<Sender<ControlMessage>>>,
    Path(shipment_id): Path<Uuid>,
    Json(message): Json<UpdateShipmentNoMessage>,
) -> Result<impl IntoResponse> {
    info!("got request of update shipment no");
    let shipment = db.get_shipment_by_id(shipment_id.into()).await?;
    if message.update_related_transfers {
        let transfers = db
            .find_mongo_transfer_by_shipment_no(&shipment.shipment_no)
            .await?;
        if !transfers.is_empty() {
            info!("update shipment_no related transfers");
            db.update_transfers_shipment_no(&shipment.shipment_no, &message.shipment_no)
                .await?;
        }
    }
    db.update_shipment_no_by_id(shipment_id.into(), &message.shipment_no)
        .await?;
    info!("done request!");
    send_control_message(&sender, ControlMessage::RefreshShipmentItem(shipment_id));
    if message.update_related_transfers {
        send_control_message(&sender, ControlMessage::RefreshTransferList);
    }
    Ok(StatusCode::OK)
}
#[derive(Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct UpdateShipmentVendorMessage {
    new_vendor: ShipmentVendor,
    update_related_transfers: bool,
}
pub async fn update_shipment_vendor(
    State(db): State<Arc<DbClient>>,
    State(sender): State<Arc<Sender<ControlMessage>>>,
    Path(shipment_id): Path<Uuid>,
    Json(message): Json<UpdateShipmentVendorMessage>,
) -> Result<impl IntoResponse> {
    info!("got request of update shipment vendor");
    if message.update_related_transfers {
        let shipment = db.get_shipment_by_id(shipment_id.into()).await?;
        let transfers = db
            .find_mongo_transfer_by_shipment_no(&shipment.shipment_no)
            .await?;
        if !transfers.is_empty() {
            let need_update_operations = message.new_vendor.is_clearance_vendor()
                != transfers[0].shipment_vendor.is_clearance_vendor();
            match need_update_operations {
                true => {
                    for transfer in transfers.iter() {
                        db.check_operations_backward_safety_by_transfer_id(transfer.id)
                            .await?;
                    }
                    for transfer in transfers {
                        let new_location = if message.new_vendor.is_clearance_vendor() {
                            InventoryLocation::PCN
                        } else {
                            InventoryLocation::CN
                        };
                        db.update_transfer_vendor_and_operations_by_transfer_id(
                            transfer.id,
                            message.new_vendor,
                            new_location,
                        )
                        .await?;
                    }
                }
                false => {
                    db.update_transfers_vendor_by_shipment_no(
                        &shipment.shipment_no,
                        message.new_vendor,
                    )
                    .await?;
                }
            }
        }
    }
    db.update_shipment_vendor(shipment_id.into(), message.new_vendor)
        .await?;
    info!("done request!");
    send_control_message(&sender, ControlMessage::RefreshShipmentItem(shipment_id));
    Ok(StatusCode::OK)
}

#[derive(Deserialize, Serialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ShipmentLite {
    id: Uuid,
    created_at: DateTime<Local>,
    update_at: DateTime<Local>,
    shipment_no: String,
    note: String,
    vendor: ShipmentVendor,
    shipment_date: DateTime<Local>,
    order_item_ids: Vec<Uuid>,
    status: ShipmentStatus,
}

impl From<MongoShipment> for ShipmentLite {
    fn from(m: MongoShipment) -> Self {
        Self {
            id: m.id.into(),
            created_at: m.created_at.to_chrono().with_timezone(&Local),
            update_at: m.update_at.to_chrono().with_timezone(&Local),
            shipment_no: m.shipment_no,
            note: m.note,
            vendor: m.vendor,
            shipment_date: m.shipment_date.to_chrono().with_timezone(&Local),
            order_item_ids: m.order_item_ids.into_iter().map(|i| i.into()).collect(),
            status: m.status,
        }
    }
}
