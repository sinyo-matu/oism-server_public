use crate::{
    db::mongo::SHIPMENT_COL,
    error_result::{Error, Result},
    server::NewShipmentInput,
};
use axum::async_trait;
use chrono::prelude::*;
use chrono::DateTime as ChronoDT;
use futures::StreamExt;
use mongodb::{
    bson::{self, doc, Bson, DateTime, Document, Uuid},
    error::UNKNOWN_TRANSACTION_COMMIT_RESULT,
    options::{Acknowledgment, ReadConcern, TransactionOptions, WriteConcern},
    ClientSession,
};
use serde::{Deserialize, Serialize};
use tracing::{info, instrument};

use super::{
    mongo::{DbClient, ORDER_ITEMS_COL},
    order::{
        update_order_item_status_to_shipped_by_id_with_session, MongoOrderItem, OrderItemStatus,
        ITEMS_PER_PAGE,
    },
    ShipmentRepo,
};

#[async_trait]
impl ShipmentRepo for DbClient {
    /// create a new shipment.
    async fn create_new_shipment(&self, input: NewShipmentInput) -> Result<()> {
        MongoShipment::publish_new_shipment(
            self,
            &input.shipment_no,
            &input.note,
            &input.vendor,
            input.shipment_date.into(),
            &input
                .item_ids
                .iter()
                .map(|id| (*id).into())
                .collect::<Vec<_>>(),
        )
        .await?;
        Ok(())
    }

    async fn query_shipments(
        &self,
        keyword: &str,
        from: ChronoDT<Utc>,
        to: ChronoDT<Utc>,
        status: &str,
        vendor: &str,
        page: Option<u32>,
    ) -> Result<(bool, Vec<MongoShipmentOutput>)> {
        Ok(query_shipments(self, keyword, from.into(), to.into(), status, vendor, page).await?)
    }

    async fn get_shipment_by_id(&self, id: Uuid) -> Result<MongoShipmentOutput> {
        Ok(get_shipment_by_id(self, id).await?)
    }

    async fn find_shipment_by_no(&self, shipment_no: &str) -> Result<Vec<MongoShipment>> {
        Ok(get_shipment_by_no(self, shipment_no).await?)
    }

    async fn find_shipments_by_no(&self, shipment_no: &str) -> Result<Vec<MongoShipmentOutput>> {
        let pipeline = vec![
            doc! {
              "$match":{
                "shipment_no":shipment_no
              }
            },
            doc! {
              "$lookup":{
                  "from": ORDER_ITEMS_COL,
                  "localField": "order_item_ids",
                  "foreignField": "id",
                  "as": "items",
              },
            },
        ];
        let mut cursor = self
            .ph_db
            .collection::<Document>(SHIPMENT_COL)
            .aggregate(pipeline, None)
            .await?;
        let mut outputs = Vec::new();
        while let Some(doc) = cursor.next().await {
            let output: MongoShipmentOutput = bson::from_document(doc?)?;
            outputs.push(output);
        }
        Ok(outputs)
    }

    async fn delete_shipment(&self, shipment_id: Uuid) -> Result<Vec<Uuid>> {
        Ok(delete_shipment(self, shipment_id).await?)
    }

    async fn update_shipment_note(&self, shipment_id: Uuid, note: &str) -> Result<()> {
        Ok(update_shipment_note(self, shipment_id, note).await?)
    }

    async fn update_shipment_status(&self, shipment_id: Uuid, status: &str) -> Result<()> {
        Ok(update_shipment_status(self, shipment_id, status).await?)
    }

    async fn update_shipment_no(
        &self,
        current_shipment_no: &str,
        new_shipment_no: &str,
    ) -> Result<()> {
        info!(
            "will update shipment_no: {} to {}",
            current_shipment_no, new_shipment_no
        );
        let query = doc! {
          "shipment_no":current_shipment_no,
        };
        let update = doc! {
          "$set":{
            "shipment_no":new_shipment_no,
          }
        };
        info!("update shipment's shipment_no");
        self.ph_db
            .collection::<MongoShipment>(SHIPMENT_COL)
            .update_many(query, update, None)
            .await?;
        Ok(())
    }

    async fn update_shipment_no_by_id(
        &self,
        shipment_id: Uuid,
        new_shipment_no: &str,
    ) -> Result<()> {
        let query = doc! {
          "id":shipment_id,
        };
        let update = doc! {
          "$set":{
            "shipment_no":new_shipment_no,
          }
        };
        info!("update shipment's shipment_no");
        self.ph_db
            .collection::<MongoShipment>(SHIPMENT_COL)
            .update_one(query, update, None)
            .await?;
        Ok(())
    }
    async fn update_shipment_vendor(
        &self,
        shipment_id: Uuid,
        new_vendor: ShipmentVendor,
    ) -> Result<()> {
        let query = doc! {
          "id":shipment_id,
        };
        let update = doc! {
          "$set":{
            "vendor":new_vendor,
          }
        };
        info!("update shipment's vendor to {new_vendor:?}");
        self.ph_db
            .collection::<MongoShipment>(SHIPMENT_COL)
            .update_one(query, update, None)
            .await?;
        Ok(())
    }
}

#[derive(Deserialize, Serialize, Debug, Clone)]
struct ShipmentId {
    id: Uuid,
}

/// Shipment object used in mongo db
#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct MongoShipment {
    pub id: Uuid,
    pub created_at: DateTime,
    pub update_at: DateTime,
    pub shipment_no: String,
    pub note: String,
    pub vendor: ShipmentVendor,
    pub shipment_date: DateTime,
    pub order_item_ids: Vec<Uuid>,
    pub status: ShipmentStatus,
}

#[derive(Deserialize, Serialize, Debug, Clone, PartialEq, Eq, Copy)]
#[serde(rename_all = "lowercase")]
pub enum ShipmentVendor {
    YY,
    SS,
    SD,
    BC,
    Ems,
    ML,
    SJ,
    PML,
}

impl ShipmentVendor {
    pub fn is_clearance_vendor(&self) -> bool {
        matches!(self, ShipmentVendor::SJ)
    }
}

impl ShipmentVendor {
    pub fn stringify_vendor(&self) -> String {
        match self {
            ShipmentVendor::YY => String::from("友谊"),
            ShipmentVendor::SS => String::from("七海"),
            ShipmentVendor::SD => String::from("顺达"),
            ShipmentVendor::BC => String::from("黒猫"),
            ShipmentVendor::Ems => String::from("EMS"),
            ShipmentVendor::ML => String::from("国内"),
            ShipmentVendor::SJ => String::from("流通王"),
            ShipmentVendor::PML => String::from("国内済"),
        }
    }
}

#[derive(Deserialize, Serialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ShipmentStatus {
    Ongoing,
    Arrival,
}

impl From<ShipmentStatus> for Bson {
    fn from(s: ShipmentStatus) -> Self {
        match s {
            ShipmentStatus::Ongoing => Bson::String(String::from("ongoing")),
            ShipmentStatus::Arrival => Bson::String(String::from("arrival")),
        }
    }
}

impl From<ShipmentVendor> for Bson {
    fn from(s: ShipmentVendor) -> Self {
        match s {
            ShipmentVendor::YY => Bson::String(String::from("yy")),
            ShipmentVendor::SS => Bson::String(String::from("ss")),
            ShipmentVendor::SD => Bson::String(String::from("sd")),
            ShipmentVendor::BC => Bson::String(String::from("bc")),
            ShipmentVendor::Ems => Bson::String(String::from("ems")),
            ShipmentVendor::ML => Bson::String(String::from("ml")),
            ShipmentVendor::SJ => Bson::String(String::from("sj")),
            ShipmentVendor::PML => Bson::String(String::from("pml")),
        }
    }
}

impl ToString for ShipmentVendor {
    fn to_string(&self) -> String {
        match self {
            ShipmentVendor::YY => String::from("yy"),
            ShipmentVendor::SS => String::from("ss"),
            ShipmentVendor::SD => String::from("sd"),
            ShipmentVendor::BC => String::from("bc"),
            ShipmentVendor::Ems => String::from("ems"),
            ShipmentVendor::ML => String::from("ml"),
            ShipmentVendor::SJ => String::from("sj"),
            ShipmentVendor::PML => String::from("pml"),
        }
    }
}

/// Shipment object for passing to front end
#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct MongoShipmentOutput {
    pub id: Uuid,
    pub created_at: DateTime,
    pub update_at: DateTime,
    pub shipment_no: String,
    pub note: String,
    pub vendor: ShipmentVendor,
    pub shipment_date: DateTime,
    pub items: Vec<MongoOrderItem>,
    pub status: ShipmentStatus,
}

impl MongoShipment {
    /// publish a new shipment id then create a new shipment
    fn new(
        shipment_no: &str,
        note: &str,
        vendor: &ShipmentVendor,
        shipment_date: DateTime,
        order_item_ids: &[Uuid],
    ) -> Self {
        Self {
            id: Uuid::new(),
            created_at: Local::now().into(),
            update_at: Local::now().into(),
            shipment_no: shipment_no.trim().to_owned(),
            note: note.trim().to_owned(),
            vendor: vendor.to_owned(),
            shipment_date,
            order_item_ids: order_item_ids.to_owned(),
            status: ShipmentStatus::Ongoing,
        }
    }
    /// the main function to publish a new shipment, will create a new , update its related order
    /// item's status then insert to db.
    pub async fn publish_new_shipment(
        db: &DbClient,
        shipment_no: &str,
        note: &str,
        vendor: &ShipmentVendor,
        shipment_date: DateTime,
        order_item_ids: &[Uuid],
    ) -> Result<Uuid> {
        let mut session = db.client.start_session(None).await?;

        let options = TransactionOptions::builder()
            .read_concern(ReadConcern::majority())
            .write_concern(WriteConcern::builder().w(Acknowledgment::Majority).build())
            .build();
        session.start_transaction(options).await?;
        let shipment = MongoShipment::new(shipment_no, note, vendor, shipment_date, order_item_ids);
        for order_item_id in order_item_ids {
            while let Err(error) = update_order_item_status_to_shipped_by_id_with_session(
                db,
                *order_item_id,
                shipment.id,
                &mut session,
            )
            .await
            {
                match error {
                    Error::Mongodb(e) => {
                        if e.contains_label(UNKNOWN_TRANSACTION_COMMIT_RESULT) {
                            continue;
                        }
                        return Err(Error::Mongodb(e));
                    }
                    _ => {
                        return Err(error);
                    }
                }
            }
        }
        while let Err(error) = shipment.insert_self_with_session(db, &mut session).await {
            match error {
                Error::Mongodb(e) => {
                    if e.contains_label(UNKNOWN_TRANSACTION_COMMIT_RESULT) {
                        continue;
                    }
                    return Err(Error::Mongodb(e));
                }
                _ => {
                    return Err(error);
                }
            }
        }

        loop {
            if let Err(ref error) = session.commit_transaction().await {
                if error.contains_label(UNKNOWN_TRANSACTION_COMMIT_RESULT) {
                    continue;
                }
            }
            break;
        }
        Ok(shipment.id)
    }

    /// insert a new shipment to db.
    async fn _insert_self(&self, db: &DbClient) -> Result<Uuid> {
        info!(
            "insert new shipment id:{} shipment no:{} vendor:{:?}",
            self.id, &self.shipment_no, &self.vendor
        );
        let doc = doc! {
            "id":self.id,
            "shipment_no":&self.shipment_no,
            "created_at":self.created_at,
            "update_at":self.update_at,
            "note":&self.note,
            "vendor":&self.vendor,
            "shipment_date":self.shipment_date,
            "order_item_ids":&self.order_item_ids,
            "status":&self.status,
        };

        db.ph_db
            .collection(SHIPMENT_COL)
            .insert_one(doc, None)
            .await?;
        info!("insert shipment success");
        Ok(self.id)
    }

    async fn insert_self_with_session(
        &self,
        db: &DbClient,
        session: &mut ClientSession,
    ) -> Result<Uuid> {
        info!(
            "insert new shipment id:{} shipment no:{} vendor:{:?}",
            self.id, &self.shipment_no, &self.vendor
        );
        let doc = doc! {
            "id":self.id,
            "shipment_no":&self.shipment_no,
            "created_at":self.created_at,
            "update_at":self.update_at,
            "note":&self.note,
            "vendor":&self.vendor,
            "shipment_date":self.shipment_date,
            "order_item_ids":&self.order_item_ids,
            "status":&self.status,
        };

        db.ph_db
            .collection(SHIPMENT_COL)
            .insert_one_with_session(doc, None, session)
            .await?;
        info!("insert shipment success");
        Ok(self.id)
    }
}

pub async fn query_shipments(
    db: &DbClient,
    keyword: &str,
    from: DateTime,
    to: DateTime,
    status: &str,
    vendor: &str,
    page: Option<u32>,
) -> Result<(bool, Vec<MongoShipmentOutput>)> {
    let mut pipeline = vec![
        doc! {
          "$match":{
            "shipment_date":{
              "$gte":from,
              "$lte":to,
            }
          }
        },
        doc! {
          "$lookup":{
              "from": ORDER_ITEMS_COL,
              "localField": "order_item_ids",
              "foreignField": "id",
              "as": "items",
          },
        },
    ];

    if !status.is_empty() {
        pipeline.push(doc! {
          "$match":{
            "status":status,
          }
        });
    }

    if !vendor.is_empty() {
        pipeline.push(doc! {
          "$match":{
            "vendor":vendor
          }
        })
    }

    if !keyword.is_empty() {
        pipeline.push(doc! {
          "$match":{
            "$or":[
              {"vendor":{
                          "$regex":keyword,
                          "$options":"i"
              }},
              {"items.customer_id":{
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
              {"items.item_code_ext":{
                          "$regex":keyword,
                          "$options":"i"
              }}
            ]
          }
        })
    }

    pipeline.push(doc! {
      "$sort":{
        "update_at":-1
      }
    });
    // page is none means this is a non-paged request.
    // we return full result.
    if page.is_none() {
        let mut cursor = db
            .ph_db
            .collection::<Document>(SHIPMENT_COL)
            .aggregate(pipeline, None)
            .await?;
        let mut outputs = Vec::new();
        while let Some(doc) = cursor.next().await {
            let output: MongoShipmentOutput = bson::from_document(doc?)?;
            outputs.push(output);
        }
        return Ok((false, outputs));
    }

    // reach here means this is a paged request
    let page = page.unwrap();
    let skip = ITEMS_PER_PAGE * page;

    pipeline.push(doc! {
        "$limit":ITEMS_PER_PAGE +skip
    });

    pipeline.push(doc! {
        "$skip":skip
    });
    let mut cursor = db
        .ph_db
        .collection::<Document>(SHIPMENT_COL)
        .aggregate(pipeline, None)
        .await?;
    let mut outputs = Vec::new();
    while let Some(doc) = cursor.next().await {
        let output: MongoShipmentOutput = bson::from_document(doc?)?;
        outputs.push(output);
    }
    Ok(((outputs.len() as u32) == ITEMS_PER_PAGE, outputs))
}

pub async fn get_shipment_by_no(db: &DbClient, no: &str) -> Result<Vec<MongoShipment>> {
    let doc = doc! {
      "shipment_no":no,
    };
    let mut shipments = db
        .ph_db
        .collection::<MongoShipment>(SHIPMENT_COL)
        .find(doc, None)
        .await?;
    let mut outputs = Vec::new();
    while let Some(shipment) = shipments.next().await {
        outputs.push(shipment?)
    }
    Ok(outputs)
}

pub async fn get_shipment_by_id(db: &DbClient, id: Uuid) -> Result<MongoShipmentOutput> {
    let pipeline = vec![
        doc! {
          "$match":{
            "id":id
          }
        },
        doc! {
          "$lookup":{
              "from": ORDER_ITEMS_COL,
              "localField": "order_item_ids",
              "foreignField": "id",
              "as": "items",
          },
        },
    ];

    let mut cursor = db
        .ph_db
        .collection::<Document>(SHIPMENT_COL)
        .aggregate(pipeline, None)
        .await?;
    let mut outputs = Vec::new();
    while let Some(doc) = cursor.next().await {
        let output: MongoShipmentOutput = bson::from_document(doc?)?;
        outputs.push(output);
    }
    Ok(outputs[0].to_owned())
}
#[derive(Deserialize, Serialize, Debug, Clone)]
struct OrderItems {
    items: Vec<MongoOrderItem>,
}

pub async fn delete_shipment(db: &DbClient, shipment_id: Uuid) -> Result<Vec<Uuid>> {
    let pipeline = vec![
        doc! {
          "$match":{
            "id":shipment_id
          }
        },
        doc! {
          "$lookup":{
              "from": ORDER_ITEMS_COL,
              "localField": "order_item_ids",
              "foreignField": "id",
              "as": "items",
          }
        },
    ];
    let mut cursor = db
        .ph_db
        .collection::<Document>(SHIPMENT_COL)
        .aggregate(pipeline, None)
        .await?;
    let mut outputs = Vec::new();
    while let Some(doc) = cursor.next().await {
        let output: OrderItems = bson::from_document(doc?)?;
        outputs.push(output);
    }

    for mut item in outputs[0]
        .items
        .clone()
        .into_iter()
        .filter(|item| item.status == OrderItemStatus::Shipped)
    {
        item.restore_self_status_to_guaranteed(db).await?
    }

    let query = doc! {
      "id":shipment_id,
    };

    db.ph_db
        .collection::<MongoShipment>(SHIPMENT_COL)
        .delete_one(query, None)
        .await?;

    Ok(outputs[0].items.iter().map(|item| item.id).collect())
}

#[instrument(name = "update shipment note inner", skip(db))]
pub async fn update_shipment_note(db: &DbClient, shipment_id: Uuid, note: &str) -> Result<()> {
    info!("update shipment :{shipment_id}'s note to {note}");
    let query = doc! {
      "id":shipment_id,
    };
    let update = doc! {
      "$set":{
        "note":note,
      }
    };
    db.ph_db
        .collection::<MongoShipment>(SHIPMENT_COL)
        .update_one(query, update, None)
        .await?;
    info!("update note success");
    Ok(())
}

pub async fn update_shipment_status(db: &DbClient, shipment_id: Uuid, status: &str) -> Result<()> {
    let query = doc! {
      "id":shipment_id,
    };
    let update = doc! {
      "$set":{
        "status":status,
      }
    };
    db.ph_db
        .collection::<MongoShipment>(SHIPMENT_COL)
        .update_one(query, update, None)
        .await?;

    Ok(())
}
