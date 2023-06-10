use crate::error_result::{Error, Result};
use chrono::prelude::*;
use mongodb::bson::Uuid;
use mongodb::{bson::doc, options::UpdateOptions};
use mongodb::{bson::Bson, ClientSession};
use serde::{Deserialize, Serialize};
use tracing::{info, instrument};

use super::{
    inventory::{InventoryLocation, MongoInventoryItem},
    mongo::{DbClient, INVENTORY_COL, OPERATIONS_COL},
};

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct MongoInventoryOperation {
    pub id: Uuid,
    pub item_code_ext: String,
    pub time: mongodb::bson::DateTime,
    pub related_id: Uuid,
    pub operation_type: MongoOperationType,
    pub count: i32,
    pub location: InventoryLocation,
    pub countered: bool,
}

impl MongoInventoryOperation {
    pub fn new(
        code: &str,
        related_id: Uuid,
        operation_type: MongoOperationType,
        count: i32,
        location: InventoryLocation,
    ) -> Self {
        Self {
            id: Uuid::new(),
            item_code_ext: code.into(),
            time: Local::now().into(),
            related_id,
            operation_type,
            count,
            location,
            countered: false,
        }
    }

    fn new_countered(
        code: &str,
        related_id: Uuid,
        operation_type: MongoOperationType,
        count: i32,
        location: InventoryLocation,
    ) -> Self {
        Self {
            id: Uuid::new(),
            item_code_ext: code.into(),
            time: Local::now().into(),
            related_id,
            operation_type,
            count,
            location,
            countered: true,
        }
    }

    pub async fn insert_self(&self, db: &DbClient) -> Result<()> {
        info!(
            "try insert new operation id:{},code:{} type:{:?} location:{:?} count:{}",
            self.id, &self.item_code_ext, &self.operation_type, &self.location, self.count
        );
        let doc = doc! {
          "id":self.id,
          "item_code_ext":&self.item_code_ext,
          "time": self.time,
          "related_id":self.related_id,
          "operation_type": &self.operation_type,
          "count": self.count,
          "location":&self.location,
          "countered":self.countered,
        };
        db.ph_db
            .collection(OPERATIONS_COL)
            .insert_one(doc, None)
            .await?;
        info!("insert operation success");
        Ok(())
    }

    pub async fn insert_self_with_session(
        &self,
        db: &DbClient,
        session: &mut ClientSession,
    ) -> Result<()> {
        info!(
            "try insert new operation id:{},code:{} type:{:?} location:{:?} count:{}",
            self.id, &self.item_code_ext, &self.operation_type, &self.location, self.count
        );
        let doc = doc! {
          "id":self.id,
          "item_code_ext":&self.item_code_ext,
          "time": self.time,
          "related_id":self.related_id,
          "operation_type": &self.operation_type,
          "count": self.count,
          "location":&self.location,
          "countered":self.countered,
        };
        db.ph_db
            .collection(OPERATIONS_COL)
            .insert_one_with_session(doc, None, session)
            .await?;
        info!("insert operation success");
        Ok(())
    }

    async fn set_self_countered(&self, db: &DbClient) -> Result<()> {
        info!("set operation counted id:{}", self.id);
        let query = doc! {
          "id":self.id,
        };

        let update = doc! {
          "$set":{
            "countered":true,
          }
        };
        let res = db
            .ph_db
            .collection::<MongoInventoryOperation>(OPERATIONS_COL)
            .update_one(query, update, None)
            .await?;
        if res.matched_count == 0 {
            return Err(Error::CanNotFindOperation(self.id.to_string()));
        }
        info!("set operation counted success");
        Ok(())
    }

    async fn update_self_count(&self, db: &DbClient, inc: i32) -> Result<()> {
        info!("update operation id:{} inc:{}", self.id, inc);
        let query = doc! {
          "id":self.id,
        };
        let update = doc! {
         "$inc":{
          "count":inc,
         }
        };
        let res = db
            .ph_db
            .collection::<MongoInventoryOperation>(OPERATIONS_COL)
            .update_one(query, update, None)
            .await?;
        if res.matched_count == 0 {
            return Err(Error::CanNotFindOperation(self.id.to_string()));
        }
        info!("operation update success");
        Ok(())
    }

    #[instrument(name="operation run full backward",skip(db,self,operation_type),fields(
       operation_id=%self.id,
       target_item=%self.item_code_ext,
       reason_type=?self.operation_type,
       count=%self.count,
    ))]
    pub async fn run_backward(
        &self,
        db: &DbClient,
        operation_type: MongoOperationType,
    ) -> Result<Option<Uuid>> {
        if self.count == 0 {
            info!("operation count is 0 pass run backward");
            return Ok(None);
        }
        if self.countered {
            info!("operation is countered pass run backward");
            return Ok(None);
        }
        info!(
            "generate new backward {:?} operation: inventory item:{} location:{:?} count: {}",
            &operation_type, &self.item_code_ext, &self.location, &self.count
        );
        self.set_self_countered(db).await?;
        let backward = Self::new_countered(
            &self.item_code_ext,
            self.related_id,
            operation_type,
            -self.count,
            self.location.to_owned(),
        );
        let id = backward.run_self(db, false).await?;
        Ok(Some(id))
    }

    #[instrument(name="operation run partial backward",skip(db,self,operation_type,backward_count),fields(
       operation_id=%self.id,
       target_item=%self.item_code_ext,
       reason_type=?self.operation_type,
       full_count=%self.count,
       backward_count=%backward_count
    ))]
    pub async fn run_partial_backward(
        &self,
        db: &DbClient,
        backward_count: u32,
        operation_type: MongoOperationType,
    ) -> Result<Option<Uuid>> {
        if self.count == 0 {
            info!("operation count is 0 pass run backward");
            return Ok(None);
        }
        if self.countered {
            info!("operation is countered pass run backward");
            return Ok(None);
        }

        info!(
      "generate partial backward {:?} operation id:{}: inventory item:{} location:{:?} partial count: {}",
      &operation_type, &self.id, &self.item_code_ext, &self.location, backward_count,
    );
        if self.count.unsigned_abs() < backward_count {
            return Err(Error::PartialBackwardCountOver(
                backward_count,
                self.count as u32,
            ));
        }
        if self.count.unsigned_abs() == backward_count {
            info!("operation count is equal to backward run backward directly");
            let id = self.run_backward(db, operation_type).await?;
            return Ok(id);
        }
        let mut backward_count = backward_count as i32;
        if self.count.is_positive() {
            backward_count = -backward_count
        }
        let backward = Self::new_countered(
            &self.item_code_ext,
            self.related_id,
            operation_type,
            backward_count,
            self.location.to_owned(),
        );
        let id = backward.run_self(db, false).await?;
        self.update_self_count(db, backward_count).await?;
        Ok(Some(id))
    }

    #[instrument(name="run inventory operation",skip(self,db),fields(
        operation_id=%self.id,
        target_item=%self.item_code_ext,
        operation_type=?self.operation_type,
    ))]
    pub async fn run_self(&self, db: &DbClient, upsert: bool) -> Result<Uuid> {
        info!(
            "run update inventory id:{} item:{} location:{:?} count:{}",
            self.id, &self.item_code_ext, &self.location, self.count
        );
        let query = doc! {
          "item_code_ext":&self.item_code_ext,
        };
        let update = doc! {
          "$inc":{
            "quantity.$[elem].quantity":self.count,
        },
        "$set":{
          "update_at":Local::now(),
        },
        "$push":{"operation_ids":&self.id}
        };
        let filter = UpdateOptions::builder()
            .array_filters(vec![doc! {
              "elem.location":&self.location,
            }])
            .build();
        let res = db
            .ph_db
            .collection::<MongoInventoryItem>(INVENTORY_COL)
            .update_one(query, update, filter)
            .await?;
        if res.matched_count == 0 {
            if upsert {
                info!("need insert item:{}", &self.item_code_ext);
                info!(
                    "run insert inventory id:{} item:{} location:{:?} count:{}",
                    self.id, &self.item_code_ext, &self.location, self.count
                );
                let doc = doc! {
                  "item_code_ext":&self.item_code_ext,
                  "quantity": InventoryLocation::create_new_inventory_quantity_docs(self.count),
                  "created_at":Local::now(),
                  "update_at": Local::now(),
                  "operation_ids": vec![&self.id]
                };
                db.ph_db
                    .collection(INVENTORY_COL)
                    .insert_one(doc, None)
                    .await?;
            } else {
                return Err(Error::InventoryItemNotFound(self.item_code_ext.clone()));
            }
        }
        self.insert_self(db).await?;
        info!("inventory update success");
        Ok(self.id)
    }

    pub async fn run_self_with_session(
        &self,
        db: &DbClient,
        upsert: bool,
        session: &mut ClientSession,
    ) -> Result<Uuid> {
        info!(
            "run update inventory id:{} item:{} location:{:?} count:{}",
            self.id, &self.item_code_ext, &self.location, self.count
        );
        let query = doc! {
          "item_code_ext":&self.item_code_ext,
        };
        let update = doc! {
          "$inc":{
            "quantity.$[elem].quantity":self.count,
        },
        "$set":{
          "update_at":Local::now(),
        },
        "$push":{"operation_ids":&self.id}
        };
        let filter = UpdateOptions::builder()
            .array_filters(vec![doc! {
              "elem.location":&self.location,
            }])
            .build();
        let res = db
            .ph_db
            .collection::<MongoInventoryItem>(INVENTORY_COL)
            .update_one_with_session(query, update, filter, session)
            .await?;
        if res.matched_count == 0 {
            if upsert {
                info!("need insert item:{}", &self.item_code_ext);
                info!(
                    "run insert inventory id:{} item:{} location:{:?} count:{}",
                    self.id, &self.item_code_ext, &self.location, self.count
                );
                let doc = doc! {
                  "item_code_ext":&self.item_code_ext,
                  "quantity": InventoryLocation::create_new_inventory_quantity_docs(self.count),
                  "created_at":Local::now(),
                  "update_at": Local::now(),
                  "operation_ids": vec![&self.id]
                };
                db.ph_db
                    .collection(INVENTORY_COL)
                    .insert_one_with_session(doc, None, session)
                    .await?;
            } else {
                return Err(Error::InventoryItemNotFound(self.item_code_ext.clone()));
            }
        }
        self.insert_self_with_session(db, session).await?;
        info!("inventory update success");
        Ok(self.id)
    }
}

#[derive(Deserialize, Serialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MongoOperationType {
    CreateEmpty,
    Arrival,
    Returned,
    DeleteReturn,
    DeleteRegister,
    DeleteOrder,
    DeleteTransfer,
    UpdateTransfer,
    ConcealOrderItem,
    Ordered,
    Move,
}

impl From<MongoOperationType> for Bson {
    fn from(o: MongoOperationType) -> Self {
        match o {
            MongoOperationType::CreateEmpty => Bson::String(String::from("create_empty")),
            MongoOperationType::Arrival => Bson::String(String::from("arrival")),
            MongoOperationType::Returned => Bson::String(String::from("returned")),
            MongoOperationType::DeleteReturn => Bson::String(String::from("delete_return")),
            MongoOperationType::DeleteRegister => Bson::String(String::from("delete_register")),
            MongoOperationType::DeleteTransfer => Bson::String(String::from("delete_transfer")),
            MongoOperationType::UpdateTransfer => Bson::String(String::from("update_transfer")),
            MongoOperationType::DeleteOrder => Bson::String(String::from("delete_order")),
            MongoOperationType::ConcealOrderItem => {
                Bson::String(String::from("conceal_order_item"))
            }
            MongoOperationType::Ordered => Bson::String(String::from("ordered")),
            MongoOperationType::Move => Bson::String(String::from("move")),
        }
    }
}

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct Operations {
    pub operations: Vec<MongoInventoryOperation>,
}
