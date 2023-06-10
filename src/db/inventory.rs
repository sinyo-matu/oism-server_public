use crate::{
    db::{invenope::MongoOperationType, mongo::OPERATIONS_COL},
    error_result::{Error, Result},
    server::inventory::InventoryQuery,
};
use axum::async_trait;
use futures::StreamExt;
use mongodb::bson::{self, Bson};
use mongodb::bson::{doc, Document};
use mongodb::{bson::Uuid, ClientSession};
use serde::{Deserialize, Serialize};
use strum::{EnumIter, IntoEnumIterator};
use tracing::info;

use super::{
    invenope::{MongoInventoryOperation, Operations},
    mongo::{DbClient, INVENTORY_COL},
    InventoryRepo,
};
#[async_trait]
impl InventoryRepo for DbClient {
    async fn query_inventory(
        &self,
        query: InventoryQuery,
    ) -> Result<(bool, Vec<MongoInventoryOutput>)> {
        Ok(query_inventory(self, query).await?)
    }

    async fn get_inventory_item_operations(
        &self,
        item_code_ext: &str,
    ) -> Result<Vec<MongoInventoryOperation>> {
        Ok(find_inventory_item_operations_by_item_code_ext(self, item_code_ext).await?)
    }

    async fn find_inventory_by_item_code_ext(
        &self,
        item_code_ext: &str,
    ) -> Result<Option<MongoInventoryItem>> {
        Ok(find_inventory_by_item_code_ext(self, item_code_ext).await?)
    }
}

#[derive(Deserialize, Serialize, Debug, Clone)]

pub struct MongoInventoryItem {
    pub item_code_ext: String,
    pub quantity: Vec<Quantity>,
    pub created_at: mongodb::bson::DateTime,
    pub update_at: mongodb::bson::DateTime,
    pub operation_ids: Vec<Uuid>,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct MongoInventoryOutput {
    pub item_code_ext: String,
    pub quantity: Vec<Quantity>,
    pub created_at: mongodb::bson::DateTime,
    pub update_at: mongodb::bson::DateTime,
    pub operation_ids: Vec<Uuid>,
}

#[derive(Deserialize, Serialize, Debug, Clone, Copy)]
pub struct Quantity {
    pub location: InventoryLocation,
    pub quantity: u32,
}

#[derive(Deserialize, Serialize, Debug, Clone, PartialEq, Eq, Copy, EnumIter)]
#[serde(rename_all = "lowercase")]
pub enum InventoryLocation {
    JP,
    CN,
    PCN,
}

impl InventoryLocation {
    /// will create a new inventory collection quantity docs
    /// and jp location will be set to `count` value
    pub fn create_new_inventory_quantity_docs(count: i32) -> Vec<Document> {
        let mut docs = Vec::new();
        for location in InventoryLocation::iter() {
            match location {
                InventoryLocation::JP => docs.push(doc! {
                  "location":location,
                  "quantity":count
                }),
                _ => docs.push(doc! {
                  "location":location,
                  "quantity":0
                }),
            }
        }
        docs
    }

    pub fn kanjified(&self) -> String {
        match self {
            Self::JP => String::from("日本"),
            Self::CN => String::from("中国"),
            Self::PCN => String::from("中国済"),
        }
    }
}

impl From<InventoryLocation> for Bson {
    fn from(l: InventoryLocation) -> Self {
        match l {
            InventoryLocation::CN => Bson::String(String::from("cn")),
            InventoryLocation::JP => Bson::String(String::from("jp")),
            InventoryLocation::PCN => Bson::String(String::from("pcn")),
        }
    }
}

impl InventoryLocation {
    pub fn is_paid(self) -> bool {
        matches!(self, InventoryLocation::PCN)
    }
}

pub async fn is_operation_could_be_backward_safely(
    db: &DbClient,
    operation: &MongoInventoryOperation,
) -> Result<bool> {
    info!(
        "start checking of operation id:{} item_code:{},quantity:{}",
        operation.id, operation.item_code_ext, operation.count
    );
    let inventory_item_operations =
        find_inventory_item_operations_by_item_code_ext(db, &operation.item_code_ext).await?;
    // reference inventory items' operations one by one until reach the current
    // register related operation see if there are unsafe operations has been run.
    for inventory_item_operation in inventory_item_operations.into_iter() {
        info!(
            "check operation id:{} type:{:?}",
            inventory_item_operation.id, inventory_item_operation.operation_type
        );
        // if react the current register operation which we want to delete. break the loop
        if inventory_item_operation.id == operation.id {
            info!("reach target break");
            break;
        }
        // if current inventory item's related operation is countered it can be deleted safely, skip it.
        if inventory_item_operation.countered {
            info!("countered continue");
            continue;
        }
        // check current inventory item's operation type if it is not the Arrival or CreateEmpty type
        // return Error.
        match inventory_item_operation.operation_type {
            MongoOperationType::Arrival | MongoOperationType::CreateEmpty => {}
            _ => return Ok(false),
        }
    }
    Ok(true)
}

pub async fn find_inventory_item_operations_by_item_code_ext(
    db: &DbClient,
    item_code_ext: &str,
) -> Result<Vec<MongoInventoryOperation>> {
    let pipeline = vec![
        doc! {
          "$match":{
            "item_code_ext":&item_code_ext,
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
        .collection::<Document>(INVENTORY_COL)
        .aggregate(pipeline, None)
        .await?;
    let mut operations_vec = Vec::new();
    while let Some(doc) = cursor.next().await {
        let operations: Operations = bson::from_document(doc?)?;
        operations_vec.push(operations);
    }
    operations_vec[0]
        .operations
        .sort_by(|a, b| b.time.cmp(&a.time));
    Ok(operations_vec[0]
        .to_owned()
        .operations
        .into_iter()
        .collect())
}

// pub async fn find_inventory_operation_by_id(
//   db: &DbClient,
//   operation_id: &str,
// ) -> Result<MongoInventoryOperation> {
//   let filter = doc! {
//     "id" :operation_id,
//   };
//   let res = db
//     .ph_db
//     .collection::<MongoInventoryOperation>(OPERATIONS_COL)
//     .find_one(filter, None)
//     .await?;
//   Ok(res.ok_or(Error::OperationNotFound)?)
// }

const ITEMS_PER_PAGE_LOCAL: u32 = 35;

async fn query_inventory(
    db: &DbClient,
    query: InventoryQuery,
) -> Result<(bool, Vec<MongoInventoryOutput>)> {
    let mut pipeline = vec![
        doc! {
          "$addFields":{
            "item_code_pre":{"$substrCP":["$item_code_ext",0,5]},
            "item_code_mid":{"$substrCP":["$item_code_ext",5,3]},
            "item_code_post":{"$substrCP":["$item_code_ext",8,3]},
            "item_code":{"$substrCP":["$item_code_ext",0,11]},
            "size_no":{"$substrCP":["$item_code_ext",12,1]},
            "color_no":{"$substrCP":["$item_code_ext",13,1]},
          }
        },
        doc! {
            "$lookup":{
              "from": "items",
        "localField": "item_code",
        "foreignField": "code",
        "as": "item",
            }
          },
        doc! {
          "$addFields":{
            "item_name":{"$arrayElemAt":["$item.item_name",0]}
          }
        },
        doc! {
          "$sort":{
            "update_at":-1,
            "item_code_pre":-1,
            "item_code_mid":1,
            "item_code_post":1,
            "size_no":1,
            "color_no":1,
          }
        },
    ];
    if !query.show_zero_quantity {
        pipeline.push(doc! {
          "$match":{
           "quantity":{
            "$elemMatch":{
              "quantity":{
                "$gt":0
              }
            }
           }
          }
        })
    }

    if let Some(location) = query.location {
        let locations: Vec<&str> = location.rsplit(',').collect();
        pipeline.push(doc! {
          "$match":{
            "quantity":{
              "$elemMatch":{
                "location":{"$in":locations},
                "quantity":{
                  "$gt":0,
                }
              }
            }
          }
        })
    }

    if let Some(category) = query.category {
        let category_content = category.to_concrete_content();
        let category_vec: Vec<&str> = category_content.trim().split(' ').collect();
        let mut or_value = vec![];
        for category in category_vec {
            or_value.push(doc! {"item.category":category});
        }
        pipeline.push(doc! {
          "$match":{
              "$or": or_value
          }
        })
    }
    if !query.keyword.is_empty() {
        pipeline.push(doc! {
          "$match":{
          "$or":[
            {"item.code":
            {
                          "$regex":&query.keyword,
                          "$options":"i"
                      }},

                      {"item.item_name":{
                          "$regex":&query.keyword,
                          "$options":"i"
                      }},
                      {"item.item_name_zh":{
                          "$regex":&query.keyword,
                          "$options":"i"
                      }},
          ]
        }
        })
    }
    pipeline.push(doc! {
      "$project":{
        "item_code_pre":0,
        "item_code_mid":0,
        "item_code_post":0,
        "item":0,
        "color_no":0,
        "size_no":0,
      }
    });
    if query.page.is_none() {
        let mut cursor = db
            .ph_db
            .collection::<MongoInventoryItem>(INVENTORY_COL)
            .aggregate(pipeline, None)
            .await?;
        let mut items = Vec::new();
        while let Some(doc) = cursor.next().await {
            items.push(bson::from_document(doc?)?)
        }
        return Ok((false, items));
    }

    let page = query.page.unwrap();
    let skip = ITEMS_PER_PAGE_LOCAL * page;

    pipeline.push(doc! {
        "$limit":ITEMS_PER_PAGE_LOCAL +skip
    });

    pipeline.push(doc! {
        "$skip":skip
    });

    let mut cursor = db
        .ph_db
        .collection::<MongoInventoryItem>(INVENTORY_COL)
        .aggregate(pipeline, None)
        .await?;
    let mut items = Vec::new();
    while let Some(doc) = cursor.next().await {
        items.push(bson::from_document(doc?)?)
    }
    Ok(((items.len() as u32) == ITEMS_PER_PAGE_LOCAL, items))
}
pub async fn find_inventory_by_item_code_ext(
    db: &DbClient,
    item_code_ext: &str,
) -> Result<Option<MongoInventoryItem>> {
    let filter = doc! {
      "item_code_ext":item_code_ext,
    };

    let res = db
        .ph_db
        .collection::<MongoInventoryItem>(INVENTORY_COL)
        .find_one(filter, None)
        .await?;
    Ok(res)
}

pub async fn find_inventory_by_item_code_ext_with_session(
    db: &DbClient,
    item_code_ext: &str,
    session: &mut ClientSession,
) -> Result<Option<MongoInventoryItem>> {
    let filter = doc! {
      "item_code_ext":item_code_ext,
    };

    let res = db
        .ph_db
        .collection::<MongoInventoryItem>(INVENTORY_COL)
        .find_one_with_session(filter, None, session)
        .await?;
    Ok(res)
}

pub async fn shift_inventory_quantity(
    db: &DbClient,
    item_code_ext: &str,
    quantity: &[Quantity],
    related_id: Uuid,
) -> Result<Vec<Uuid>> {
    let mut operation_ids = Vec::new();
    let inventory_opt = find_inventory_by_item_code_ext(db, item_code_ext).await?;
    if inventory_opt.is_none() {
        return Err(Error::InventoryNotFound);
    }
    let inventory = inventory_opt.unwrap();
    // check if the requested accumulated inventory quantity of
    // every location equal to current
    let current_quantity = inventory.quantity.iter().fold(0, |mut acc, current_q| {
        acc += current_q.quantity;
        acc
    });
    let request_quantity = quantity.iter().fold(0, |mut acc, current_q| {
        acc += current_q.quantity;
        acc
    });
    if current_quantity != request_quantity {
        return Err(Error::Changed);
    }
    //////////////////////////////////////////////////////////

    let zip = inventory.quantity.iter().zip(quantity);
    // loop over every location
    for (current_quantity, requested_quantity) in zip {
        if current_quantity.quantity == requested_quantity.quantity {
            continue;
        }
        let move_quantity = requested_quantity.quantity as i32 - current_quantity.quantity as i32;
        info!(
            "supply {} to {:?}",
            move_quantity, requested_quantity.location
        );
        let operation = MongoInventoryOperation::new(
            item_code_ext,
            related_id,
            super::invenope::MongoOperationType::Move,
            move_quantity,
            requested_quantity.location,
        );
        let id = operation.run_self(db, false).await?;
        operation_ids.push(id);
    }

    Ok(operation_ids)
}
