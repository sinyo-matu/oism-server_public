use super::{
    auth::{self, User},
    PhDataBase, PhItem, SMTAuthDataBase,
};
use crate::error_result::Result;
use axum::async_trait;
use mongodb::bson::Uuid;
use mongodb::{bson::doc, options::ClientOptions, Client, Database};
use tracing::info;

pub const INVENTORY_COL: &str = "inventory";
pub const REGISTERS_COL: &str = "registers";
pub const OPERATIONS_COL: &str = "operations";
pub const ORDER_ITEMS_COL: &str = "order_items";
pub const ORDERS_COL: &str = "orders";
pub const SHIPMENT_COL: &str = "shipments";
pub const RETURNS_COL: &str = "returns";
pub const TRANSFERS_COL: &str = "transfers";
pub const ITEMS_COL: &str = "items";
pub const USERS_COL: &str = "users";

#[derive(Clone, Debug)]
pub struct DbClient {
    pub client: Client,
    pub ph_db: Database,
}

impl DbClient {
    pub async fn init(connect_string: &str, database_name: &str) -> Result<Self> {
        let mut client_options = ClientOptions::parse(connect_string).await?;
        client_options.app_name = Some(String::from("pinkhouse"));
        let client = Client::with_options(client_options)?;
        client.list_database_names(None, None).await?;
        let database = client.database(database_name);
        info!("db started successfully");
        Ok(Self {
            client,
            ph_db: database,
        })
    }
}

#[async_trait]
impl PhDataBase for DbClient {
    async fn find_one_by_item_code(&self, item_code: &str) -> Result<Option<PhItem>> {
        let query = doc! {
            "code":item_code
        };
        let item_op = self
            .ph_db
            .collection::<PhItem>(ITEMS_COL)
            .find_one(query, None)
            .await?;
        Ok(item_op)
    }
}

#[async_trait]
impl SMTAuthDataBase for DbClient {
    async fn check_is_username_occupied(&self, username: &str) -> Result<bool> {
        Ok(auth::check_is_username_occupied(self, username).await?)
    }

    async fn create_user(&self, user: User) -> Result<()> {
        Ok(auth::create_user(self, user).await?)
    }

    async fn find_user(&self, id: Uuid) -> Result<User> {
        Ok(auth::find_user(self, id).await?)
    }

    async fn find_user_by_username(&self, username: &str) -> Result<User> {
        Ok(auth::find_user_by_username(self, username).await?)
    }
}
