use crate::{
    db::{
        invenope::MongoOperationType,
        inventory::{is_operation_could_be_backward_safely, shift_inventory_quantity},
        mongo::{OPERATIONS_COL, TRANSFERS_COL},
        shipment::get_shipment_by_no,
    },
    error_result::{Error, Result},
    server::transfer::NewTransferInputItem,
};
use axum::async_trait;
use chrono::{DateTime as ChronoDT, Local, Utc};
use futures::StreamExt;
use mongodb::bson::{self, doc, DateTime, Document, Uuid};
use serde::{Deserialize, Serialize};
use tracing::{debug, info, instrument};

use super::{
    invenope::{MongoInventoryOperation, Operations},
    inventory::InventoryLocation,
    mongo::{DbClient, SHIPMENT_COL},
    shipment::{MongoShipment, ShipmentVendor},
    TransferRepo,
};
#[derive(Debug, Deserialize)]
struct FindShipmentsByTransferIdOutput {
    shipments: Vec<MongoShipment>,
}

#[async_trait]
impl TransferRepo for DbClient {
    async fn create_new_transfer(
        &self,
        shipment_no: &str,
        note: &str,
        transfer_date: ChronoDT<Utc>,
        shipment_vendor: ShipmentVendor,
        items: Vec<NewTransferInputItem>,
    ) -> Result<()> {
        let builder = MongoTransferBuilder::new(
            shipment_no,
            note,
            transfer_date.into(),
            shipment_vendor,
            &items,
        );
        builder.publish_new_transfer(self).await?;
        Ok(())
    }

    async fn find_transfer_by_id(&self, id: Uuid) -> Result<MongoTransferOutput> {
        Ok(find_transfer_by_id(id, self).await?)
    }
    async fn find_shipment_by_transfer_id(&self, id: Uuid) -> Result<Vec<MongoShipment>> {
        debug!("got transfer id:{id}");
        let pipeline = vec![
            doc! {
              "$match":{
                "id":id,
              }
            },
            doc! {
                "$lookup":{
                "from":SHIPMENT_COL,
                "localField":"shipment_id",
                "foreignField":"id",
                "as":"shipments",
              }
            },
        ];
        let mut cursor = self
            .ph_db
            .collection::<Document>(TRANSFERS_COL)
            .aggregate(pipeline, None)
            .await?;

        let mut outputs = Vec::new();
        while let Some(doc) = cursor.next().await {
            let output: FindShipmentsByTransferIdOutput = bson::from_document(doc?)?;
            outputs.push(output);
        }
        debug!("outputs :{outputs:?}");
        if outputs.is_empty() {
            return Ok(Vec::new());
        }
        Ok(outputs[0].shipments.to_owned())
    }

    async fn delete_transfer_by_id(&self, id: Uuid) -> Result<()> {
        Ok(delete_transfer_by_id(self, id).await?)
    }

    async fn find_transfer_by_shipment_id(
        &self,
        shipment_id: Uuid,
    ) -> Result<Option<Vec<MongoTransferOutput>>> {
        Ok(find_transfer_by_shipment_id(self, shipment_id).await?)
    }
    async fn query_transfers(
        &self,
        from: ChronoDT<Utc>,
        to: ChronoDT<Utc>,
        keyword: Option<String>,
    ) -> Result<Vec<MongoTransferOutput>> {
        Ok(query_transfers(self, from.into(), to.into(), keyword).await?)
    }

    async fn find_transfer_by_shipment_no(
        &self,
        shipment_no: &str,
    ) -> Result<Vec<MongoTransferOutput>> {
        let query = vec![
            doc! {
                "$match":{
                    "shipment_no":shipment_no
                }
            },
            doc! {
              "$lookup":{
                "from":OPERATIONS_COL,
                "localField":"operation_ids",
                "foreignField":"id",
                "as":"items",
              },
            },
        ];
        let mut cursor = self
            .ph_db
            .collection::<Document>(TRANSFERS_COL)
            .aggregate(query, None)
            .await?;
        let mut outputs = Vec::new();
        while let Some(doc) = cursor.next().await {
            let output: MongoTransferOutput = bson::from_document(doc?)?;
            outputs.push(output);
        }

        Ok(outputs)
    }

    async fn find_mongo_transfer_by_shipment_no(
        &self,
        shipment_no: &str,
    ) -> Result<Vec<MongoTransfer>> {
        let doc = doc! {
          "shipment_no":shipment_no,
        };
        let mut transfers = self
            .ph_db
            .collection::<MongoTransfer>(TRANSFERS_COL)
            .find(doc, None)
            .await?;
        let mut outputs = Vec::new();
        while let Some(transfer) = transfers.next().await {
            outputs.push(transfer?)
        }
        Ok(outputs)
    }

    async fn update_transfers_shipment_no(
        &self,
        current_shipment_no: &str,
        new_shipment_no: &str,
    ) -> Result<()> {
        let query = doc! {
          "shipment_no":current_shipment_no,
        };
        let update = doc! {
          "$set":{
            "shipment_no":new_shipment_no,
          }
        };
        self.ph_db
            .collection::<MongoTransfer>(TRANSFERS_COL)
            .update_many(query, update, None)
            .await?;
        Ok(())
    }

    async fn update_transfer_shipment_no_by_id(
        &self,
        transfer_id: Uuid,
        new_shipment_no: &str,
    ) -> Result<()> {
        let query = doc! {
          "id":transfer_id,
        };
        let update = doc! {
          "$set":{
            "shipment_no":new_shipment_no,
          }
        };
        self.ph_db
            .collection::<MongoTransfer>(TRANSFERS_COL)
            .update_many(query, update, None)
            .await?;
        Ok(())
    }

    async fn update_transfers_vendor_by_shipment_no(
        &self,
        shipment_no: &str,
        new_vender: ShipmentVendor,
    ) -> Result<()> {
        let query = doc! {
          "shipment_no":shipment_no,
        };
        let update = doc! {
          "$set":{
            "shipment_vendor":new_vender,
          }
        };
        self.ph_db
            .collection::<MongoTransfer>(TRANSFERS_COL)
            .update_many(query, update, None)
            .await?;
        Ok(())
    }

    async fn check_operations_backward_safety_by_transfer_id(
        &self,
        transfer_id: Uuid,
    ) -> Result<()> {
        let old_operations = find_operations_by_transfer_id(self, transfer_id).await?;
        for operation in old_operations.iter().filter(|o| o.count > 0) {
            if !super::inventory::is_operation_could_be_backward_safely(self, operation).await? {
                return Err(Error::InvalidOperation);
            }
        }
        Ok(())
    }

    async fn update_transfer_vendor_and_operations_by_transfer_id(
        &self,
        transfer_id: Uuid,
        new_vender: ShipmentVendor,
        new_location: InventoryLocation,
    ) -> Result<()> {
        let old_operations = find_operations_by_transfer_id(self, transfer_id).await?;
        let mut new_operation_ids = Vec::new();
        for operation in old_operations {
            if operation.count > 0 {
                operation
                    .run_backward(self, MongoOperationType::UpdateTransfer)
                    .await?;
                let new_operation = MongoInventoryOperation::new(
                    &operation.item_code_ext,
                    transfer_id,
                    MongoOperationType::UpdateTransfer,
                    operation.count,
                    new_location,
                );
                let id = new_operation.run_self(self, false).await?;
                new_operation_ids.push(id);
                continue;
            }
            new_operation_ids.push(operation.id);
        }
        let query = doc! {
          "id":transfer_id,
        };
        let update = doc! {
          "$set":{
            "shipment_vendor":new_vender,
            "operation_ids":new_operation_ids,
          }
        };
        self.ph_db
            .collection::<MongoTransfer>(TRANSFERS_COL)
            .update_many(query, update, None)
            .await?;
        Ok(())
    }
}

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct MongoTransfer {
    pub id: Uuid,
    pub created_at: DateTime,
    pub update_at: DateTime,
    pub shipment_no: String,
    pub shipment_id: Option<Uuid>,
    pub transfer_date: DateTime,
    pub shipment_vendor: ShipmentVendor,
    pub note: String,
    pub operation_ids: Vec<Uuid>,
}

impl MongoTransfer {
    fn new(
        id: Uuid,
        shipment_id: Option<Uuid>,
        shipment_no: &str,
        note: &str,
        transfer_date: DateTime,
        shipment_vendor: ShipmentVendor,
        operation_ids: &[Uuid],
    ) -> Self {
        Self {
            id,
            created_at: Local::now().into(),
            update_at: Local::now().into(),
            shipment_no: shipment_no.to_string(),
            shipment_id,
            transfer_date,
            shipment_vendor,
            note: note.to_owned(),
            operation_ids: operation_ids.to_owned(),
        }
    }

    async fn insert_self(&self, db: &DbClient) -> Result<()> {
        info!(
            "insert new transfer id:{} ship by shipment_id:{:?}",
            self.id, self.shipment_id
        );
        let doc = doc! {
          "id":self.id,
          "created_at":self.created_at,
          "update_at":self.update_at,
          "shipment_no":&self.shipment_no,
          "shipment_id":self.shipment_id,
          "transfer_date":self.transfer_date,
          "note":&self.note,
          "shipment_vendor":&self.shipment_vendor,
          "operation_ids":&self.operation_ids
        };
        db.ph_db
            .collection(TRANSFERS_COL)
            .insert_one(doc, None)
            .await?;
        info!("insert new transfer success");
        Ok(())
    }
}

pub struct MongoTransferBuilder {
    pub transfer_id: Uuid,
    pub shipment_no: String,
    pub transfer_date: DateTime,
    pub note: String,
    pub shipment_vendor: ShipmentVendor,
    pub items: Vec<NewTransferInputItem>,
}

impl MongoTransferBuilder {
    pub fn new(
        shipment_no: &str,
        note: &str,
        transfer_date: DateTime,
        shipment_vendor: ShipmentVendor,
        items: &[NewTransferInputItem],
    ) -> Self {
        Self {
            transfer_id: Uuid::new(),
            shipment_no: shipment_no.trim().to_owned(),
            transfer_date,
            shipment_vendor,
            note: note.trim().to_owned(),
            items: items.to_owned(),
        }
    }

    #[instrument(name="publish new transfer",skip(self,db),fields(
        transfer_id=%self.transfer_id,
        shipment_no=%self.shipment_no,
    ))]
    pub async fn publish_new_transfer(&self, db: &DbClient) -> Result<MongoTransfer> {
        info!("try publish new transfer id:{}", self.transfer_id);
        let mut operation_ids = Vec::new();
        for item in self.items.iter() {
            info!("try shift {}'s inventory", item.item_code_ext);
            let mut ids =
                shift_inventory_quantity(db, &item.item_code_ext, &item.quantity, self.transfer_id)
                    .await?;
            operation_ids.append(&mut ids);
        }
        info!("check if shipment no:{} existing.", &self.shipment_no);
        let shipments = get_shipment_by_no(db, &self.shipment_no).await?;
        if !shipments.is_empty() {
            info!(
                "shipment no:{} exists, so use shipment's infos",
                &self.shipment_no
            );
            let transfer = MongoTransfer::new(
                self.transfer_id,
                Some(shipments[0].id),
                &self.shipment_no,
                &self.note,
                shipments[0].shipment_date,
                shipments[0].vendor,
                &operation_ids,
            );
            info!("publish new transfer id:{} success", self.transfer_id);
            transfer.insert_self(db).await?;
            return Ok(transfer);
        }
        info!(
            "shipment no:{} not exists, use input infos",
            &self.shipment_no
        );
        let transfer = MongoTransfer::new(
            self.transfer_id,
            None,
            &self.shipment_no,
            &self.note,
            self.transfer_date,
            self.shipment_vendor,
            &operation_ids,
        );
        transfer.insert_self(db).await?;
        info!("publish new transfer id:{} success", self.transfer_id);

        Ok(transfer)
    }
}

#[derive(Deserialize, Debug, Clone)]
pub struct MongoTransferOutput {
    pub id: Uuid,
    pub created_at: DateTime,
    pub update_at: DateTime,
    pub shipment_no: String,
    pub shipment_id: Option<Uuid>,
    pub transfer_date: DateTime,
    pub shipment_vendor: ShipmentVendor,
    pub note: String,
    pub items: Vec<MongoTransferItem>,
}

#[derive(Deserialize, Debug, Clone)]
pub struct MongoTransferItem {
    pub item_code_ext: String,
    pub count: i32,
    pub location: InventoryLocation,
}

pub async fn find_transfer_by_id(id: Uuid, db: &DbClient) -> Result<MongoTransferOutput> {
    let pipeline = vec![
        doc! {
            "$match":{
                "id":id
            }
        },
        doc! {
            "$lookup":{
          "from": OPERATIONS_COL,
          "localField": "operation_ids",
          "foreignField": "id",
          "as": "items",
            }
        },
        doc! {
            "$sort":{
                "transfer_date":-1
            }
        },
    ];

    let mut cursor = db
        .ph_db
        .collection::<Document>(TRANSFERS_COL)
        .aggregate(pipeline, None)
        .await?;
    let mut outputs = Vec::new();
    while let Some(doc) = cursor.next().await {
        let output: MongoTransferOutput = bson::from_document(doc?)?;
        outputs.push(output);
    }
    if outputs.is_empty() {
        return Err(Error::TransferNotFound(id.to_string()));
    }
    Ok(outputs[0].to_owned())
}

pub async fn query_transfers(
    db: &DbClient,
    from: DateTime,
    to: DateTime,
    keyword: Option<String>,
) -> Result<Vec<MongoTransferOutput>> {
    let mut pipeline = vec![
        doc! {
          "$match":{
            "transfer_date":{
              "$gte":from,
              "$lte":to,
            }
          }
        },
        doc! {
          "$sort":{
            "transfer_date":-1
          }
        },
        doc! {
          "$lookup":{
              "from": OPERATIONS_COL,
              "localField": "operation_ids",
              "foreignField": "id",
              "as": "items",
          },
        },
    ];
    if let Some(keyword) = keyword.as_deref() {
        pipeline.push(doc! {
          "$match":{
            "$or":[
              {"items.item_code_ext":{
                          "$regex":keyword,
                          "$options":"i"
              }},
              {"shipment_no":{
                          "$regex":keyword,
                          "$options":"i"
              }},
              {"note":{
                          "$regex":keyword,
                          "$options":"i"
              }},
            ]
          }
        })
    }

    let mut cursor = db
        .ph_db
        .collection::<Document>(TRANSFERS_COL)
        .aggregate(pipeline, None)
        .await?;
    let mut outputs = Vec::new();
    while let Some(doc) = cursor.next().await {
        let output: MongoTransferOutput = bson::from_document(doc?)?;
        outputs.push(output);
    }
    Ok(outputs)
}

pub async fn delete_transfer_by_id(db: &DbClient, id: Uuid) -> Result<()> {
    info!("try to delete transfer id:{}", id);
    let operations = find_operations_by_transfer_id(db, id).await?;
    for operation in operations.iter().filter(|o| o.count > 0) {
        if !is_operation_could_be_backward_safely(db, operation).await? {
            return Err(Error::InvalidOperation);
        }
    }
    for operation in operations {
        operation
            .run_backward(db, MongoOperationType::DeleteTransfer)
            .await?;
    }
    let query = doc! {
      "id":id
    };
    db.ph_db
        .collection::<MongoTransfer>(TRANSFERS_COL)
        .delete_one(query, None)
        .await?;
    Ok(())
}

pub async fn find_operations_by_transfer_id(
    db: &DbClient,
    transfer_id: Uuid,
) -> Result<Vec<MongoInventoryOperation>> {
    let query = vec![
        doc! {
          "$match":{
            "id":transfer_id,
          },
        },
        doc! {
          "$lookup":{
            "from":OPERATIONS_COL,
            "localField":"operation_ids",
            "foreignField":"id",
            "as":OPERATIONS_COL,
          },
        },
    ];
    let mut cursor = db
        .ph_db
        .collection::<Document>(TRANSFERS_COL)
        .aggregate(query, None)
        .await?;
    let mut transfers = Vec::new();
    while let Some(doc) = cursor.next().await {
        let output: Operations = bson::from_document(doc?)?;
        transfers.push(output);
    }

    Ok(transfers[0].to_owned().operations)
}

pub async fn find_transfer_by_shipment_id(
    db: &DbClient,
    shipment_id: Uuid,
) -> Result<Option<Vec<MongoTransferOutput>>> {
    let query = vec![
        doc! {
            "$match":{
                "shipment_id":shipment_id
            }
        },
        doc! {
          "$lookup":{
            "from":OPERATIONS_COL,
            "localField":"operation_ids",
            "foreignField":"id",
            "as":"items",
          },
        },
    ];
    let mut cursor = db
        .ph_db
        .collection::<Document>(TRANSFERS_COL)
        .aggregate(query, None)
        .await?;
    let mut outputs = Vec::new();
    while let Some(doc) = cursor.next().await {
        let output: MongoTransferOutput = bson::from_document(doc?)?;
        outputs.push(output);
    }

    if outputs.is_empty() {
        return Ok(None);
    }

    Ok(Some(outputs))
}
