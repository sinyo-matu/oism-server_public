use std::sync::Arc;

use crate::{
    db::{mongo::DbClient, InventoryRepo},
    error_result::Result,
};
use axum::{
    extract::{Path, Query, State},
    routing::get,
    Json, Router,
};
use serde::Deserialize;

use crate::db::{inventory::Quantity, InventoryOperation, InventoryOutput};

use super::{export::export_jp_inventory, AppState, PagedResponse};

pub fn get_inventory_router() -> Router<AppState> {
    Router::new()
        .route("/", get(query_inventory))
        .route(
            "/operations/:item_code_ext",
            get(get_inventory_item_operations),
        )
        .route(
            "/quantity/:item_code_ext",
            get(get_inventory_quantity_by_item_code_ext),
        )
        .route("/export", get(export_jp_inventory))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InventoryQuery {
    pub keyword: String,
    pub category: Option<Category>,
    pub show_zero_quantity: bool,
    // because the unsupported of array query, this is this a workaround.
    // string like "jp,cn" will parsed into ["jp","cn"]
    pub location: Option<String>,
    pub page: Option<u32>,
}

pub async fn query_inventory(
    Query(query): Query<InventoryQuery>,
    State(db): State<Arc<DbClient>>,
) -> Result<Json<PagedResponse<InventoryOutput>>> {
    let current_page = query.page.unwrap_or(0);
    let (has_next, items) = db.query_inventory(query).await?;
    let res = PagedResponse {
        data: items.into_iter().map(|i| i.into()).collect::<Vec<_>>(),
        has_next,
        next: current_page + 1,
    };
    Ok(res.into())
}

pub async fn get_inventory_item_operations(
    Path(item_code_ext): Path<String>,
    State(db): State<Arc<DbClient>>,
) -> Result<Json<Vec<InventoryOperation>>> {
    let res = db.get_inventory_item_operations(&item_code_ext).await?;
    Ok(res.into_iter().map(|o| o.into()).collect::<Vec<_>>().into())
}

pub async fn get_inventory_quantity_by_item_code_ext(
    Path(item_code_ext): Path<String>,
    State(db): State<Arc<DbClient>>,
) -> Result<Json<Vec<Quantity>>> {
    let res = db.find_inventory_by_item_code_ext(&item_code_ext).await?;
    match res {
        Some(i) => Ok(i.quantity.into()),
        None => Ok(vec![].into()),
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Category {
    Top,
    Skirt,
    OnePiece,
    Outer,
    Pants,
    Bag,
    Shoes,
    Accessory,
    Hat,
    Others,
}

impl Category {
    pub fn to_concrete_content(&self) -> String {
        match self {
        Self::Top => String::from("トップス"),
        Self::Skirt => String::from("スカート"),
        Self::OnePiece=> String::from("ワンピース"),
        Self::Outer  => String::from("ジャケット/アウター"),
        Self::Pants  => String::from("パンツ"),
        Self::Bag    => String::from("バッグ"),
        Self::Shoes  => String::from("シューズ"),
        Self::Accessory => String::from("アクセサリー ヘアアクセサリー"),
        Self::Hat    => String::from("帽子"),
        Self::Others => String::from("レッグウェア その他 ファッション雑貨 財布/小物 ルームウェア インテリア 雑貨/ホビー PCスマホグッズ/家電"),
       }
    }
}
