use std::{collections::HashMap, sync::Arc};

use axum::{
    extract::{Path, Query, State},
    response::IntoResponse,
    Json,
};
use chrono::prelude::*;
use serde::{Deserialize, Serialize};
use tracing::{debug, instrument, warn};
use uuid::Uuid;

use crate::db::{
    inventory::InventoryLocation, mongo::DbClient, InventoryRepo, PhDataBase, ShipmentRepo,
};
use crate::{
    db::{order::OrderItemStatus, PhItem, TransferRepo},
    error_result::{validate_http_response, Result},
    server::auth::SETTINGS,
};

use super::{inventory::InventoryQuery, shipment::QueryShipmentMessage};

#[derive(Serialize)]
pub struct ExportQueryShipmentMessage {
    filename: String,
    rows: Vec<Vec<String>>,
}

#[instrument(name="export shipments",skip(message,db,http_client),fields(
    request_id = %Uuid::new_v4()
))]
pub async fn export_shipments(
    Query(message): Query<QueryShipmentMessage>,
    State(db): State<Arc<DbClient>>,
    State(http_client): State<Arc<reqwest::Client>>,
) -> Result<impl IntoResponse> {
    let mut items_map: HashMap<(String, String), usize> = std::collections::HashMap::new();
    let shipments = db
        .query_shipments(
            &message.keyword,
            message.from,
            message.to,
            &message.status,
            &message.vendor,
            message.page,
        )
        .await?;
    let mut rows = Vec::new();
    for shipment in shipments.1.iter() {
        for item in shipment
            .items
            .iter()
            .filter(|items| items.status != OrderItemStatus::Concealed)
        {
            let q = items_map
                .entry((item.item_code_ext.clone(), item.rate.to_string()))
                .or_insert(0);
            *q += 1;
        }
    }

    let mut items = shipments
        .1
        .into_iter()
        .flat_map(|shipment| shipment.items)
        .filter(|item| item.status != OrderItemStatus::Concealed)
        .collect::<Vec<_>>();
    items.sort_by(|a, b| a.item_code_ext.cmp(&b.item_code_ext));
    for item in items.iter() {
        if let Some(q) = items_map.get(&(item.item_code_ext.clone(), item.rate.to_string())) {
            let item_detail = db
                .find_one_by_item_code(&item.item_code_ext.as_str()[..11])
                .await?
                .unwrap_or_else(|| PhItem::new_dummy(&item.item_code_ext, 0));
            let price_without_tax = get_tax_exclusive_price(item_detail.price);
            let row = vec![
                item.item_code_ext[0..11].to_string(),
                format!("{}", price_without_tax),
                item.item_code_ext[11..12].to_string(),
                item.item_code_ext[12..13].to_string(),
                String::from(""),
                format!("{}", q),
                stringify_rate(item.rate),
                format!(
                    "{}",
                    (*q as f64 * price_without_tax as f64 * item.rate).ceil() as u32
                ),
            ];
            rows.push(row);
            items_map.remove(&(item.item_code_ext.clone(), item.rate.to_string()));
        }
    }
    let now = Local::now();
    let filename = format!(
        "{}年{}年{}日生成出荷一覧.xlsx",
        now.year(),
        now.month(),
        now.day()
    );
    let message = ExportQueryShipmentMessage {
        filename: filename.clone(),
        rows,
    };
    let resp = http_client
        .post(format!(
            "{}/export/query_shipment",
            SETTINGS.utility.get_utility_url()
        ))
        .json(&message)
        .send()
        .await?;
    let url = validate_http_response::<DownLoadUrlResponse>(resp)
        .await?
        .url;

    Ok(Json(ExportFileResponse { url, filename }))
}

/// export a single shipment includes below column:
/// | 品牌 | 商品 | 数量 | 单件日元价格（不含税） | 合集日元价格（不含税） | 颜色 | 产地 | 材质 | 条形码 |
//pub async fn export_shipment_by_id(
//    Path(id): Path<Uuid>,
//    State(db): State<Arc<DbClient>>,
//) -> Result<impl IntoResponse> {
//    let mut shipment = db.get_shipment_by_id(id.into()).await?;
//    let transfers_opt =db
//        .find_transfer_by_shipment_id(shipment.id)
//        .await?;
//    let mut writer = ShipmentWriter::new(
//        &shipment.shipment_no,
//        &shipment.vendor.to_string(),
//        "./static",
//    );
//
//    //FIXME Is there a better way to do this??
//    let mut items_map: HashMap<&String, usize> = std::collections::HashMap::new();
//    shipment
//        .items
//        .sort_by(|a, b| a.customer_id.cmp(&b.customer_id));
//    for item in shipment
//        .items
//        .iter()
//        .filter(|item| item.status != OrderItemStatus::Concealed)
//    {
//        let q = items_map.entry(&item.item_code_ext).or_insert(0);
//        *q += 1;
//    }
//
//    for item in shipment.items.iter() {
//        if let Some(q) = items_map.get(&item.item_code_ext) {
//            let item_detail =db
//                .find_one_by_item_code(&item.item_code_ext.as_str()[..11])
//                .await?
//                .unwrap_or_else(|| PhItem::new_dummy(&item.item_code_ext, 0));
//
//            let item_type = get_item_type(&item.item_code_ext.as_str()[5..8])?;
//            let row = vec![
//                String::from("pinkhouse"),
//                item_type,
//                format!("{}", q),
//                format!("{}", get_tax_exclusive_price(item_detail.price)),
//                String::from(""),
//                String::from(&item.item_code_ext.as_str()[12..13]),
//                item_detail.made_in.unwrap_or_else(|| String::from("")),
//                item_detail.material.clone(),
//                item_detail.code.clone(),
//            ];
//            writer.add_row(&row);
//            items_map.remove(&item.item_code_ext);
//        }
//    }
//    if let Some(transfers) = transfers_opt {
//        for transfer in transfers {
//            for item in transfer
//                .items
//                .iter()
//                .filter(|item| item.count.is_positive())
//            {
//                let item_detail =db
//                    .find_one_by_item_code(&item.item_code_ext.as_str()[..11])
//                    .await?
//                    .unwrap_or_else(|| PhItem::new_dummy(&item.item_code_ext, 0));
//                let item_type = get_item_type(&item.item_code_ext.as_str()[5..8])?;
//                let row = vec![
//                    String::from("pinkhouse*"),
//                    item_type,
//                    format!("{}", item.count),
//                    format!("{}", get_tax_exclusive_price(item_detail.price)),
//                    String::from(""),
//                    String::from(&item.item_code_ext.as_str()[12..13]),
//                    item_detail.made_in.unwrap_or_else(|| String::from("")),
//                    item_detail.material.clone(),
//                    item_detail.code.clone(),
//                ];
//                writer.add_row(&row);
//            }
//        }
//    }
//    let shipment_datetime = shipment
//        .shipment_date
//        .to_chrono()
//        .with_timezone(&Local)
//        .format("%Y%m%d")
//        .to_string();
//
//    let file_name = format!(
//        "{}_eliad草纸_{}_{}",
//        &shipment.vendor.stringify_vendor(),
//        shipment_datetime,
//        &shipment.shipment_no
//    );
//    let path = tokio::task::spawn_blocking(|| writer.write(file_name)).await??;
//    let encoded = percent_encoding::percent_encode(path.as_bytes(), NON_ALPHANUMERIC);
//    Ok(Json::from(FilePath::new(&format!("/files/{encoded}"))))
//}
#[derive(Serialize)]
pub struct ExportSingleShipmentMessage {
    filename: String,
    shipment_no: String,
    rows: Vec<Vec<String>>,
}
#[derive(Deserialize)]
pub struct DownLoadUrlResponse {
    url: String,
}

#[derive(Serialize)]
pub struct ExportFileResponse {
    url: String,
    filename: String,
}
/// export a single shipment includes below column:
/// | 品牌 | 商品 | 数量 | 单件日元价格（不含税） | 合集日元价格（不含税） | 产地 | 材质 | 条形码 |
#[instrument(name = "export single shipment except color", skip(db))]
pub async fn export_shipment_by_id_except_color_no(
    Path(id): Path<Uuid>,
    State(db): State<Arc<DbClient>>,
    State(http_client): State<Arc<reqwest::Client>>,
) -> Result<impl IntoResponse> {
    let shipment = db.get_shipment_by_id(id.into()).await?;
    // find all shipments include the above one
    let shipment_items = db
        .find_shipments_by_no(&shipment.shipment_no)
        .await?
        .into_iter()
        .flat_map(|shipment| shipment.items)
        .filter(|item| item.status != OrderItemStatus::Concealed)
        .collect::<Vec<_>>();
    // find all transfers
    let transfer_items = db
        .find_transfer_by_shipment_no(&shipment.shipment_no)
        .await?
        .into_iter()
        .flat_map(|transfer| transfer.items)
        .collect::<Vec<_>>();
    let mut rows = Vec::new();
    //FIXME Is there a better way to do this??
    let mut items_map: HashMap<&str, usize> = std::collections::HashMap::new();
    let mut rates_map: HashMap<&str, f64> = std::collections::HashMap::new();
    for item in shipment_items.iter() {
        let q = items_map
            .entry(&item.item_code_ext.as_str()[..11])
            .or_insert(0);
        *q += 1;
        // loop over all shipment items set the discount rate to the lowest value
        // then even there are multi discount rate for same item_code discount rate will be the lowest value
        // transfer item as well
        let current_rate = rates_map
            .entry(&item.item_code_ext.as_str()[..11])
            .or_insert(item.rate);
        if item.rate > *current_rate {
            rates_map.insert(&item.item_code_ext.as_str()[..11], item.rate);
        }
    }

    for item in transfer_items
        .iter()
        .filter(|item| item.count.is_positive())
    {
        let q = items_map
            .entry(&item.item_code_ext.as_str()[..11])
            .or_insert(0);
        *q += item.count as usize;
    }

    for item in shipment_items.iter() {
        if let Some(q) = items_map.get(&item.item_code_ext.as_str()[..11]) {
            let item_detail = db
                .find_one_by_item_code(&item.item_code_ext.as_str()[..11])
                .await?
                .unwrap_or_else(|| PhItem::new_dummy(&item.item_code_ext, 0));
            let rate = rates_map
                .get(&item.item_code_ext.as_str()[..11])
                .unwrap_or(&1.0);
            let item_type = get_item_type(&item.item_code_ext.as_str()[5..8]);
            let row = vec![
                String::from("pinkhouse"),
                item_type,
                format!("{}", q),
                format!(
                    "{}",
                    get_tax_exclusive_price(item_detail.get_discounted_price(*rate))
                ),
                String::from(""),
                item_detail.made_in.unwrap_or_else(|| String::from("")),
                item_detail.material.clone(),
                item_detail.code.clone(),
            ];
            rows.push(row);
            items_map.remove(&item.item_code_ext.as_str()[..11]);
        }
    }
    for item in transfer_items
        .iter()
        .filter(|item| item.count.is_positive())
    {
        if let Some(q) = items_map.get(&item.item_code_ext.as_str()[..11]) {
            let item_detail = db
                .find_one_by_item_code(&item.item_code_ext.as_str()[..11])
                .await?
                .unwrap_or_else(|| PhItem::new_dummy(&item.item_code_ext, 0));
            let item_type = get_item_type(&item.item_code_ext.as_str()[5..8]);
            let row = vec![
                String::from("pinkhouse"),
                item_type,
                format!("{}", q),
                format!("{}", get_tax_exclusive_price(item_detail.price)),
                String::from(""),
                item_detail.made_in.unwrap_or_else(|| String::from("")),
                item_detail.material.clone(),
                item_detail.code.clone(),
            ];
            rows.push(row);
            items_map.remove(&item.item_code_ext.as_str()[..11]);
        }
    }
    let shipment_datetime = shipment
        .shipment_date
        .to_chrono()
        .with_timezone(&Local)
        .format("%Y%m%d")
        .to_string();

    let filename = format!(
        "{}_eliad草纸_{}_{}.xlsx",
        &shipment.vendor.stringify_vendor(),
        shipment_datetime,
        &shipment.shipment_no
    );
    debug!("generated new file");
    let message = ExportSingleShipmentMessage {
        filename: filename.clone(),
        rows,
        shipment_no: shipment.shipment_no,
    };
    let resp = http_client
        .post(format!(
            "{}/export/single_shipment",
            SETTINGS.utility.get_utility_url()
        ))
        .json(&message)
        .send()
        .await?;
    let url = validate_http_response::<DownLoadUrlResponse>(resp)
        .await?
        .url;

    Ok(Json(ExportFileResponse { url, filename }))
}

/// export a single shipment includes below column:
/// | 序号 | 品牌 | 商品 | 单件日元价格（不含税） | 产地 | 材质 | 条形码 |
#[instrument(name = "export single shipment contained ordered", skip(db))]
pub async fn export_shipment_ordered(
    Path(id): Path<Uuid>,
    State(db): State<Arc<DbClient>>,
    State(http_client): State<Arc<reqwest::Client>>,
) -> Result<impl IntoResponse> {
    let shipment = db.get_shipment_by_id(id.into()).await?;
    // find all shipments include the above one
    let mut shipment_items = db
        .find_shipments_by_no(&shipment.shipment_no)
        .await?
        .into_iter()
        .flat_map(|shipment| shipment.items)
        .collect::<Vec<_>>();
    // find all transfers
    shipment_items.sort_by(|a, b| a.customer_id.cmp(&b.customer_id));
    let mut rows = Vec::new();
    for (i, item) in shipment_items.iter().enumerate() {
        let item_detail = db
            .find_one_by_item_code(&item.item_code_ext.as_str()[..11])
            .await?
            .unwrap_or_else(|| PhItem::new_dummy(&item.item_code_ext, 0));
        let rate = item.rate;
        let item_type = get_item_type(&item.item_code_ext.as_str()[5..8]);
        // if order is concealed set customer id to empty string
        let customer_id = if item.status == OrderItemStatus::Concealed {
            String::from("-")
        } else {
            item.customer_id.to_string()
        };
        let row = vec![
            (i + 1).to_string(),
            customer_id,
            item_type,
            format!(
                "{}",
                get_tax_exclusive_price(item_detail.get_discounted_price(rate))
            ),
            item_detail.made_in.unwrap_or_else(|| String::from("")),
            item_detail.material.clone(),
            item_detail.code.clone(),
            item.item_code_ext.as_str()[12..13].to_string(),
        ];
        rows.push(row);
    }
    let shipment_datetime = shipment
        .shipment_date
        .to_chrono()
        .with_timezone(&Local)
        .format("%Y%m%d")
        .to_string();

    let filename = format!(
        "{}_发货_eliad草纸_{}_{}.xlsx",
        &shipment.vendor.stringify_vendor(),
        shipment_datetime,
        &shipment.shipment_no
    );
    debug!("generated new file");
    let message = ExportSingleShipmentMessage {
        filename: filename.clone(),
        rows,
        shipment_no: shipment.shipment_no,
    };
    let resp = http_client
        .post(format!(
            "{}/export/single_shipment_ordered",
            SETTINGS.utility.get_utility_url()
        ))
        .json(&message)
        .send()
        .await?;
    let url = validate_http_response::<DownLoadUrlResponse>(resp)
        .await?
        .url;

    Ok(Json(ExportFileResponse { url, filename }))
}

#[derive(Serialize)]
struct ExportJPInventoryMessage {
    filename: String,
    rows: Vec<Vec<String>>,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ExportInventoryQuery {
    location: InventoryLocation,
}

/// export inventory includes below column:
/// 图片 | 条形码 | 尺码 | 色号 | 数量 | 所在地 |
#[instrument(name = "export inventory include all location", skip(db))]
pub async fn export_jp_inventory(
    Query(export_location): Query<ExportInventoryQuery>,
    State(db): State<Arc<DbClient>>,
    State(http_client): State<Arc<reqwest::Client>>,
) -> Result<impl IntoResponse> {
    let location_query = if export_location.location == InventoryLocation::JP {
        String::from("jp")
    } else {
        String::from("cn,pcn")
    };
    let query = InventoryQuery {
        keyword: String::from(""),
        category: None,
        show_zero_quantity: false,
        location: Some(location_query),
        page: None,
    };
    let (_, inventory) = db.query_inventory(query).await?;
    let mut rows = vec![];
    for inventory_item in inventory {
        let item_code = &inventory_item.item_code_ext[0..11];
        let item_size = &inventory_item.item_code_ext[11..12];
        let item_color = &inventory_item.item_code_ext[12..13];
        let item_q = if export_location.location == InventoryLocation::JP {
            inventory_item.quantity[0].quantity.to_string()
        } else {
            (inventory_item.quantity[1].quantity + inventory_item.quantity[2].quantity).to_string()
        };
        rows.push(vec![
            format!(
                "https://d2vg6jg1lu9m12.cloudfront.net/{}_{}.jpeg",
                item_code, item_color
            ),
            item_code.to_string(),
            item_size.to_string(),
            item_color.to_string(),
            item_q,
            export_location.location.kanjified(),
        ])
    }
    let now = Local::now();
    let filename = format!(
        "{}年{}月{}日导出{}在库.xlsx",
        now.year(),
        now.month(),
        now.day(),
        export_location.location.kanjified(),
    );
    let message = ExportJPInventoryMessage {
        filename: filename.clone(),
        rows,
    };
    let resp = http_client
        .post(format!(
            "{}/export/inventory",
            SETTINGS.utility.get_utility_url()
        ))
        .json(&message)
        .send()
        .await?;
    let url = validate_http_response::<DownLoadUrlResponse>(resp)
        .await?
        .url;

    Ok(Json(ExportFileResponse { url, filename }))
}

fn get_tax_exclusive_price(i: u32) -> u32 {
    (i as f64 / 1.1).round() as u32
}

fn stringify_rate(i: f64) -> String {
    if i == 1.0 {
        return String::from("-");
    }
    format!("{}%Off", ((1.0 - i) * 100.0).round() as u32)
}

fn get_item_type(input: &str) -> String {
    match input {
        "FB_" => String::from("衬衫"),
        "FBY" => String::from("长衬衫"),
        "FS_" => String::from("半裙"),
        "FSY" => String::from("半裙"),
        "FA_" => String::from("连衣裙"),
        "FAY" => String::from("连衣裙"),
        "FP_" => String::from("裤子"),
        "FJM" => String::from("外套"),
        "KPO" => String::from("毛衣"),
        "UPO" => String::from("针织衫"),
        "UBY" => String::from("长上衣"),
        "UA_" => String::from("针织连衣裙"),
        "UCD" => String::from("针织开衫"),
        "KCD" => String::from("毛线开衫"),
        "PSH" => String::from("鞋子"),
        "PBG" => String::from("包包"),
        "PSC" => String::from("袜子"),
        "PE_" => String::from("配饰"),
        "FC_" => String::from("大衣"),
        "UTR" => String::from("卫衣"),
        "UTS" => String::from("T恤"),
        "PSE" => String::from("围巾"),
        "PAC" => String::from("首饰"),
        "FJ_" => String::from("外套夹克"),
        "UP_" => String::from("裤子"),
        "PHT" => String::from("帽子"),
        "FV_" => String::from("马甲"),
        "LJM" => String::from("皮夹克"),
        "KV_" => String::from("针织马甲"),
        "PHH" => String::from("画册"),
        _ => {
            warn!("{input} is not presented above");
            input.to_string()
        }
    }
}
