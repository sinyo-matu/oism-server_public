pub mod auth;
pub mod invenope;
pub mod inventory;
pub mod mongo;
pub mod order;
pub mod register;
pub mod retrn;
pub mod shipment;
pub mod transfer;

use crate::{
    error_result::Result,
    server::{
        inventory::InventoryQuery, retrn::NewReturnInputItem, transfer::NewTransferInputItem,
        NewShipmentInput, OrderRegisterInput,
    },
};
use axum::async_trait;
use chrono::{serde::ts_seconds, DateTime, Utc};
use mongodb::{
    bson::doc,
    bson::{oid::ObjectId, Bson, Uuid},
};
use serde::{Deserialize, Serialize};

use self::{
    auth::User,
    invenope::{MongoInventoryOperation, MongoOperationType},
    inventory::{InventoryLocation, MongoInventoryItem, MongoInventoryOutput, Quantity},
    mongo::{DbClient, ITEMS_COL},
    order::{
        ConcealItemOutput, DeleteOrderOutput, MongoOrderItem, MongoOrderOutput, OrderItemStatus,
    },
    register::{MongoRegisterItem, MongoRegisterOutput},
    retrn::{MongoReturnItem, MongoReturnOutput},
    shipment::{MongoShipment, MongoShipmentOutput, ShipmentStatus, ShipmentVendor},
    transfer::{MongoTransfer, MongoTransferOutput},
};

#[async_trait]
pub trait PhDataBase: Send + Sync + 'static {
    async fn find_one_by_item_code(&self, code: &str) -> Result<Option<PhItem>>;
}

#[async_trait]
pub trait RegisterRepo: Send + Sync + 'static {
    async fn insert_stock_register(&self, register: &StockRegisterInput) -> Result<()>;

    async fn delete_stock_register(&self, register_id: Uuid) -> Result<String>;

    async fn find_register_by_no(&self, no: &str) -> Result<Vec<MongoRegisterOutput>>;

    async fn query_registers(
        &self,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
        keyword: Option<String>,
        page: Option<u32>,
    ) -> Result<(bool, Vec<MongoRegisterOutput>)>;

    async fn get_register_by_id(&self, id: Uuid) -> Result<MongoRegisterOutput>;
}
#[async_trait]
pub trait InventoryRepo: Send + Sync + 'static {
    async fn query_inventory(
        &self,
        query: InventoryQuery,
    ) -> Result<(bool, Vec<MongoInventoryOutput>)>;

    async fn get_inventory_item_operations(
        &self,
        item_code_ext: &str,
    ) -> Result<Vec<MongoInventoryOperation>>;

    async fn find_inventory_by_item_code_ext(
        &self,
        item_code_ext: &str,
    ) -> Result<Option<MongoInventoryItem>>;
}

#[async_trait]
pub trait OrderRepo: Send + Sync + 'static {
    async fn create_order(&self, input: OrderRegisterInput) -> Result<()>;

    async fn query_orders(
        &self,
        keyword: &str,
        status: &str,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
        page: Option<u32>,
    ) -> Result<(bool, Vec<MongoOrderOutput>)>;

    /// why need this? frond end will load order first then load its order items.
    /// because order items need be update their state independently.
    async fn get_order_by_id(&self, id: Uuid) -> Result<MongoOrderOutput>;

    async fn get_order_by_taobao_no(&self, taobao_order_no: &str) -> Result<Vec<MongoOrderOutput>>;
    /// delete an order. if its related order items is guaranteed order item.
    /// this will release guaranteed inventory.
    /// and delete the order items too.
    /// will return deleted order items' ids.
    async fn delete_order(&self, order_id: Uuid) -> Result<DeleteOrderOutput>;

    /// conceal an order item in an order,if its a guaranteed order item.
    /// this will release guaranteed inventory.
    /// and update order order item's status to concealed,update order item's update_at field.
    async fn conceal_order_item(&self, order_item_id: Uuid) -> Result<ConcealItemOutput>;

    async fn get_order_item_by_id(&self, order_item_id: Uuid) -> Result<MongoOrderItem>;

    /// update the order's note and this will update order's
    /// related order items' note as well.
    /// and update their update_at field.
    async fn update_order_note(&self, order_id: Uuid, note: &str) -> Result<()>;

    /// query order items with guaranteed status
    async fn query_order_items(
        &self,
        keyword: &str,
        status: &OrderItemStatus,
    ) -> Result<Vec<MongoOrderItem>>;

    /// check order items which matched the from input items' item_code_ext,
    /// if their status is backordering then change its status to guarantee.
    /// then update inventory.
    async fn check_then_update_order_status(
        &self,
        items: Vec<RegisterItem>,
    ) -> Result<Vec<MongoOrderItem>>;

    async fn update_order_item_rate(&self, id: Uuid, rate: f64) -> Result<()>;
}

#[async_trait]
pub trait ShipmentRepo: Send + Sync + 'static {
    /// create a new shipment, then update its related order item's status to shipped.
    /// and update related order item and order's update_at field.
    async fn create_new_shipment(&self, input: NewShipmentInput) -> Result<()>;

    /// query shipments will return shipment ids
    async fn query_shipments(
        &self,
        keyword: &str,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
        status: &str,
        vendor: &str,
        page: Option<u32>,
    ) -> Result<(bool, Vec<MongoShipmentOutput>)>;

    async fn get_shipment_by_id(&self, id: Uuid) -> Result<MongoShipmentOutput>;

    async fn delete_shipment(&self, shipment_id: Uuid) -> Result<Vec<Uuid>>;

    async fn update_shipment_note(&self, shipment_id: Uuid, note: &str) -> Result<()>;

    async fn find_shipment_by_no(&self, shipment_no: &str) -> Result<Vec<MongoShipment>>;

    async fn find_shipments_by_no(&self, shipment_no: &str) -> Result<Vec<MongoShipmentOutput>>;

    async fn update_shipment_status(&self, shipment_id: Uuid, status: &str) -> Result<()>;

    async fn update_shipment_no(
        &self,
        current_shipment_no: &str,
        new_shipment_no: &str,
    ) -> Result<()>;

    async fn update_shipment_no_by_id(
        &self,
        shipment_id: Uuid,
        new_shipment_no: &str,
    ) -> Result<()>;
    async fn update_shipment_vendor(
        &self,
        shipment_id: Uuid,
        new_vendor: ShipmentVendor,
    ) -> Result<()>;
}

#[async_trait]
pub trait TransferRepo: Send + Sync + 'static {
    async fn create_new_transfer(
        &self,
        shipment_no: &str,
        note: &str,
        transfer_date: DateTime<Utc>,
        shipment_vendor: ShipmentVendor,
        items: Vec<NewTransferInputItem>,
    ) -> Result<()>;

    async fn find_transfer_by_id(&self, id: Uuid) -> Result<MongoTransferOutput>;
    async fn find_shipment_by_transfer_id(&self, id: Uuid) -> Result<Vec<MongoShipment>>;
    async fn query_transfers(
        &self,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
        keyword: Option<String>,
    ) -> Result<Vec<MongoTransferOutput>>;

    async fn delete_transfer_by_id(&self, id: Uuid) -> Result<()>;

    async fn find_transfer_by_shipment_id(
        &self,
        shipment_id: Uuid,
    ) -> Result<Option<Vec<MongoTransferOutput>>>;

    async fn find_transfer_by_shipment_no(
        &self,
        shipment_no: &str,
    ) -> Result<Vec<MongoTransferOutput>>;

    async fn find_mongo_transfer_by_shipment_no(
        &self,
        shipment_no: &str,
    ) -> Result<Vec<MongoTransfer>>;

    async fn update_transfers_shipment_no(
        &self,
        current_shipment_no: &str,
        new_shipment_no: &str,
    ) -> Result<()>;

    async fn update_transfer_shipment_no_by_id(
        &self,
        transfer_id: Uuid,
        new_shipment_no: &str,
    ) -> Result<()>;

    async fn update_transfers_vendor_by_shipment_no(
        &self,
        shipment_no: &str,
        new_vender: ShipmentVendor,
    ) -> Result<()>;

    async fn check_operations_backward_safety_by_transfer_id(
        &self,
        transfer_id: Uuid,
    ) -> Result<()>;
    async fn update_transfer_vendor_and_operations_by_transfer_id(
        &self,
        transfer_id: Uuid,
        new_vender: ShipmentVendor,
        new_location: InventoryLocation,
    ) -> Result<()>;
}

#[async_trait]
pub trait ReturnRepo: Send + Sync + 'static {
    async fn create_new_return(
        &self,
        return_no: &str,
        return_date: DateTime<Utc>,
        note: &str,
        items: Vec<NewReturnInputItem>,
    ) -> Result<()>;

    async fn query_returns(
        &self,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
        keyword: Option<String>,
    ) -> Result<Vec<MongoReturnOutput>>;

    async fn get_return_by_id(&self, id: Uuid) -> Result<MongoReturnOutput>;

    async fn delete_return_by_id(&self, id: Uuid) -> Result<()>;
}
#[async_trait]
pub trait SMTAuthDataBase: Send + Sync + 'static {
    async fn check_is_username_occupied(&self, username: &str) -> Result<bool>;

    async fn create_user(&self, user: User) -> Result<()>;

    async fn find_user(&self, id: Uuid) -> Result<User>;

    async fn find_user_by_username(&self, username: &str) -> Result<User>;
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ItemSize {
    pub size_table: Option<SizeTable>,
    pub size_description: Option<String>,
    pub size_zh: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct SizeTable {
    pub head: Vec<String>,
    pub body: Vec<Vec<String>>,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct PhItem {
    pub _id: ObjectId,
    pub code: String,
    pub category: Vec<String>,
    pub item_name: Option<String>,
    pub made_in: Option<String>,
    pub label: String,
    pub price: u32,
    pub url: String,
    pub piccount: u32,
    pub bucket: String,
    pub material: String,
    pub created_at: Option<mongodb::bson::DateTime>,
    pub update_at: Option<mongodb::bson::DateTime>,
    pub size: Option<ItemSize>,
    pub item_name_zh: Option<String>,
    pub is_published: bool,
}

impl PhItem {
    pub fn new_dummy(item_code_ext: &str, price: u32) -> Self {
        Self {
            _id: ObjectId::new(),
            code: item_code_ext[0..11].to_string(),
            category: vec![String::from("")],
            item_name: None,
            made_in: None,
            label: String::from(""),
            price,
            url: String::from(""),
            piccount: 0,
            bucket: String::from(""),
            material: String::from(""),
            created_at: None,
            update_at: None,
            size: None,
            item_name_zh: None,
            is_published: false,
        }
    }

    pub async fn insert_self(&self, db: &DbClient) -> Result<()> {
        let doc = doc! {
          "_id":self._id,
          "code":&self.code,
          "category":&self.category,
          "item_name":&self.item_name,
          "made_in":&self.made_in,
          "label":&self.label,
          "price":self.price,
          "url":&self.url,
          "piccount":self.piccount,
          "bucket":&self.bucket,
          "material":&self.material,
          "create_at":self.created_at,
          "update_at":self.update_at,
          "size":Bson::Null,
          "item_name_zh":&self.item_name_zh,
          "is_published":self.is_published,
        };

        db.ph_db.collection(ITEMS_COL).insert_one(doc, None).await?;
        Ok(())
    }

    pub fn get_discounted_price(&self, discount_rate: f64) -> u32 {
        let float_num = (self.price as f64) * discount_rate;

        if (float_num - float_num.trunc()) < 0.5 {
            float_num.round() as u32
        } else {
            float_num.ceil() as u32
        }
    }
}

impl From<PhItem> for ReplyPhItem {
    fn from(ph_item: PhItem) -> Self {
        ReplyPhItem {
            code: ph_item.code,
            category: ph_item.category,
            item_name: ph_item.item_name,
            label: ph_item.label,
            made_in: ph_item.made_in,
            price: ph_item.price,
            url: ph_item.url,
            piccount: ph_item.piccount,
            material: ph_item.material,
            bucket: ph_item.bucket,
            created_at: ph_item.created_at.map(|c| c.to_chrono()),
            update_at: ph_item.update_at.map(|u| u.to_chrono()),
            size: ph_item.size,
            item_name_zh: ph_item.item_name_zh,
        }
    }
}

#[derive(Deserialize, Serialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ReplyPhItem {
    pub code: String,
    pub category: Vec<String>,
    pub item_name: Option<String>,
    pub made_in: Option<String>,
    pub price: u32,
    pub label: String,
    pub url: String,
    pub piccount: u32,
    pub bucket: String,
    pub material: String,
    pub created_at: Option<DateTime<Utc>>,
    pub update_at: Option<DateTime<Utc>>,
    pub size: Option<ItemSize>,
    pub item_name_zh: Option<String>,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Register {
    pub id: Uuid,
    #[serde(with = "ts_seconds")]
    pub arrival_date: DateTime<Utc>,
    #[serde(with = "ts_seconds")]
    pub created_at: DateTime<Utc>,
    pub no: String,
    pub items: Vec<RegisterItem>,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct StockRegisterInput {
    #[serde(with = "ts_seconds")]
    pub arrival_date: DateTime<Utc>,
    pub no: String,
    pub items: Vec<RegisterItemInput>,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct RegisterItem {
    pub item_code_ext: String,
    pub count: u32,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct RegisterItemInput {
    pub item_code_ext: String,
    pub count: u32,
    pub price: u32,
    pub is_manual: bool,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct InventoryItem {
    pub item_code_ext: String,
    pub quantity: Vec<Quantity>,
    #[serde(with = "ts_seconds")]
    pub update_at: DateTime<Utc>,
    pub operation_ids: Vec<Uuid>,
}

impl From<MongoInventoryItem> for InventoryItem {
    fn from(m: MongoInventoryItem) -> Self {
        InventoryItem {
            item_code_ext: m.item_code_ext,
            quantity: m.quantity,
            update_at: m.update_at.to_chrono(),
            operation_ids: m.operation_ids,
        }
    }
}

#[derive(Deserialize, Serialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct InventoryOutput {
    pub item_code_ext: String,
    pub quantity: Vec<Quantity>,
    #[serde(with = "ts_seconds")]
    pub update_at: DateTime<Utc>,
    pub operation_ids: Vec<Uuid>,
}

impl From<MongoInventoryOutput> for InventoryOutput {
    fn from(m: MongoInventoryOutput) -> Self {
        Self {
            item_code_ext: m.item_code_ext,
            quantity: m.quantity,
            update_at: m.update_at.to_chrono(),
            operation_ids: m.operation_ids,
        }
    }
}

#[derive(Deserialize, Serialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct InventoryOperation {
    pub id: Uuid,
    pub item_code_ext: String,
    #[serde(with = "ts_seconds")]
    pub time: DateTime<Utc>,
    pub related_id: Uuid,
    pub operation_type: OperationType,
    pub count: i32,
    pub location: InventoryLocation,
}

impl From<MongoInventoryOperation> for InventoryOperation {
    fn from(m: MongoInventoryOperation) -> Self {
        InventoryOperation {
            id: m.id,
            item_code_ext: m.item_code_ext,
            time: m.time.to_chrono(),
            related_id: m.related_id,
            operation_type: m.operation_type.into(),
            count: m.count,
            location: m.location,
        }
    }
}

#[derive(Deserialize, Serialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
#[serde(tag = "type")]
pub enum OperationType {
    CreateEmpty,
    Arrival,
    Returned,
    DeleteRegister,
    DeleteOrder,
    DeleteReturn,
    DeleteTransfer,
    UpdateTransfer,
    ConcealOrderItem,
    Ordered,
    Move,
}

impl From<MongoOperationType> for OperationType {
    fn from(m: MongoOperationType) -> Self {
        match m {
            MongoOperationType::CreateEmpty => OperationType::CreateEmpty,
            MongoOperationType::Arrival => OperationType::Arrival,
            MongoOperationType::Returned => OperationType::Returned,
            MongoOperationType::DeleteOrder => OperationType::DeleteOrder,
            MongoOperationType::DeleteRegister => OperationType::DeleteRegister,
            MongoOperationType::DeleteReturn => OperationType::DeleteReturn,
            MongoOperationType::DeleteTransfer => OperationType::DeleteTransfer,
            MongoOperationType::UpdateTransfer => OperationType::UpdateTransfer,
            MongoOperationType::ConcealOrderItem => OperationType::ConcealOrderItem,
            MongoOperationType::Ordered => OperationType::Ordered,
            MongoOperationType::Move => OperationType::Move,
        }
    }
}

impl From<MongoRegisterOutput> for Register {
    fn from(m: MongoRegisterOutput) -> Self {
        Register {
            id: m.id,
            created_at: m.created_at.to_chrono(),
            arrival_date: m.arrival_date.to_chrono(),
            no: m.no,
            items: m.items.into_iter().map(|i| i.into()).collect::<Vec<_>>(),
        }
    }
}

impl From<MongoRegisterItem> for RegisterItem {
    fn from(m: MongoRegisterItem) -> Self {
        Self {
            item_code_ext: m.item_code_ext,
            count: m.count,
        }
    }
}

#[derive(Deserialize, Serialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Order {
    pub id: Uuid,
    #[serde(with = "ts_seconds")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "ts_seconds")]
    pub update_at: DateTime<Utc>,
    #[serde(with = "ts_seconds")]
    pub order_datetime: DateTime<Utc>,
    pub taobao_order_no: String,
    pub customer_id: String,
    pub note: String,
    pub items: Vec<OrderItem>,
}

impl From<MongoOrderOutput> for Order {
    fn from(m: MongoOrderOutput) -> Self {
        Self {
            id: m.id,
            created_at: m.created_at.to_chrono(),
            update_at: m.update_at.to_chrono(),
            order_datetime: m.order_datetime.to_chrono(),
            taobao_order_no: m.taobao_order_no,
            customer_id: m.customer_id,
            note: m.note,
            items: m.items.into_iter().map(|i| i.into()).collect::<Vec<_>>(),
        }
    }
}

/// order item object used in interacting with front end
#[derive(Deserialize, Serialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct OrderItem {
    pub id: Uuid,
    #[serde(with = "ts_seconds")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "ts_seconds")]
    pub update_at: DateTime<Utc>,
    #[serde(with = "ts_seconds")]
    pub order_datetime: DateTime<Utc>,
    pub item_code_ext: String,
    pub customer_id: String,
    pub rate: f64,
    pub location: InventoryLocation,
    pub status: OrderItemStatus,
    pub order_id: Uuid,
    pub note: String,
    pub shipment_id: Option<Uuid>,
}

impl From<MongoOrderItem> for OrderItem {
    fn from(m: MongoOrderItem) -> Self {
        Self {
            id: m.id,
            created_at: m.created_at.to_chrono(),
            update_at: m.update_at.to_chrono(),
            order_datetime: m.order_datetime.to_chrono(),
            item_code_ext: m.item_code_ext,
            customer_id: m.customer_id,
            rate: m.rate,
            location: m.location,
            status: m.status,
            order_id: m.order_id,
            note: m.note,
            shipment_id: m.shipment_id,
        }
    }
}

/// shipment object used in interacting with frond end.
#[derive(Serialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Shipment {
    id: Uuid,
    #[serde(with = "ts_seconds")]
    created_at: DateTime<Utc>,
    #[serde(with = "ts_seconds")]
    update_at: DateTime<Utc>,
    shipment_no: String,
    note: String,
    vendor: ShipmentVendor,
    #[serde(with = "ts_seconds")]
    shipment_date: DateTime<Utc>,
    items: Vec<OrderItem>,
    status: ShipmentStatus,
}

impl From<MongoShipmentOutput> for Shipment {
    fn from(m: MongoShipmentOutput) -> Self {
        Self {
            id: m.id,
            created_at: m.created_at.to_chrono(),
            update_at: m.update_at.to_chrono(),
            shipment_no: m.shipment_no,
            note: m.note,
            vendor: m.vendor,
            shipment_date: m.shipment_date.to_chrono(),
            items: m.items.into_iter().map(|i| i.into()).collect::<Vec<_>>(),
            status: m.status,
        }
    }
}

#[derive(Deserialize, Serialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Return {
    pub id: Uuid,
    #[serde(with = "ts_seconds")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "ts_seconds")]
    pub update_at: DateTime<Utc>,
    pub return_no: String,
    #[serde(with = "ts_seconds")]
    pub return_date: DateTime<Utc>,
    pub note: String,
    pub items: Vec<ReturnItem>,
}

impl From<MongoReturnOutput> for Return {
    fn from(m: MongoReturnOutput) -> Self {
        Self {
            id: m.id,
            created_at: m.created_at.to_chrono(),
            update_at: m.update_at.to_chrono(),
            return_no: m.return_no,
            return_date: m.return_date.to_chrono(),
            note: m.note,
            items: m.items.into_iter().map(|i| i.into()).collect(),
        }
    }
}

#[derive(Deserialize, Serialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ReturnItem {
    pub item_code_ext: String,
    pub count: u32,
}

impl From<MongoReturnItem> for ReturnItem {
    fn from(m: MongoReturnItem) -> Self {
        Self {
            item_code_ext: m.item_code_ext,
            count: m.count.unsigned_abs(),
        }
    }
}
