use crate::{
    db::{invenope::MongoInventoryOperation, mongo::OPERATIONS_COL},
    error_result::Result,
    server::retrn::NewReturnInputItem,
};
use axum::async_trait;
use chrono::{DateTime as ChronoDT, Local, Utc};
use futures::StreamExt;
use mongodb::bson::{self, doc, DateTime, Document, Uuid};
use serde::{Deserialize, Serialize};
use tracing::info;

use super::{
    invenope::Operations,
    mongo::{DbClient, RETURNS_COL},
    ReturnRepo,
};

#[async_trait]
impl ReturnRepo for DbClient {
    async fn create_new_return(
        &self,
        return_no: &str,
        return_date: ChronoDT<Utc>,
        note: &str,
        items: Vec<NewReturnInputItem>,
    ) -> Result<()> {
        let builder = MongoReturnBuilder::new(return_no, return_date.into(), note, &items);
        builder.publish_new_return(self).await?;
        Ok(())
    }

    async fn query_returns(
        &self,
        from: ChronoDT<Utc>,
        to: ChronoDT<Utc>,
        keyword: Option<String>,
    ) -> Result<Vec<MongoReturnOutput>> {
        Ok(query_returns(self, from.into(), to.into(), keyword).await?)
    }

    async fn get_return_by_id(&self, id: Uuid) -> Result<MongoReturnOutput> {
        Ok(get_return_by_id(self, id).await?)
    }

    async fn delete_return_by_id(&self, id: Uuid) -> Result<()> {
        Ok(delete_return_by_id(self, id).await?)
    }
}

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct MongoReturn {
    pub id: Uuid,
    pub created_at: DateTime,
    pub update_at: DateTime,
    pub return_no: String,
    pub return_date: DateTime,
    pub note: String,
    pub operation_ids: Vec<Uuid>,
}

impl MongoReturn {
    fn new(
        id: Uuid,
        return_no: &str,
        return_date: DateTime,
        note: &str,
        operation_ids: &[Uuid],
    ) -> Self {
        Self {
            id,
            created_at: Local::now().into(),
            update_at: Local::now().into(),
            return_no: return_no.trim().to_owned(),
            return_date,
            note: note.trim().to_owned(),
            operation_ids: operation_ids.to_owned(),
        }
    }

    async fn insert_self(&self, db: &DbClient) -> Result<()> {
        info!("insert new return id:{} no:{}", self.id, &self.return_no);
        let doc = doc! {
          "id":self.id,
          "created_at":self.created_at,
          "update_at":self.update_at,
          "return_no":&self.return_no,
          "return_date":self.return_date,
          "note":&self.note,
          "operation_ids":&self.operation_ids
        };
        db.ph_db
            .collection(RETURNS_COL)
            .insert_one(doc, None)
            .await?;
        info!("insert new return success");
        Ok(())
    }
}

pub struct MongoReturnBuilder {
    pub return_id: Uuid,
    pub return_no: String,
    pub return_date: DateTime,
    pub note: String,
    pub items: Vec<NewReturnInputItem>,
}

impl MongoReturnBuilder {
    pub fn new(
        return_no: &str,
        return_date: DateTime,
        note: &str,
        items: &[NewReturnInputItem],
    ) -> Self {
        Self {
            return_id: Uuid::new(),
            return_no: return_no.to_owned(),
            return_date,
            note: note.to_owned(),
            items: items.to_owned(),
        }
    }

    pub async fn publish_new_return(&self, db: &DbClient) -> Result<MongoReturn> {
        info!("try publish new return id:{}", self.return_id);
        let mut operation_ids = Vec::new();
        //FIXME this new to check did inventory changed before this function run, this is as same as new order.
        //if there are more than one operator race may happen;
        for item in self.items.iter() {
            let id = MongoInventoryOperation::new(
                &item.item_code_ext,
                self.return_id,
                crate::db::invenope::MongoOperationType::Returned,
                -(item.quantity[0].quantity as i32),
                crate::db::inventory::InventoryLocation::JP,
            )
            .run_self(db, false)
            .await?;
            operation_ids.push(id);
        }
        let retrn = MongoReturn::new(
            self.return_id,
            &self.return_no,
            self.return_date,
            &self.note,
            &operation_ids,
        );
        retrn.insert_self(db).await?;
        info!("publish new return id:{} success", self.return_id);
        Ok(retrn)
    }
}

async fn find_operations_by_return_id(
    db: &DbClient,
    return_id: Uuid,
) -> Result<Vec<MongoInventoryOperation>> {
    let query = vec![
        doc! {
          "$match":{
            "id":return_id,
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
        .collection::<Document>(RETURNS_COL)
        .aggregate(query, None)
        .await?;
    let mut operations = Vec::new();
    while let Some(doc) = cursor.next().await {
        let output: Operations = bson::from_document(doc?)?;
        operations.push(output);
    }

    Ok(operations[0].to_owned().operations)
}

pub async fn delete_return_by_id(db: &DbClient, id: Uuid) -> Result<()> {
    info!("try to delete return id:{}", id);
    let operations = find_operations_by_return_id(db, id).await?;
    for operation in operations {
        operation
            .run_backward(db, super::invenope::MongoOperationType::DeleteReturn)
            .await?;
    }
    let query = doc! {
      "id":id
    };
    db.ph_db
        .collection::<MongoReturn>(RETURNS_COL)
        .delete_one(query, None)
        .await?;
    info!("delete return id {} success", id);
    Ok(())
}

#[derive(Deserialize, Serialize, Debug, Clone)]
struct ReturnId {
    id: Uuid,
}

pub async fn query_returns(
    db: &DbClient,
    from: DateTime,
    to: DateTime,
    keyword: Option<String>,
) -> Result<Vec<MongoReturnOutput>> {
    let mut pipeline = vec![
        doc! {
          "$match":{
            "return_date":{
              "$gte":from,
              "$lte":to,
            }
          }
        },
        doc! {
          "$sort":{
            "return_date":-1
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
              {"return_no":{
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
        .collection::<Document>(RETURNS_COL)
        .aggregate(pipeline, None)
        .await?;
    let mut outputs = Vec::new();
    while let Some(doc) = cursor.next().await {
        let output: MongoReturnOutput = bson::from_document(doc?)?;
        outputs.push(output);
    }
    Ok(outputs)
}

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct MongoReturnOutput {
    pub id: Uuid,
    pub created_at: DateTime,
    pub update_at: DateTime,
    pub return_no: String,
    pub return_date: DateTime,
    pub note: String,
    pub items: Vec<MongoReturnItem>,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct MongoReturnItem {
    pub item_code_ext: String,
    pub count: i32,
}

pub async fn get_return_by_id(db: &DbClient, id: Uuid) -> Result<MongoReturnOutput> {
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
        .collection::<Document>(RETURNS_COL)
        .aggregate(pipeline, None)
        .await?;
    let mut outputs = Vec::new();
    while let Some(doc) = cursor.next().await {
        let output: MongoReturnOutput = bson::from_document(doc?)?;
        outputs.push(output);
    }
    Ok(outputs[0].to_owned())
}
