use crate::{
    db::mongo::OPERATIONS_COL,
    error_result::{Error, Result},
};
use axum::async_trait;
use chrono::prelude::*;
use futures::StreamExt;
use mongodb::bson::{self, doc, Document, Uuid};
use serde::{Deserialize, Serialize};
use tracing::{info, instrument};

use super::{
    invenope::{MongoInventoryOperation, MongoOperationType, Operations},
    inventory::{self, InventoryLocation},
    mongo::{DbClient, REGISTERS_COL},
    PhDataBase, PhItem, RegisterItemInput, RegisterRepo, StockRegisterInput,
};

#[async_trait]
impl RegisterRepo for DbClient {
    async fn insert_stock_register(&self, input: &StockRegisterInput) -> Result<()> {
        let builder = MongoRegisterBuilder::new(input.arrival_date.into(), &input.no, &input.items);
        builder.publish_mongo_register(self).await?;
        Ok(())
    }

    async fn delete_stock_register(&self, register_id: Uuid) -> Result<String> {
        info!("new delete register request id:{}", register_id);
        delete_stock_register(self, register_id).await?;
        Ok(register_id.to_string())
    }

    async fn find_register_by_no(&self, no: &str) -> Result<Vec<MongoRegisterOutput>> {
        Ok(find_register_by_no(self, no).await?)
    }

    async fn get_register_by_id(&self, id: Uuid) -> Result<MongoRegisterOutput> {
        Ok(get_register_by_id(self, id).await?)
    }
    async fn query_registers(
        &self,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
        keyword: Option<String>,
        page: Option<u32>,
    ) -> Result<(bool, Vec<MongoRegisterOutput>)> {
        Ok(query_registers(self, from.into(), to.into(), keyword, page).await?)
    }
}

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct MongoRegister {
    pub id: Uuid,
    pub created_at: mongodb::bson::DateTime,
    pub arrival_date: mongodb::bson::DateTime,
    pub no: String,
    pub operation_ids: Vec<Uuid>,
}

impl MongoRegister {
    fn new(
        id: Uuid,
        arrival_date: mongodb::bson::DateTime,
        no: &str,
        operation_ids: Vec<Uuid>,
    ) -> Self {
        Self {
            id,
            created_at: Local::now().into(),
            arrival_date,
            no: no.to_owned(),
            operation_ids,
        }
    }

    async fn insert_self(&self, db: &DbClient) -> Result<Uuid> {
        info!(
            "insert new register id:{},arrival date:{}",
            self.id, self.arrival_date
        );
        let doc = doc! {
        "id":self.id,
        "created_at": self.created_at,
        "arrival_date":self.arrival_date,
        "no":&self.no,
        "operation_ids":&self.operation_ids,
         };
        db.ph_db
            .collection(REGISTERS_COL)
            .insert_one(doc, None)
            .await?;
        info!("insert register success");
        Ok(self.id)
    }
}

pub struct MongoRegisterBuilder {
    pub register_id: Uuid,
    pub arrival_date: mongodb::bson::DateTime,
    pub register_no: String,
    pub items: Vec<RegisterItemInput>,
}

impl MongoRegisterBuilder {
    pub fn new(
        arrival_date: mongodb::bson::DateTime,
        register_no: &str,
        items: &[RegisterItemInput],
    ) -> Self {
        Self {
            register_id: Uuid::new(),
            arrival_date,
            register_no: register_no.to_owned(),
            items: items.to_owned(),
        }
    }

    pub async fn publish_mongo_register(self, db: &DbClient) -> Result<MongoRegister> {
        let operation_ids = self.update_inventory(db).await?;
        let register = MongoRegister::new(
            self.register_id,
            self.arrival_date,
            &self.register_no,
            operation_ids,
        );
        register.insert_self(db).await?;
        Ok(register)
    }

    async fn update_inventory(&self, db: &DbClient) -> Result<Vec<Uuid>> {
        let mut ope_ids = Vec::new();
        for item in self.items.iter() {
            if item.is_manual {
                info!("detected manual input item");
                let item_opt = db.find_one_by_item_code(&item.item_code_ext[0..11]).await?;
                if item_opt.is_none() {
                    info!(
                        "item is not found in db create a new dummy for {} price:{}",
                        &item.item_code_ext[0..11],
                        item.price
                    );
                    PhItem::new_dummy(&item.item_code_ext, item.price)
                        .insert_self(db)
                        .await?
                }
            }
            let operation = MongoInventoryOperation::new(
                &item.item_code_ext,
                self.register_id,
                MongoOperationType::Arrival,
                item.count as i32,
                InventoryLocation::JP,
            );
            let ope_id = operation.run_self(db, true).await?;
            ope_ids.push(ope_id);
        }
        info!("inventory update finish");
        Ok(ope_ids)
    }
}

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct MongoRegisterOutput {
    pub id: Uuid,
    pub created_at: mongodb::bson::DateTime,
    pub arrival_date: mongodb::bson::DateTime,
    pub no: String,
    pub items: Vec<MongoRegisterItem>,
}

/// because super::RegisterItem rename to camelCase,
/// bson::from_document can not deserialize super::Register correctly,
/// so I need this to convert.
#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct MongoRegisterItem {
    pub item_code_ext: String,
    pub count: u32,
}

pub async fn find_register_by_no(db: &DbClient, no: &str) -> Result<Vec<MongoRegisterOutput>> {
    let query = vec![
        doc! {
          "$match":{
            "no":no,
          },
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
        .collection::<Document>(REGISTERS_COL)
        .aggregate(query, None)
        .await?;
    let mut registers = Vec::new();
    while let Some(doc) = cursor.next().await {
        let register: MongoRegisterOutput = bson::from_document(doc?)?;
        registers.push(register);
    }

    Ok(registers)
}

#[derive(Deserialize, Serialize, Debug, Clone)]
struct RegisterIds {
    id: Uuid,
}
const ITEMS_PER_PAGE_LOCAL: u32 = 9;
pub async fn query_registers(
    db: &DbClient,
    from: mongodb::bson::DateTime,
    to: mongodb::bson::DateTime,
    keyword: Option<String>,
    page: Option<u32>,
) -> Result<(bool, Vec<MongoRegisterOutput>)> {
    let mut pipeline = vec![
        doc! {
        "$match":{
          "created_at":{
            "$gte":from,
            "$lte":to,
          }
        }},
        doc! {
          "$lookup":{
              "from": OPERATIONS_COL,
              "localField": "operation_ids",
              "foreignField": "id",
              "as": "items",
          },
        },
        doc! {
         "$sort":{
           "created_at":-1
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
            ]
          }
        })
    }
    if page.is_none() {
        let mut cursor = db
            .ph_db
            .collection::<Document>(REGISTERS_COL)
            .aggregate(pipeline, None)
            .await?;
        let mut outputs = Vec::new();
        while let Some(doc) = cursor.next().await {
            let output: MongoRegisterOutput = bson::from_document(doc?)?;
            outputs.push(output);
        }
        return Ok((false, outputs));
    }
    // reach here means this is a paged request
    let page = page.unwrap();
    let skip = ITEMS_PER_PAGE_LOCAL * page;

    pipeline.push(doc! {
        "$limit":ITEMS_PER_PAGE_LOCAL +skip
    });

    pipeline.push(doc! {
        "$skip":skip
    });

    let mut cursor = db
        .ph_db
        .collection::<Document>(REGISTERS_COL)
        .aggregate(pipeline, None)
        .await?;
    let mut outputs = Vec::new();
    while let Some(doc) = cursor.next().await {
        let output: MongoRegisterOutput = bson::from_document(doc?)?;
        outputs.push(output);
    }
    Ok(((outputs.len() as u32) == ITEMS_PER_PAGE_LOCAL, outputs))
}

pub async fn get_register_by_id(db: &DbClient, id: Uuid) -> Result<MongoRegisterOutput> {
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
          },
        },
    ];

    let mut cursor = db
        .ph_db
        .collection::<Document>(REGISTERS_COL)
        .aggregate(pipeline, None)
        .await?;
    let mut outputs = Vec::new();
    while let Some(doc) = cursor.next().await {
        let output: MongoRegisterOutput = bson::from_document(doc?)?;
        outputs.push(output);
    }
    Ok(outputs[0].to_owned())
}

#[instrument(name = "delete register inner", skip(db, register_id))]
pub async fn delete_stock_register(db: &DbClient, register_id: Uuid) -> Result<String> {
    info!("try delete register:{register_id}");
    // get register related operations
    let operations = find_operations_by_register_id(db, register_id).await?;
    // registerが登録された後、registerが影響したinventory itemのquantityがbackwardされたらまずい
    // operationががされていないかチェックします。
    // ArrivalとDeleteRegister以外はまずいoperationになります。
    // check register operations one by one
    info!("find {} operation(s)", operations.len());
    for operation in operations.iter() {
        // get current operation's related inventory item's operation in from new to old order.
        if !inventory::is_operation_could_be_backward_safely(db, operation).await? {
            return Err(Error::RegisterCanNotDelete);
        }
        // if reach this line register can be deleted safely so run operation backward.
        operation
            .run_backward(db, MongoOperationType::DeleteRegister)
            .await?;
    }
    let query = doc! {
      "id":register_id,
    };
    db.ph_db
        .collection::<MongoRegister>(REGISTERS_COL)
        .delete_one(query, None)
        .await?;
    info!("delete id:{} success", register_id);
    Ok(register_id.to_string())
}

async fn find_operations_by_register_id(
    db: &DbClient,
    id: Uuid,
) -> Result<Vec<MongoInventoryOperation>> {
    info!("find register:{}'s operations", id);
    let query = vec![
        doc! {
          "$match":{
            "id":id,
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
        .collection::<Document>(REGISTERS_COL)
        .aggregate(query, None)
        .await?;
    let mut operations = Vec::new();
    while let Some(doc) = cursor.next().await {
        let register: Operations = bson::from_document(doc?)?;
        operations.push(register);
    }

    Ok(operations[0].to_owned().operations)
}
