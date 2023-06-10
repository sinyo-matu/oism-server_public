use crate::{
    db::{
        inventory::{
            find_inventory_by_item_code_ext, find_inventory_by_item_code_ext_with_session,
        },
        mongo::OPERATIONS_COL,
        shipment::get_shipment_by_id,
        transfer::MongoTransferBuilder,
        InventoryRepo, PhDataBase,
    },
    error_result::{Error, Result},
    server::{transfer::NewTransferInputItem, InputOrderItem, OrderRegisterInput},
};
use async_recursion::async_recursion;
use axum::async_trait;
use chrono::prelude::*;
use chrono::serde::ts_seconds;
use futures::StreamExt;
use mongodb::{
    bson::{self, bson, doc, Bson, Document, Uuid},
    error::UNKNOWN_TRANSACTION_COMMIT_RESULT,
    options::{
        Acknowledgment, AggregateOptions, Collation, ReadConcern, TransactionOptions, WriteConcern,
    },
    ClientSession,
};
use serde::{Deserialize, Serialize};
use tracing::{debug, info, instrument};

use self::domain::TaobaoOrderNo;

use super::{
    invenope::{MongoInventoryOperation, MongoOperationType, Operations},
    inventory::{InventoryLocation, MongoInventoryItem},
    mongo::{DbClient, ORDERS_COL, ORDER_ITEMS_COL},
    OrderRepo, PhItem, RegisterItem,
};

pub struct DeleteOrderOutput {
    pub deleted_items: Vec<MongoOrderItem>,
    pub item_is_shipped_ids: Vec<Uuid>,
}

pub struct ConcealItemOutput {
    pub concealed_item: MongoOrderItem,
    pub is_shipped: bool,
}

#[async_trait]
impl OrderRepo for DbClient {
    #[instrument(name = "create order in db", skip(self, input))]
    async fn create_order(&self, input: OrderRegisterInput) -> Result<()> {
        info!("new create order request");
        let order_builder = MongoOrderBuilder::new(
            TaobaoOrderNo::parse(&input.taobao_order_no)?,
            &input.customer_id,
            &input.note,
            &input.items,
            input.order_datetime.into(),
        );
        let _order = order_builder.publish_mongo_order(self).await?;
        Ok(())
    }

    async fn query_orders(
        &self,
        keyword: &str,
        status: &str,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
        page: Option<u32>,
    ) -> Result<(bool, Vec<MongoOrderOutput>)> {
        Ok(query_orders(self, keyword, status, from.into(), to.into(), page).await?)
    }

    async fn check_then_update_order_status(
        &self,
        items: Vec<RegisterItem>,
    ) -> Result<Vec<MongoOrderItem>> {
        Ok(check_then_update_order_status(self, items).await?)
    }
    async fn get_order_by_id(&self, id: Uuid) -> Result<MongoOrderOutput> {
        Ok(get_order_by_id(self, id).await?)
    }

    async fn get_order_by_taobao_no(&self, taobao_order_no: &str) -> Result<Vec<MongoOrderOutput>> {
        let taobao_no = TaobaoOrderNo::parse(taobao_order_no)?;
        let pipeline = vec![
            doc! {
              "$match":{
                "taobao_order_no":taobao_no.get_inner()
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
            .collection::<MongoOrderOutput>(ORDERS_COL)
            .aggregate(pipeline, None)
            .await?;
        let mut outputs = Vec::new();
        while let Some(doc) = cursor.next().await {
            let output: MongoOrderOutput = bson::from_document(doc?)?;
            outputs.push(output);
        }
        Ok(outputs)
    }

    async fn delete_order(&self, order_id: Uuid) -> Result<DeleteOrderOutput> {
        info!("new delete order request id:{}", order_id);
        Ok(delete_order(self, order_id).await?)
    }

    async fn get_order_item_by_id(&self, order_item_id: Uuid) -> Result<MongoOrderItem> {
        Ok(find_order_item_by_id(self, order_item_id).await?)
    }

    async fn conceal_order_item(&self, order_item_id: Uuid) -> Result<ConcealItemOutput> {
        info!("new conceal order item request id:{}", order_item_id);
        Ok(conceal_order_item(self, order_item_id).await?)
    }

    async fn update_order_note(&self, order_id: Uuid, note: &str) -> Result<()> {
        info!("update order note request id:{},note:{}", order_id, note);
        Ok(update_order_note(self, order_id, note).await?)
    }

    async fn query_order_items(
        &self,
        keyword: &str,
        status: &OrderItemStatus,
    ) -> Result<Vec<MongoOrderItem>> {
        Ok(query_order_items(self, keyword, status).await?)
    }

    async fn update_order_item_rate(&self, id: Uuid, rate: f64) -> Result<()> {
        let rate = OrderItemRate::parse(rate)?;
        Ok(update_order_item_rate(self, id, rate).await?)
    }
}

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct MongoOrder {
    pub id: Uuid,
    pub created_at: mongodb::bson::DateTime,
    pub update_at: mongodb::bson::DateTime,
    pub order_datetime: mongodb::bson::DateTime,
    pub taobao_order_no: String,
    pub customer_id: String,
    pub note: String,
    pub order_item_ids: Vec<Uuid>,
    pub operation_ids: Vec<Uuid>,
}
impl MongoOrder {
    fn new(
        id: Uuid,
        taobao_order_no: &str,
        customer_id: &str,
        note: &str,
        order_item_ids: &[Uuid],
        operation_ids: &[Uuid],
        order_datetime: bson::DateTime,
    ) -> Self {
        Self {
            id,
            created_at: Local::now().into(),
            update_at: Local::now().into(),
            order_datetime,
            taobao_order_no: taobao_order_no.to_owned(),
            customer_id: customer_id.to_owned(),
            note: note.to_owned(),
            order_item_ids: order_item_ids.to_owned(),
            operation_ids: operation_ids.to_owned(),
        }
    }

    async fn insert_self(&self, db: &DbClient) -> Result<()> {
        let doc = doc! {
          "id":self.id,
          "created_at":self.created_at,
          "update_at":self.update_at,
          "order_datetime":self.order_datetime,
          "taobao_order_no":&self.taobao_order_no,
          "customer_id":&self.customer_id,
          "note":&self.note,
          "order_item_ids":&self.order_item_ids,
          "operation_ids":&self.operation_ids,
        };
        db.ph_db
            .collection(ORDERS_COL)
            .insert_one(doc, None)
            .await?;
        Ok(())
    }
}

pub struct MongoOrderBuilder {
    pub order_id: Uuid,
    pub taobao_order_no: String,
    pub customer_id: String,
    pub note: String,
    pub items: Vec<InputOrderItem>,
    pub order_datetime: bson::DateTime,
}

impl MongoOrderBuilder {
    pub fn new(
        taobao_order_no: TaobaoOrderNo,
        customer_id: &str,
        note: &str,
        items: &[InputOrderItem],
        order_datetime: bson::DateTime,
    ) -> Self {
        Self {
            order_id: Uuid::new(),
            taobao_order_no: taobao_order_no.get_inner(),
            customer_id: customer_id.trim().to_owned(),
            note: note.trim().to_owned(),
            items: items.to_owned(),
            order_datetime,
        }
    }

    pub async fn publish_mongo_order(&self, db: &DbClient) -> Result<MongoOrder> {
        let (order_item_ids, operation_ids) = self.create_order_items(db).await?;
        let order = MongoOrder::new(
            self.order_id,
            &self.taobao_order_no,
            &self.customer_id,
            &self.note,
            &order_item_ids,
            &operation_ids,
            self.order_datetime,
        );
        order.insert_self(db).await?;
        Ok(order)
    }

    #[instrument(name = "create order items in db", skip(self, db),fields(
        order_id = %self.order_id
    ))]
    async fn create_order_items(&self, db: &DbClient) -> Result<(Vec<Uuid>, Vec<Uuid>)> {
        let mut operation_ids = Vec::new();
        let mut order_item_ids = Vec::new();
        for input_item in self.items.iter() {
            if input_item.is_manual {
                create_dummy_phitem(db, &input_item.item_code_ext[0..11], input_item.price).await?;
            }
            let inventory =
                get_inventory_item(db, &input_item.item_code_ext, self.order_id).await?;
            // WARNING
            // this process need order in inventory.quantity and input_item.quantity to be same.
            let zipped = inventory.quantity.iter().zip(input_item.quantity.iter());
            // this will see in stock and requested by location continuously.
            for (in_stock, requested) in zipped {
                debug!(
                    "in_stock location:{:?},requested location: {:?}",
                    in_stock.location, requested.location
                );
                assert!(in_stock.location == requested.location);
                // if not requested in this location pass it.
                if requested.quantity == 0 {
                    info!(
                        "location:{:?} is requested quantity:{} so pass",
                        &requested.location, requested.quantity
                    );
                    continue;
                }
                // if quantity in stock of this location is 0 publish back ordering Order item immediately.
                if in_stock.quantity == 0 {
                    info!(
                        "location:{:?} have 0 in stock inventory create backordering order item",
                        requested.location
                    );
                    let item_b_ids = create_backordering_order_item(
                        db,
                        self,
                        input_item,
                        requested.location,
                        requested.quantity,
                    )
                    .await?;
                    order_item_ids.extend(item_b_ids);
                    continue;
                }
                // if theres is in stock quantity and it can cover requested quantity.
                // publish order item by requested quantity.
                if in_stock.quantity >= requested.quantity {
                    info!(
                    "in stock location:{:?} has enough quantity:{} which be requested :{} publish new inventory operation",
                    &in_stock.location, in_stock.quantity, requested.quantity
                    );
                    let (operation_id, item_ids) = create_guaranteed_order_item(
                        db,
                        self,
                        input_item,
                        requested.location,
                        requested.quantity,
                    )
                    .await?;
                    operation_ids.push(operation_id);
                    order_item_ids.extend(item_ids);
                    continue;
                }
                // last there is in stock quantity but it is not enough for requested.
                // publish order item for every in stock quantity for guaranteed.
                // then publish left requested quantity by back ordering.
                info!(
                "inventory in stock location:{:?} is not enough in stock:{} which be requested:{},publish new Guaranteed and BackOrdering order item",
                &in_stock.location, in_stock.quantity, requested.quantity
                );
                let (operation_id, item_ids) = create_guaranteed_order_item(
                    db,
                    self,
                    input_item,
                    requested.location,
                    in_stock.quantity,
                )
                .await?;
                operation_ids.push(operation_id);
                order_item_ids.extend(item_ids);
                let item_b_ids = create_backordering_order_item(
                    db,
                    self,
                    input_item,
                    requested.location,
                    requested.quantity - in_stock.quantity,
                )
                .await?;
                order_item_ids.extend(item_b_ids);
            }
        }
        Ok((order_item_ids, operation_ids))
    }
}

#[instrument(name = "get inventory item", skip(db, order_id, item_code_ext))]
async fn get_inventory_item(
    db: &DbClient,
    item_code_ext: &str,
    order_id: Uuid,
) -> Result<MongoInventoryItem> {
    match db.find_inventory_by_item_code_ext(item_code_ext).await? {
        None => {
            info!(
                "code:{} not found,create new empty inventory item",
                &item_code_ext
            );
            let operation = MongoInventoryOperation::new(
                item_code_ext,
                order_id,
                MongoOperationType::CreateEmpty,
                0,
                InventoryLocation::JP,
            );
            operation.run_self(db, true).await?;
            let item = db
                .find_inventory_by_item_code_ext(item_code_ext)
                .await?
                .ok_or_else(|| Error::InventoryItemNotFound(item_code_ext.to_owned()))?;
            Ok(item)
        }
        Some(item) => Ok(item),
    }
}

#[instrument(name = "create dummy phitem", skip(db))]
async fn create_dummy_phitem(db: &DbClient, item_code: &str, item_price: u32) -> Result<()> {
    let item_opt = db.find_one_by_item_code(item_code).await?;
    if item_opt.is_none() {
        info!("item is not found in db create a new dummy",);
        PhItem::new_dummy(item_code, item_price)
            .insert_self(db)
            .await?
    }
    Ok(())
}
#[instrument(name="create backordering order item",skip(db,builder,input_item,location,quantity),fields(
    item_code_ext=%input_item.item_code_ext,
    location=?location,
    quantity=%quantity,
))]
async fn create_backordering_order_item(
    db: &DbClient,
    builder: &MongoOrderBuilder,
    input_item: &InputOrderItem,
    location: InventoryLocation,
    quantity: u32,
) -> Result<Vec<Uuid>> {
    let order_item_b = MongoOrderItem::new(
        &input_item.item_code_ext,
        location,
        builder.order_datetime,
        input_item.rate,
        OrderItemStatus::BackOrdering,
        builder.order_id,
        &builder.customer_id,
        &builder.note,
    );
    order_item_b.insert_self(db, quantity).await
}

#[instrument(name="create guaranteed order item",skip(db,builder,input_item,location,quantity),fields(
    item_code_ext=%input_item.item_code_ext,
    location=?location,
    quantity=%quantity,
))]
async fn create_guaranteed_order_item(
    db: &DbClient,
    builder: &MongoOrderBuilder,
    input_item: &InputOrderItem,
    location: InventoryLocation,
    quantity: u32,
) -> Result<(Uuid, Vec<Uuid>)> {
    let operation = MongoInventoryOperation::new(
        &input_item.item_code_ext,
        builder.order_id,
        MongoOperationType::Ordered,
        -(quantity as i32),
        location,
    );
    //FIXME maybe we let order_item hold the operation id,then the item conceal operation should be more simple.
    let operation_id = operation.run_self(db, false).await?;
    let order_item_b = MongoOrderItem::new(
        &input_item.item_code_ext,
        location,
        builder.order_datetime,
        input_item.rate,
        OrderItemStatus::Guaranteed,
        builder.order_id,
        &builder.customer_id,
        &builder.note,
    );
    Ok((operation_id, order_item_b.insert_self(db, quantity).await?))
}

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct MongoOrderItem {
    pub id: Uuid,
    pub created_at: mongodb::bson::DateTime,
    pub update_at: mongodb::bson::DateTime,
    pub order_datetime: mongodb::bson::DateTime,
    pub customer_id: String,
    pub item_code_ext: String,
    pub rate: f64,
    pub location: InventoryLocation,
    pub status: OrderItemStatus,
    pub order_id: Uuid,
    pub note: String,
    pub shipment_id: Option<Uuid>,
}

impl MongoOrderItem {
    #[allow(clippy::too_many_arguments)]
    fn new(
        item_code_ext: &str,
        location: InventoryLocation,
        order_datetime: bson::DateTime,
        rate: f64,
        status: OrderItemStatus,
        order_id: Uuid,
        customer_id: &str,
        note: &str,
    ) -> Self {
        Self {
            id: Uuid::new(),
            created_at: Local::now().into(),
            update_at: Local::now().into(),
            order_datetime,
            customer_id: customer_id.to_owned(),
            item_code_ext: item_code_ext.to_owned(),
            rate,
            location: location.to_owned(),
            note: note.to_owned(),
            status,
            order_id,
            shipment_id: None,
        }
    }

    async fn insert_self(&self, db: &DbClient, quantity: u32) -> Result<Vec<Uuid>> {
        info!(
            "try insert {} order item id:{} code:{}",
            quantity, self.id, &self.item_code_ext
        );
        let mut docs = Vec::new();
        let mut ids = Vec::new();
        for _ in 0..quantity {
            let id = Uuid::new();
            let doc = doc! {
              "id":id,
              "created_at":self.created_at,
              "update_at":self.update_at,
              "order_datetime":self.order_datetime,
              "item_code_ext":&self.item_code_ext,
              "customer_id":&self.customer_id,
              "rate":self.rate,
              "location":&self.location,
              "note":&self.note,
              "status":&self.status,
              "order_id":self.order_id,
              "shipment_id":self.shipment_id,
            };
            docs.push(doc);
            ids.push(id);
        }
        db.ph_db
            .collection(ORDER_ITEMS_COL)
            .insert_many(docs, None)
            .await?;
        info!(
            "insert order item id:{} code:{} success",
            self.id, &self.item_code_ext
        );
        Ok(ids)
    }

    async fn delete_self(&self, db: &DbClient) -> Result<()> {
        let query = doc! {
          "id":self.id,
        };
        db.ph_db
            .collection::<MongoOrderItem>(ORDER_ITEMS_COL)
            .delete_one(query, None)
            .await?;
        Ok(())
    }
    /// if concealed item is shipped and its ship_date is not current month.
    /// this will return its ship_date and itself
    /// if concealed item is not shipped will return None
    #[async_recursion]
    #[instrument(name = "conceal order item self", skip(self, db))]
    async fn conceal(&mut self, db: &DbClient) -> Result<Option<()>> {
        info!(
            "try conceal order_item id:{} order_id:{}",
            self.id, self.order_id
        );
        match self.status {
            OrderItemStatus::BackOrdering => {
                info!("order_item is backordering skip inventory operation check");
                // update order
                update_order_update_at_by_id(db, self.order_id).await?;
                // update order item
                update_order_item_to_conceal_by_id(db, self.id).await?;
                Ok(None)
            }
            OrderItemStatus::Shipped => {
                info!("order item id:{} is shipped", self.id);
                self.restore_self_status_to_guaranteed(db).await?;
                self.conceal(db).await?;
                let shipment = get_shipment_by_id(db, self.shipment_id.unwrap()).await?;
                if self.location == InventoryLocation::JP {
                    info!("order_item's location is JP so publish new transfer");
                    info!(
                        "found item's shipment id:{shipment_id}",
                        shipment_id = shipment.id
                    );
                    let inventory = find_inventory_by_item_code_ext(db, &self.item_code_ext)
                        .await?
                        .unwrap();
                    //check if shipment vendor is clearance vendor the new location should be PCN
                    let new_location = if shipment.vendor.is_clearance_vendor() {
                        InventoryLocation::PCN
                    } else {
                        InventoryLocation::CN
                    };
                    let quantity = inventory
                        .quantity
                        .into_iter()
                        .map(|mut q| {
                            if q.location == new_location {
                                q.quantity += 1;
                            }
                            if q.location == InventoryLocation::JP {
                                q.quantity -= 1;
                            }
                            q
                        })
                        .collect::<Vec<_>>();
                    let items = vec![NewTransferInputItem {
                        item_code_ext: self.item_code_ext.clone(),
                        quantity,
                    }];
                    MongoTransferBuilder::new(
                        &shipment.shipment_no,
                        &format!("{}さん注文出荷後、キャンセル分", &self.customer_id),
                        shipment.shipment_date,
                        shipment.vendor,
                        &items,
                    )
                    .publish_new_transfer(db)
                    .await?;
                }
                Ok(Some(()))
            }
            OrderItemStatus::Guaranteed => {
                info!("order item is guaranteed");
                let order_operations = find_order_operations_by_id(db, self.order_id).await?;
                for operation in order_operations {
                    match operation.operation_type {
                        MongoOperationType::Ordered | MongoOperationType::CreateEmpty
                            if (operation.item_code_ext == self.item_code_ext)
                                && (operation.location == self.location) =>
                        {
                            info!(
                                "found match operation id:{} count:{} location:{:?} run backward ",
                                operation.id, operation.count, &operation.location
                            );
                            operation
                                .run_partial_backward(db, 1, MongoOperationType::ConcealOrderItem)
                                .await?;
                            // update order
                            update_order_update_at_by_id(db, self.order_id).await?;
                            // update order item
                            update_order_item_to_conceal_by_id(db, self.id).await?;
                        }
                        _ => (),
                    }
                }
                Ok(None)
            }
            OrderItemStatus::Concealed => Ok(None),
        }
    }
    /// Update a order item's status to shipped.
    #[instrument(name="update order item to shipped",skip(self,db),fields(
        id=%self.id,
        customer_id=%self.customer_id,
        item=%self.item_code_ext,
        location=?self.location,
    ))]
    async fn update_self_status_to_shipped(&self, db: &DbClient, shipment_id: Uuid) -> Result<()> {
        assert!(self.status == OrderItemStatus::Guaranteed);
        let now = Local::now();
        // update order item
        info!(
            "update order item id:{} status to shipped by {}",
            self.id, shipment_id
        );
        let query = doc! {
          "id":self.id,
        };
        let update = doc! {
          "$set":{
            "update_at":now,
            "status":OrderItemStatus::Shipped,
            "shipment_id":shipment_id,
          }
        };
        db.ph_db
            .collection::<MongoOrderItem>(ORDER_ITEMS_COL)
            .update_one(query, update, None)
            .await?;

        // update order
        let query = doc! {
          "id":self.order_id,
        };
        let update = doc! {
          "$set":{
            "update_at":now,
          }
        };
        info!("update order:{} update at", self.order_id);
        db.ph_db
            .collection::<MongoOrder>(ORDERS_COL)
            .update_one(query, update, None)
            .await?;
        info!("update order item:{} to shipped success", self.id);
        Ok(())
    }

    #[instrument(name="update order item to shipped with session",skip(self,db,session),fields(
        id=%self.id,
        customer_id=%self.customer_id,
        item=%self.item_code_ext,
        location=?self.location,
    ))]
    async fn update_self_status_to_shipped_with_session(
        &self,
        db: &DbClient,
        shipment_id: Uuid,
        session: &mut ClientSession,
    ) -> Result<()> {
        assert!(self.status == OrderItemStatus::Guaranteed);
        let now = Local::now();
        // update order item
        info!(
            "update order item id:{} status to shipped by {}",
            self.id, shipment_id
        );
        let query = doc! {
          "id":self.id,
        };
        let update = doc! {
          "$set":{
            "update_at":now,
            "status":OrderItemStatus::Shipped,
            "shipment_id":shipment_id,
          }
        };
        db.ph_db
            .collection::<MongoOrderItem>(ORDER_ITEMS_COL)
            .update_one_with_session(query, update, None, session)
            .await?;

        // update order
        let query = doc! {
          "id":self.order_id,
        };
        let update = doc! {
          "$set":{
            "update_at":now,
          }
        };
        info!("update order:{} update at", self.order_id);
        db.ph_db
            .collection::<MongoOrder>(ORDERS_COL)
            .update_one(query, update, None)
            .await?;
        info!("update order item:{} to shipped success", self.id);
        Ok(())
    }
    #[instrument(name="restore order item to guaranteed",skip(self,db),fields(
        id=%self.id,
        customer_id=%self.customer_id,
        item=%self.item_code_ext,
        location=?self.location,
    ))]
    pub async fn restore_self_status_to_guaranteed(&mut self, db: &DbClient) -> Result<()> {
        assert!(self.status == OrderItemStatus::Shipped);
        let now = Local::now();
        // update order item
        info!("restore order item id:{} status to  guaranteed", self.id);
        let query = doc! {
          "id":self.id,
        };
        let update = doc! {
          "$set":{
            "update_at":now,
            "status":OrderItemStatus::Guaranteed,
            "shipment_id":Bson::Null,
          }
        };
        db.ph_db
            .collection::<MongoOrderItem>(ORDER_ITEMS_COL)
            .update_one(query, update, None)
            .await?;

        // update order
        let query = doc! {
          "id":self.order_id,
        };
        let update = doc! {
          "$set":{
            "update_at":now,
          }
        };
        info!("update order:{} update at", self.order_id);
        db.ph_db
            .collection::<MongoOrder>(ORDERS_COL)
            .update_one(query, update, None)
            .await?;
        self.status = OrderItemStatus::Guaranteed;
        info!("restore order item:{} to guaranteed success", self.id);
        Ok(())
    }

    #[allow(dead_code)]
    async fn update_self_status_to_guaranteed(&self, db: &DbClient) -> Result<()> {
        assert!(self.status != OrderItemStatus::Guaranteed);
        assert!(self.status != OrderItemStatus::Shipped);
        let operation = MongoInventoryOperation::new(
            &self.item_code_ext,
            self.order_id,
            MongoOperationType::Ordered,
            -1,
            self.location.to_owned(),
        );
        let operation_id = operation.run_self(db, false).await?;
        let now = Local::now();
        // update order item
        info!(
            "update order item id:{} status to guaranteed by new register",
            self.id
        );
        let query = doc! {
          "id":self.id,
        };
        let update = doc! {
          "$set":{
            "update_at":now,
            "status":OrderItemStatus::Guaranteed,
          },
        };
        db.ph_db
            .collection::<MongoOrderItem>(ORDER_ITEMS_COL)
            .update_one(query, update, None)
            .await?;

        // update order
        let query = doc! {
          "id":self.order_id,
        };
        let update = doc! {
          "$set":{
            "update_at":now,
          },
          "$push":{
            "operation_ids":operation_id
          }
        };
        info!(
            "update order:{} update at, push operation id:{}",
            self.order_id, operation_id
        );
        db.ph_db
            .collection::<MongoOrder>(ORDERS_COL)
            .update_one(query, update, None)
            .await?;
        info!("update order item:{} to guaranteed success", self.id);
        Ok(())
    }

    async fn update_self_status_to_guaranteed_with_session(
        &self,
        db: &DbClient,
        session: &mut ClientSession,
    ) -> Result<()> {
        assert!(self.status != OrderItemStatus::Guaranteed);
        assert!(self.status != OrderItemStatus::Shipped);
        let operation = MongoInventoryOperation::new(
            &self.item_code_ext,
            self.order_id,
            MongoOperationType::Ordered,
            -1,
            self.location.to_owned(),
        );
        let operation_id = operation.run_self_with_session(db, false, session).await?;
        let now = Local::now();
        // update order item
        info!(
            "update order item id:{} status to guaranteed by new register",
            self.id
        );
        let query = doc! {
          "id":self.id,
        };
        let update = doc! {
          "$set":{
            "update_at":now,
            "status":OrderItemStatus::Guaranteed,
          },
        };
        db.ph_db
            .collection::<MongoOrderItem>(ORDER_ITEMS_COL)
            .update_one_with_session(query, update, None, session)
            .await?;

        // update order
        let query = doc! {
          "id":self.order_id,
        };
        let update = doc! {
          "$set":{
            "update_at":now,
          },
          "$push":{
            "operation_ids":operation_id
          }
        };
        info!(
            "update order:{} update at, push operation id:{}",
            self.order_id, operation_id
        );
        db.ph_db
            .collection::<MongoOrder>(ORDERS_COL)
            .update_one_with_session(query, update, None, session)
            .await?;
        info!("update order item:{} to guaranteed success", self.id);
        Ok(())
    }
}

#[derive(Deserialize, Serialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum OrderItemStatus {
    BackOrdering,
    Guaranteed,
    Shipped,
    Concealed,
}

impl From<OrderItemStatus> for Bson {
    fn from(o: OrderItemStatus) -> Self {
        match o {
            OrderItemStatus::BackOrdering => Bson::String(String::from("backordering")),
            OrderItemStatus::Guaranteed => Bson::String(String::from("guaranteed")),
            OrderItemStatus::Shipped => Bson::String(String::from("shipped")),
            OrderItemStatus::Concealed => Bson::String(String::from("concealed")),
        }
    }
}

#[derive(Deserialize, Debug, Clone)]
pub struct MongoOrderOutput {
    pub id: Uuid,
    pub created_at: mongodb::bson::DateTime,
    pub update_at: mongodb::bson::DateTime,
    pub order_datetime: mongodb::bson::DateTime,
    pub taobao_order_no: String,
    pub customer_id: String,
    pub note: String,
    pub items: Vec<MongoOrderItem>,
}

async fn find_order_operations_by_id(
    db: &DbClient,
    id: Uuid,
) -> Result<Vec<MongoInventoryOperation>> {
    let pipeline = vec![
        doc! {
          "$match":{
            "id":id,
          }
        },
        doc! {
          "$lookup":{
         "from":OPERATIONS_COL,
            "localField":"operation_ids",
            "foreignField":"id",
            "as":"operations",
          }
        },
    ];
    let mut cursor = db
        .ph_db
        .collection::<Document>(ORDERS_COL)
        .aggregate(pipeline, None)
        .await?;

    let mut outputs = Vec::new();
    while let Some(doc) = cursor.next().await {
        let output: Operations = bson::from_document(doc?)?;
        outputs.push(output);
    }
    Ok(outputs[0].operations.to_owned())
}

#[derive(Deserialize, Serialize, Debug, Clone)]
struct OrderId {
    id: Uuid,
}

pub const ITEMS_PER_PAGE: u32 = 10;

pub async fn query_orders(
    db: &DbClient,
    keyword: &str,
    status: &str,
    from: bson::DateTime,
    to: bson::DateTime,
    page: Option<u32>,
) -> Result<(bool, Vec<MongoOrderOutput>)> {
    let mut pipeline = vec![
        doc! {
          "$match":{
            "order_datetime":{
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

    if !keyword.is_empty() {
        pipeline.push(doc! {
          "$match":{
            "$or":[
              {"taobao_order_no":{
                          "$regex":keyword,
                          "$options":"i"
              }},
              {"customer_id":{
                          "$regex":keyword,
                          "$options":"i"
              }},
              {"items.item_code_ext":{
                          "$regex":keyword,
                          "$options":"i"
              }},
              {"note":{
                          "$regex":keyword,
                          "$options":"i"
              }}
            ]
          }
        })
    }

    if !status.is_empty() {
        let statuses: Vec<&str> = status.trim().split(',').collect();
        let mut or_doc = Vec::new();
        for status in statuses {
            or_doc.push(doc! {"items.status":status})
        }
        pipeline.push(doc! {
          "$match":
            {
              "$or":or_doc,
            },
        })
    }
    pipeline.push(doc! {
    "$sort":{
        "created_at":-1,
        "taobao_order_no":-1,
        "order_datetime":-1,
    }});
    let collation = Collation::builder()
        .locale("en_US")
        .numeric_ordering(true)
        .build();
    // page is none means this is a non-paged request.
    // we return full result.
    let option = AggregateOptions::builder().collation(collation).build();
    if page.is_none() {
        let mut cursor = db
            .ph_db
            .collection::<Document>(ORDERS_COL)
            .aggregate(pipeline, option)
            .await?;
        let mut outputs = Vec::new();
        while let Some(doc) = cursor.next().await {
            let output: MongoOrderOutput = bson::from_document(doc?)?;
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
        .collection::<Document>(ORDERS_COL)
        .aggregate(pipeline, None)
        .await?;
    let mut outputs = Vec::new();
    while let Some(doc) = cursor.next().await {
        let output: MongoOrderOutput = bson::from_document(doc?)?;
        outputs.push(output);
    }
    Ok(((outputs.len() as u32) == ITEMS_PER_PAGE, outputs))
}

pub async fn get_order_by_id(db: &DbClient, id: Uuid) -> Result<MongoOrderOutput> {
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
        .collection::<MongoOrderOutput>(ORDERS_COL)
        .aggregate(pipeline, None)
        .await?;
    let mut outputs = Vec::new();
    while let Some(doc) = cursor.next().await {
        let output: MongoOrderOutput = bson::from_document(doc?)?;
        outputs.push(output);
    }
    if outputs.is_empty() {
        return Err(Error::OrderNotFound(id.to_string()));
    }
    Ok(outputs[0].to_owned())
}

#[derive(Deserialize, Serialize, Debug, Clone)]
struct DeletePreOutput {
    operations: Vec<MongoInventoryOperation>,
    order_items: Vec<MongoOrderItem>,
}

#[instrument(name = "inner delete order", skip(db, id))]
pub async fn delete_order(db: &DbClient, id: Uuid) -> Result<DeleteOrderOutput> {
    let pipeline = vec![
        doc! {
          "$match":{
            "id":id,
          }
        },
        doc! {
          "$lookup":{
            "from":OPERATIONS_COL,
            "localField":"operation_ids",
            "foreignField":"id",
            "as":"operations",
          },
        },
        doc! {
          "$lookup":{
            "from":ORDER_ITEMS_COL,
            "localField":"order_item_ids",
            "foreignField":"id",
            "as":"order_items",
          }
        },
    ];
    let mut cursor = db
        .ph_db
        .collection::<Document>(ORDERS_COL)
        .aggregate(pipeline, None)
        .await?;

    let mut outputs = Vec::new();
    while let Some(doc) = cursor.next().await {
        let output: DeletePreOutput = bson::from_document(doc?)?;
        outputs.push(output);
    }
    let mut item_is_shipped_ids = vec![];
    for mut order_items in outputs[0].order_items.clone().into_iter() {
        if order_items.conceal(db).await?.is_some() {
            item_is_shipped_ids.push(order_items.id)
        }
    }
    let query = doc! {
      "id":id,
    };
    db.ph_db
        .collection::<MongoOrder>(ORDERS_COL)
        .delete_one(query, None)
        .await?;
    for item in outputs[0].order_items.iter() {
        item.delete_self(db).await?;
    }
    Ok(DeleteOrderOutput {
        deleted_items: outputs[0].order_items.clone(),
        item_is_shipped_ids,
    })
}

pub async fn find_order_item_by_id(db: &DbClient, id: Uuid) -> Result<MongoOrderItem> {
    let filter = doc! {
      "id":id,
    };
    db.ph_db
        .collection::<MongoOrderItem>(ORDER_ITEMS_COL)
        .find_one(filter, None)
        .await?
        .ok_or_else(|| Error::OrderItemNotFound(id.to_string()))
}

async fn update_order_update_at_by_id(db: &DbClient, id: Uuid) -> Result<()> {
    let query = doc! {
      "id":id,
    };
    let update = doc! {
      "$set":{
        "update_at":Local::now(),
      }
    };
    info!("update order id:{} update at", id);
    db.ph_db
        .collection::<MongoOrder>(ORDERS_COL)
        .update_one(query, update, None)
        .await?;
    Ok(())
}

/// find the order item by provided id, then update its status to shipped.
#[instrument(name = "update order item to shipped", skip(db))]
pub async fn update_order_item_status_to_shipped_by_id(
    db: &DbClient,
    id: Uuid,
    shipment_id: Uuid,
) -> Result<()> {
    let item = find_order_item_by_id(db, id).await?;
    item.update_self_status_to_shipped(db, shipment_id).await?;
    Ok(())
}
#[instrument(name = "update order item to shipped with session", skip(db, session))]
pub async fn update_order_item_status_to_shipped_by_id_with_session(
    db: &DbClient,
    id: Uuid,
    shipment_id: Uuid,
    session: &mut ClientSession,
) -> Result<()> {
    let item = find_order_item_by_id(db, id).await?;
    item.update_self_status_to_shipped_with_session(db, shipment_id, session)
        .await?;
    Ok(())
}

#[instrument(name = "update order item to conceal", skip(db))]
async fn update_order_item_to_conceal_by_id(db: &DbClient, id: Uuid) -> Result<()> {
    let query = doc! {
      "id":id,
    };
    let update = doc! {
      "$set":{
        "update_at":Local::now(),
        "status":OrderItemStatus::Concealed,
      },
    };
    info!("update order item id:{} status to conceal", id);
    db.ph_db
        .collection::<MongoOrderItem>(ORDER_ITEMS_COL)
        .update_one(query, update, None)
        .await?;
    Ok(())
}

#[async_recursion]
#[instrument(name = "inner conceal order item", skip(db, id))]
pub async fn conceal_order_item(db: &DbClient, id: Uuid) -> Result<ConcealItemOutput> {
    let mut order_item = find_order_item_by_id(db, id).await?;
    if order_item.conceal(db).await?.is_some() {
        return Ok(ConcealItemOutput {
            concealed_item: order_item,
            is_shipped: true,
        });
    }
    Ok(ConcealItemOutput {
        concealed_item: order_item,
        is_shipped: false,
    })
}

pub async fn update_order_note(db: &DbClient, id: Uuid, note: &str) -> Result<()> {
    let query = doc! {
      "id":id,
    };
    let update = doc! {
      "$set":{
        "note":note,
      }
    };
    //update order note
    db.ph_db
        .collection::<MongoOrder>(ORDERS_COL)
        .update_one(query, update, None)
        .await?;

    //update order item note
    let query = doc! {
      "order_id":id,
    };
    let update = doc! {
      "$set":{
        "note":note,
      }
    };
    db.ph_db
        .collection::<MongoOrderItem>(ORDER_ITEMS_COL)
        .update_many(query, update, None)
        .await?;
    Ok(())
}

pub async fn query_order_items(
    db: &DbClient,
    keyword: &str,
    status: &OrderItemStatus,
) -> Result<Vec<MongoOrderItem>> {
    let mut filter = doc! {
      "status":status,
    };
    if !keyword.is_empty() {
        let bson = bson! {
        [
                  {"item_code_ext":{
                      "$regex":keyword,
                      "$options":"i"
                  }},
                  {"customer_id":{
                      "$regex":keyword,
                      "$options":"i"
                  }},
                  {"customer_id":{
                      "$regex":keyword,
                      "$options":"i"
                  }},
                  {"note":{
                      "$regex":keyword,
                      "$options":"i"
                  }},
        ]
        };
        filter.insert("$or", bson);
    }
    let options = mongodb::options::FindOptions::builder()
        .sort(doc! {"order_datetime":1})
        .build();
    let mut cursor = db
        .ph_db
        .collection::<MongoOrderItem>(ORDER_ITEMS_COL)
        .find(filter, options)
        .await?;
    let mut outputs = Vec::new();
    while let Some(output) = cursor.next().await {
        outputs.push(output?)
    }
    Ok(outputs)
}

#[instrument(name = "find order items by code,status and location", skip(db))]
async fn find_order_items_by_code_status_location(
    db: &DbClient,
    item_code_ext: &str,
    status: &OrderItemStatus,
    location: &InventoryLocation,
) -> Result<Vec<MongoOrderItem>> {
    let filter = doc! {
      "item_code_ext":item_code_ext,
      "location":location,
      "status":status,
    };

    //should output order by order_datetime asc
    let options = mongodb::options::FindOptions::builder()
        .sort(doc! {"order_datetime":1})
        .build();

    let mut cursor = db
        .ph_db
        .collection::<MongoOrderItem>(ORDER_ITEMS_COL)
        .find(filter, options)
        .await?;
    let mut outputs = Vec::new();
    while let Some(output) = cursor.next().await {
        outputs.push(output?)
    }
    Ok(outputs)
}

#[instrument(
    name = "find order items by code,status and location",
    skip(db, session)
)]
async fn find_order_items_by_code_status_location_with_session(
    db: &DbClient,
    item_code_ext: &str,
    status: &OrderItemStatus,
    location: &InventoryLocation,
    session: &mut ClientSession,
) -> Result<Vec<MongoOrderItem>> {
    let filter = doc! {
      "item_code_ext":item_code_ext,
      "location":location,
      "status":status,
    };

    //should output order by order_datetime asc
    let options = mongodb::options::FindOptions::builder()
        .sort(doc! {"order_datetime":1})
        .build();

    let mut cursor = db
        .ph_db
        .collection::<MongoOrderItem>(ORDER_ITEMS_COL)
        .find_with_session(filter, options, session)
        .await?;
    let mut outputs = Vec::new();
    while let Some(output) = cursor.next(session).await {
        outputs.push(output?)
    }
    Ok(outputs)
}

#[instrument(name = "inner check then update order status", skip(db, items))]
pub async fn check_then_update_order_status(
    db: &DbClient,
    items: Vec<RegisterItem>,
) -> Result<Vec<MongoOrderItem>> {
    let mut session = db.client.start_session(None).await?;
    let options = TransactionOptions::builder()
        .read_concern(ReadConcern::majority())
        .write_concern(WriteConcern::builder().w(Acknowledgment::Majority).build())
        .build();
    session.start_transaction(options).await?;
    let mut res_items = Vec::new();
    for input_item in items {
        while let Err(error) = check_then_update_item_with_session(
            db,
            &mut res_items,
            &input_item.item_code_ext,
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
    loop {
        if let Err(ref error) = session.commit_transaction().await {
            if error.contains_label(UNKNOWN_TRANSACTION_COMMIT_RESULT) {
                continue;
            }
        }
        break;
    }
    Ok(res_items)
}

#[instrument(name = "check then update item sequentially", skip(db, res_items))]
async fn check_then_update_item(
    db: &DbClient,
    res_items: &mut Vec<MongoOrderItem>,
    item_code_ext: &str,
) -> Result<()> {
    info!("check item_code_ext:{item_code_ext}");
    let inventory = find_inventory_by_item_code_ext(db, item_code_ext)
        .await?
        .unwrap();
    //check inventory by locations
    for in_stock in inventory
        .quantity
        .iter()
        .filter(|in_stock| in_stock.quantity != 0)
    {
        let back_ordering_order_items = find_order_items_by_code_status_location(
            db,
            item_code_ext,
            &OrderItemStatus::BackOrdering,
            &in_stock.location,
        )
        .await?;
        if back_ordering_order_items.is_empty() {
            continue;
        }
        for index in 0..in_stock.quantity {
            info!(
                "update item no {} id:{} to guaranteed",
                index, back_ordering_order_items[index as usize].id
            );
            back_ordering_order_items[index as usize]
                .update_self_status_to_guaranteed(db)
                .await?;
            res_items.push(back_ordering_order_items[index as usize].clone());
            if index as usize == back_ordering_order_items.len() - 1 {
                break;
            }
        }
    }
    Ok(())
}

#[instrument(
    name = "check then update item sequentially",
    skip(db, res_items, session)
)]
async fn check_then_update_item_with_session(
    db: &DbClient,
    res_items: &mut Vec<MongoOrderItem>,
    item_code_ext: &str,
    session: &mut ClientSession,
) -> Result<()> {
    info!("check item_code_ext:{item_code_ext}");
    let inventory = find_inventory_by_item_code_ext_with_session(db, item_code_ext, session)
        .await?
        .unwrap();
    //check inventory by locations
    for in_stock in inventory
        .quantity
        .iter()
        .filter(|in_stock| in_stock.quantity != 0)
    {
        let back_ordering_order_items = find_order_items_by_code_status_location_with_session(
            db,
            item_code_ext,
            &OrderItemStatus::BackOrdering,
            &in_stock.location,
            session,
        )
        .await?;
        if back_ordering_order_items.is_empty() {
            continue;
        }
        for index in 0..in_stock.quantity {
            info!(
                "update item no {} id:{} to guaranteed",
                index, back_ordering_order_items[index as usize].id
            );
            back_ordering_order_items[index as usize]
                .update_self_status_to_guaranteed_with_session(db, session)
                .await?;
            res_items.push(back_ordering_order_items[index as usize].clone());
            if index as usize == back_ordering_order_items.len() - 1 {
                break;
            }
        }
    }
    Ok(())
}
use domain::OrderItemRate;
#[instrument(name = "update order item rate inner", skip(db, id, rate))]
async fn update_order_item_rate(db: &DbClient, id: Uuid, rate: OrderItemRate) -> Result<()> {
    info!("update order item {id} rate to {}", rate.get_inner());
    let query = doc! {
      "id":id,
    };
    let update = doc! {
      "$set":{
        "rate":rate.get_inner(),
      }
    };

    db.ph_db
        .collection::<MongoOrder>(ORDER_ITEMS_COL)
        .update_one(query, update, None)
        .await?;

    info!("update order item rate success");
    Ok(())
}

pub use domain::OrderValidateError;
mod domain {
    use chrono::NaiveDateTime;
    use thiserror::Error;

    #[derive(Debug, Error)]
    pub enum OrderValidateError {
        #[error("Invalid Taobao order no digit")]
        TaobaoOrderNoDigit,
        #[error("Invalid Taobao order no datetime")]
        TaobaoOrderNoDatetime,
        #[error("Invalid Taobao order no not number")]
        TaobaoOrderNoNotNumber,

        #[error("order item rate out of range")]
        OrderItemRateOutOfRange,
    }
    /// aka 支付宝交易号
    /// parse requirement:
    /// 1. 28 digits
    /// 2. numeric
    /// 3. first 8 digits is formatted YYYYmmdd
    pub struct TaobaoOrderNo(String);

    impl TaobaoOrderNo {
        pub fn parse(input: &str) -> Result<Self, OrderValidateError> {
            let input = input.trim();
            if !input.chars().all(|c| c.is_numeric()) {
                return Err(OrderValidateError::TaobaoOrderNoNotNumber);
            }
            if input.len() != 28 {
                return Err(OrderValidateError::TaobaoOrderNoDigit);
            }
            if NaiveDateTime::parse_from_str(
                &format!("{} 00:00:00", &input[0..8]),
                "%Y%m%d %H:%M:%S",
            )
            .is_err()
            {
                return Err(OrderValidateError::TaobaoOrderNoDatetime);
            }
            Ok(TaobaoOrderNo(String::from(input)))
        }

        pub fn get_inner(self) -> String {
            self.0
        }
    }

    pub struct OrderItemRate(f64);

    impl OrderItemRate {
        pub fn parse(input: f64) -> Result<Self, OrderValidateError> {
            if input <= 0.0 || input > 1.0 {
                return Err(OrderValidateError::OrderItemRateOutOfRange);
            }
            Ok(OrderItemRate(input))
        }

        pub fn get_inner(&self) -> f64 {
            self.0
        }
    }
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OutdatedOrder {
    #[serde(with = "ts_seconds")]
    created_date: DateTime<Utc>,
    item_code: String,
    customer_id: String,
}
