pub mod auth;
pub mod export;
pub mod inventory;
pub mod middleware;
pub mod order;
pub mod path_control;
pub mod register;
pub mod retrn;
pub mod shipment;
pub mod transfer;
pub mod ws;

use crate::{
    application_path_gen::PrivatePath,
    cache::OrderCache,
    error_result::Result,
    server::{
        auth::{get_user_info_handler, login, sign_up, token_refresh_handler, UserInfo},
        inventory::get_inventory_router,
        retrn::get_return_router,
        shipment::get_shipment_router,
        transfer::get_transfer_router,
        ws::{handle_ws, ControlMessage},
    },
    services::google_service::GoogleService,
};
use axum::{
    extract::FromRef,
    http::header::{AUTHORIZATION, CONTENT_ENCODING, CONTENT_LANGUAGE, CONTENT_TYPE, LOCATION},
    http::StatusCode,
    middleware::from_extractor,
    response::IntoResponse,
    routing::{any, get, post},
    Extension, Router,
};
use chrono::prelude::*;
use chrono::serde::ts_seconds;
use mongodb::bson::Bson;
use path_control::ApplicationPath;
use reqwest::Method;
use serde::{Deserialize, Serialize};
use std::{fmt::Display, net::TcpListener, sync::Arc};
use tokio::sync::broadcast::Sender;
use tower::ServiceBuilder;
use tower_http::trace::TraceLayer;
use tower_http::{compression::CompressionLayer, cors::CorsLayer};
use tracing::{info, instrument};
use uuid::Uuid;

use crate::{
    cache::MapCache,
    db::{inventory::Quantity, mongo::DbClient, shipment::ShipmentVendor},
};

#[derive(Clone, FromRef)]
pub struct AppState {
    db_client: Arc<DbClient>,
    order_cache: Arc<dyn OrderCache>,
    http_client: Arc<reqwest::Client>,
    sender: Arc<Sender<ControlMessage>>,
    google_service: Arc<GoogleService>,
}

#[instrument(skip(db_client))]
pub async fn server_start(db_client: DbClient, listener: TcpListener) {
    let db = Arc::new(db_client);
    let cache = MapCache::new();
    let order_cache = cache as Arc<dyn OrderCache>;
    let http_client = Arc::new(reqwest::Client::new());
    let origins = vec![
        "https://oism.app".parse().unwrap(),
        "http://localhost:3000".parse().unwrap(),
        "http://localhost:8000".parse().unwrap(),
        "https://tools.oism.app".parse().unwrap(),
        "http://localhost:25504".parse().unwrap(),
    ];
    let cors = CorsLayer::new()
        .allow_methods([
            Method::GET,
            Method::POST,
            Method::PATCH,
            Method::DELETE,
            Method::HEAD,
            Method::OPTIONS,
            Method::PUT,
        ])
        .allow_credentials(true)
        .allow_headers(vec![
            AUTHORIZATION,
            CONTENT_TYPE,
            LOCATION,
            CONTENT_LANGUAGE,
            CONTENT_ENCODING,
        ])
        .allow_origin(origins);
    let google_service = Arc::new(GoogleService::default());
    let (orders_tx, _rx) = tokio::sync::broadcast::channel::<ControlMessage>(100);
    let shared_tx = Arc::new(orders_tx);
    let state = AppState {
        db_client: db,
        order_cache,
        http_client,
        sender: shared_tx,
        google_service,
    };
    let layer = ServiceBuilder::new()
        .layer(TraceLayer::new_for_http())
        .layer(CompressionLayer::new())
        .layer(cors);
    let PrivatePath {
        orders_path,
        order_items_path,
        registers_path,
        return_path,
        inventory_path,
        shipment_path,
        transfer_path,
        control_path,
        health_check_path,
        user_info_path,
    } = PrivatePath::default();
    let control_route = Router::new().route("/", get(handle_ws));
    let health_check_route = Router::new().route("/", get(health_check));
    let user_info_route = Router::new().route("/", get(get_user_info_handler));
    let private_route = Router::new()
        .nest(
            health_check_path.root_path().as_str(),
            health_check_path.inject_auth_router(health_check_route),
        )
        .nest(
            orders_path.root_path().as_str(),
            orders_path.inject_auth_router(order::get_router()),
        )
        .nest(
            order_items_path.root_path().as_str(),
            order_items_path.inject_auth_router(order::get_items_router()),
        )
        .nest(
            registers_path.root_path().as_str(),
            registers_path.inject_auth_router(register::get_router()),
        )
        .nest(
            inventory_path.root_path().as_str(),
            inventory_path.inject_auth_router(get_inventory_router()),
        )
        .nest(
            return_path.root_path().as_str(),
            return_path.inject_auth_router(get_return_router()),
        )
        .nest(
            shipment_path.root_path().as_str(),
            shipment_path.inject_auth_router(get_shipment_router()),
        )
        .nest(
            transfer_path.root_path().as_str(),
            transfer_path.inject_auth_router(get_transfer_router()),
        )
        .nest(
            control_path.root_path().as_str(),
            control_path.inject_auth_router(control_route),
        )
        .nest(
            user_info_path.root_path().as_str(),
            user_info_path.inject_auth_router(user_info_route),
        )
        .route_layer(from_extractor::<UserInfo>());
    let sign_up_route = Router::new().route("/", post(sign_up));
    let login_route = Router::new().route("/", post(login));
    let refresh_token_route = Router::new().route("/", any(token_refresh_handler));
    let public_route = Router::new()
        .nest("/signup", sign_up_route)
        .nest("/refresh_token", refresh_token_route)
        .nest("/login", login_route);
    let api_route = Router::new()
        .nest("/public", public_route)
        .nest("/private", private_route)
        .layer(Extension(state.clone()))
        .with_state(state);

    let app = Router::new().nest("/api/v1", api_route).layer(layer);
    info!("server started at {}", listener.local_addr().unwrap());
    axum::Server::from_tcp(listener)
        .unwrap()
        .serve(app.into_make_service())
        .await
        .expect("server start failed");
}

async fn health_check() -> Result<impl IntoResponse> {
    Ok(StatusCode::OK)
}

#[derive(Deserialize, Serialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct OrderRegisterInput {
    pub taobao_order_no: String,
    pub customer_id: String,
    pub note: String,
    pub items: Vec<InputOrderItem>,
    #[serde(with = "ts_seconds")]
    pub order_datetime: DateTime<Utc>,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct InputOrderItem {
    pub item_code_ext: String,
    pub rate: f64,
    pub quantity: Vec<Quantity>,
    pub price: u32,
    pub is_manual: bool,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct NewShipmentInput {
    pub shipment_no: String,
    pub note: String,
    pub vendor: ShipmentVendor,
    #[serde(with = "ts_seconds")]
    pub shipment_date: DateTime<Utc>,
    pub item_ids: Vec<Uuid>,
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct PagedResponse<D> {
    pub data: Vec<D>,
    pub next: u32,
    pub has_next: bool,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum AppPrivateRoute {
    HealthCheck,
    Orders,
    OrderItems,
    Registers,
    Inventory,
    Return,
    Shipment,
    Transfer,
    Control,
    UserInfo,
    Root,
}

impl From<String> for AppPrivateRoute {
    fn from(s: String) -> Self {
        println!("path :{s}");
        match s.as_str() {
            "/health_check" => AppPrivateRoute::HealthCheck,
            "/orders" => AppPrivateRoute::Orders,
            "/order_items" => AppPrivateRoute::OrderItems,
            "/registers" => AppPrivateRoute::Registers,
            "/inventory" => AppPrivateRoute::Inventory,
            "/return" => AppPrivateRoute::Return,
            "/shipment" => AppPrivateRoute::Shipment,
            "/transfer" => AppPrivateRoute::Transfer,
            "/control" => AppPrivateRoute::Control,
            "/user_info" => AppPrivateRoute::UserInfo,
            "/" => AppPrivateRoute::Root,
            _ => unreachable!(),
        }
    }
}

impl Display for AppPrivateRoute {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AppPrivateRoute::HealthCheck => f.write_str("health_check"),
            AppPrivateRoute::Orders => f.write_str("orders"),
            AppPrivateRoute::OrderItems => f.write_str("order_items"),
            AppPrivateRoute::Registers => f.write_str("registers"),
            AppPrivateRoute::Inventory => f.write_str("inventory"),
            AppPrivateRoute::Return => f.write_str("return"),
            AppPrivateRoute::Shipment => f.write_str("shipment"),
            AppPrivateRoute::Transfer => f.write_str("transfer"),
            AppPrivateRoute::Control => f.write_str("control"),
            AppPrivateRoute::UserInfo => f.write_str("user_info"),
            AppPrivateRoute::Root => f.write_str("root"),
        }
    }
}

impl From<AppPrivateRoute> for Bson {
    fn from(r: AppPrivateRoute) -> Self {
        match r {
            AppPrivateRoute::HealthCheck => Bson::String(String::from("health_check")),
            AppPrivateRoute::Orders => Bson::String(String::from("orders")),
            AppPrivateRoute::OrderItems => Bson::String(String::from("order_items")),
            AppPrivateRoute::Registers => Bson::String(String::from("registers")),
            AppPrivateRoute::Inventory => Bson::String(String::from("inventory")),
            AppPrivateRoute::Return => Bson::String(String::from("return")),
            AppPrivateRoute::Shipment => Bson::String(String::from("shipment")),
            AppPrivateRoute::Transfer => Bson::String(String::from("transfer")),
            AppPrivateRoute::Control => Bson::String(String::from("control")),
            AppPrivateRoute::UserInfo => Bson::String(String::from("user_info")),
            AppPrivateRoute::Root => Bson::String(String::from("root")),
        }
    }
}
