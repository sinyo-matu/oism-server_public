// auto generated application permissions
#[macro_export]
macro_rules! impl_application_path {
    ($n:ident) => {
        impl $crate::server::path_control::ApplicationPath for $n {
            fn root_path(&self) -> String {
                self.route.clone()
            }
            fn inject_auth_router(
                self,
                router: axum::Router<$crate::server::AppState>,
            ) -> axum::Router<$crate::server::AppState> {
                let cloned =
                    std::sync::Arc::new(self) as std::sync::Arc<dyn $crate::server::path_control::ApplicationPath>;
                router.route_layer(axum::middleware::from_fn_with_state(
                    cloned,
                    $crate::server::middleware::auth,
                ))
            }
            fn get_matcher(
                &self,
            ) -> &matchit::Router<
                std::collections::HashMap<axum::http::Method, $crate::db::auth::UserRole>,
            > {
                &self.matcher
            }
        }
    };
}
#[derive(Clone)]
pub struct OrdersPath {
    pub route: String,
    matcher: matchit::Router<std::collections::HashMap<axum::http::Method, crate::db::auth::UserRole>> 
}

impl Default for OrdersPath {
fn default() -> Self {
    let mut matcher = matchit::Router::new();
   matcher
    .insert(
        "/",
        std::collections::HashMap::from([
            (axum::http::Method::GET,crate::db::auth::UserRole::Viewer),
            (axum::http::Method::POST,crate::db::auth::UserRole::Editor),
        ]),
        ).unwrap();
   matcher
    .insert(
        "/:id",
        std::collections::HashMap::from([
            (axum::http::Method::GET,crate::db::auth::UserRole::Viewer),
            (axum::http::Method::DELETE,crate::db::auth::UserRole::Editor),
        ]),
        ).unwrap();
   matcher
    .insert(
        "/taobao_no/:taobao_no",
        std::collections::HashMap::from([
            (axum::http::Method::GET,crate::db::auth::UserRole::Viewer),
        ]),
        ).unwrap();
   matcher
    .insert(
        "/:id/note",
        std::collections::HashMap::from([
            (axum::http::Method::PATCH,crate::db::auth::UserRole::Editor),
        ]),
        ).unwrap();
   matcher
    .insert(
        "/check_then_update",
        std::collections::HashMap::from([
            (axum::http::Method::PUT,crate::db::auth::UserRole::Editor),
        ]),
        ).unwrap();

        Self {
            route: String::from("/orders"),
            matcher
        }
    }
}

impl_application_path!(OrdersPath);
    
#[derive(Clone)]
pub struct OrderItemsPath {
    pub route: String,
    matcher: matchit::Router<std::collections::HashMap<axum::http::Method, crate::db::auth::UserRole>> 
}

impl Default for OrderItemsPath {
fn default() -> Self {
    let mut matcher = matchit::Router::new();
   matcher
    .insert(
        "/",
        std::collections::HashMap::from([
            (axum::http::Method::GET,crate::db::auth::UserRole::Viewer),
        ]),
        ).unwrap();
   matcher
    .insert(
        "/:id",
        std::collections::HashMap::from([
            (axum::http::Method::GET,crate::db::auth::UserRole::Viewer),
            (axum::http::Method::DELETE,crate::db::auth::UserRole::Editor),
        ]),
        ).unwrap();
   matcher
    .insert(
        "/:id/rate",
        std::collections::HashMap::from([
            (axum::http::Method::PATCH,crate::db::auth::UserRole::Editor),
        ]),
        ).unwrap();

        Self {
            route: String::from("/order_items"),
            matcher
        }
    }
}

impl_application_path!(OrderItemsPath);
    
#[derive(Clone)]
pub struct RegistersPath {
    pub route: String,
    matcher: matchit::Router<std::collections::HashMap<axum::http::Method, crate::db::auth::UserRole>> 
}

impl Default for RegistersPath {
fn default() -> Self {
    let mut matcher = matchit::Router::new();
   matcher
    .insert(
        "/",
        std::collections::HashMap::from([
            (axum::http::Method::GET,crate::db::auth::UserRole::Viewer),
            (axum::http::Method::POST,crate::db::auth::UserRole::Editor),
        ]),
        ).unwrap();
   matcher
    .insert(
        "/:id",
        std::collections::HashMap::from([
            (axum::http::Method::GET,crate::db::auth::UserRole::Viewer),
            (axum::http::Method::DELETE,crate::db::auth::UserRole::Editor),
        ]),
        ).unwrap();

        Self {
            route: String::from("/registers"),
            matcher
        }
    }
}

impl_application_path!(RegistersPath);
    
#[derive(Clone)]
pub struct InventoryPath {
    pub route: String,
    matcher: matchit::Router<std::collections::HashMap<axum::http::Method, crate::db::auth::UserRole>> 
}

impl Default for InventoryPath {
fn default() -> Self {
    let mut matcher = matchit::Router::new();
   matcher
    .insert(
        "/",
        std::collections::HashMap::from([
            (axum::http::Method::GET,crate::db::auth::UserRole::Viewer),
        ]),
        ).unwrap();
   matcher
    .insert(
        "/operations/:item_code_ext",
        std::collections::HashMap::from([
            (axum::http::Method::GET,crate::db::auth::UserRole::Viewer),
        ]),
        ).unwrap();
   matcher
    .insert(
        "/quantity/:item_code_ext",
        std::collections::HashMap::from([
            (axum::http::Method::GET,crate::db::auth::UserRole::Viewer),
        ]),
        ).unwrap();
   matcher
    .insert(
        "/export",
        std::collections::HashMap::from([
            (axum::http::Method::GET,crate::db::auth::UserRole::Viewer),
        ]),
        ).unwrap();

        Self {
            route: String::from("/inventory"),
            matcher
        }
    }
}

impl_application_path!(InventoryPath);
    
#[derive(Clone)]
pub struct ReturnPath {
    pub route: String,
    matcher: matchit::Router<std::collections::HashMap<axum::http::Method, crate::db::auth::UserRole>> 
}

impl Default for ReturnPath {
fn default() -> Self {
    let mut matcher = matchit::Router::new();
   matcher
    .insert(
        "/",
        std::collections::HashMap::from([
            (axum::http::Method::GET,crate::db::auth::UserRole::Viewer),
            (axum::http::Method::POST,crate::db::auth::UserRole::Editor),
        ]),
        ).unwrap();
   matcher
    .insert(
        "/:id",
        std::collections::HashMap::from([
            (axum::http::Method::GET,crate::db::auth::UserRole::Viewer),
            (axum::http::Method::DELETE,crate::db::auth::UserRole::Editor),
        ]),
        ).unwrap();

        Self {
            route: String::from("/return"),
            matcher
        }
    }
}

impl_application_path!(ReturnPath);
    
#[derive(Clone)]
pub struct ShipmentPath {
    pub route: String,
    matcher: matchit::Router<std::collections::HashMap<axum::http::Method, crate::db::auth::UserRole>> 
}

impl Default for ShipmentPath {
fn default() -> Self {
    let mut matcher = matchit::Router::new();
   matcher
    .insert(
        "/",
        std::collections::HashMap::from([
            (axum::http::Method::GET,crate::db::auth::UserRole::Viewer),
            (axum::http::Method::POST,crate::db::auth::UserRole::Editor),
        ]),
        ).unwrap();
   matcher
    .insert(
        "/:id",
        std::collections::HashMap::from([
            (axum::http::Method::GET,crate::db::auth::UserRole::Viewer),
            (axum::http::Method::DELETE,crate::db::auth::UserRole::Editor),
        ]),
        ).unwrap();
   matcher
    .insert(
        "/:id/note",
        std::collections::HashMap::from([
            (axum::http::Method::PATCH,crate::db::auth::UserRole::Editor),
        ]),
        ).unwrap();
   matcher
    .insert(
        "/:id/status",
        std::collections::HashMap::from([
            (axum::http::Method::PUT,crate::db::auth::UserRole::Editor),
        ]),
        ).unwrap();
   matcher
    .insert(
        "/:id/no",
        std::collections::HashMap::from([
            (axum::http::Method::PUT,crate::db::auth::UserRole::Editor),
        ]),
        ).unwrap();
   matcher
    .insert(
        "/:id/vendor",
        std::collections::HashMap::from([
            (axum::http::Method::PUT,crate::db::auth::UserRole::Editor),
        ]),
        ).unwrap();
   matcher
    .insert(
        "/:id/export",
        std::collections::HashMap::from([
            (axum::http::Method::GET,crate::db::auth::UserRole::Viewer),
        ]),
        ).unwrap();
   matcher
    .insert(
        "/:id/export_ordered",
        std::collections::HashMap::from([
            (axum::http::Method::GET,crate::db::auth::UserRole::Viewer),
        ]),
        ).unwrap();
   matcher
    .insert(
        "/by_no/:no",
        std::collections::HashMap::from([
            (axum::http::Method::GET,crate::db::auth::UserRole::Viewer),
        ]),
        ).unwrap();
   matcher
    .insert(
        "/export",
        std::collections::HashMap::from([
            (axum::http::Method::GET,crate::db::auth::UserRole::Viewer),
        ]),
        ).unwrap();

        Self {
            route: String::from("/shipment"),
            matcher
        }
    }
}

impl_application_path!(ShipmentPath);
    
#[derive(Clone)]
pub struct TransferPath {
    pub route: String,
    matcher: matchit::Router<std::collections::HashMap<axum::http::Method, crate::db::auth::UserRole>> 
}

impl Default for TransferPath {
fn default() -> Self {
    let mut matcher = matchit::Router::new();
   matcher
    .insert(
        "/",
        std::collections::HashMap::from([
            (axum::http::Method::GET,crate::db::auth::UserRole::Viewer),
            (axum::http::Method::POST,crate::db::auth::UserRole::Editor),
        ]),
        ).unwrap();
   matcher
    .insert(
        "/:id",
        std::collections::HashMap::from([
            (axum::http::Method::GET,crate::db::auth::UserRole::Viewer),
            (axum::http::Method::DELETE,crate::db::auth::UserRole::Editor),
        ]),
        ).unwrap();
   matcher
    .insert(
        "/:id/shipments",
        std::collections::HashMap::from([
            (axum::http::Method::GET,crate::db::auth::UserRole::Viewer),
        ]),
        ).unwrap();
   matcher
    .insert(
        "/:id/shipment_no",
        std::collections::HashMap::from([
            (axum::http::Method::PUT,crate::db::auth::UserRole::Editor),
        ]),
        ).unwrap();
   matcher
    .insert(
        "/shipment_no/:shipment_no",
        std::collections::HashMap::from([
            (axum::http::Method::GET,crate::db::auth::UserRole::Viewer),
        ]),
        ).unwrap();
   matcher
    .insert(
        "/by_shipment_id/:shipment_id",
        std::collections::HashMap::from([
            (axum::http::Method::GET,crate::db::auth::UserRole::Viewer),
        ]),
        ).unwrap();

        Self {
            route: String::from("/transfer"),
            matcher
        }
    }
}

impl_application_path!(TransferPath);
    
#[derive(Clone)]
pub struct ControlPath {
    pub route: String,
    matcher: matchit::Router<std::collections::HashMap<axum::http::Method, crate::db::auth::UserRole>> 
}

impl Default for ControlPath {
fn default() -> Self {
    let mut matcher = matchit::Router::new();
   matcher
    .insert(
        "/",
        std::collections::HashMap::from([
            (axum::http::Method::GET,crate::db::auth::UserRole::Visitor),
        ]),
        ).unwrap();

        Self {
            route: String::from("/control"),
            matcher
        }
    }
}

impl_application_path!(ControlPath);
    
#[derive(Clone)]
pub struct HealthCheckPath {
    pub route: String,
    matcher: matchit::Router<std::collections::HashMap<axum::http::Method, crate::db::auth::UserRole>> 
}

impl Default for HealthCheckPath {
fn default() -> Self {
    let mut matcher = matchit::Router::new();
   matcher
    .insert(
        "/",
        std::collections::HashMap::from([
            (axum::http::Method::GET,crate::db::auth::UserRole::Visitor),
        ]),
        ).unwrap();

        Self {
            route: String::from("/health_check"),
            matcher
        }
    }
}

impl_application_path!(HealthCheckPath);
    
#[derive(Clone)]
pub struct UserInfoPath {
    pub route: String,
    matcher: matchit::Router<std::collections::HashMap<axum::http::Method, crate::db::auth::UserRole>> 
}

impl Default for UserInfoPath {
fn default() -> Self {
    let mut matcher = matchit::Router::new();
   matcher
    .insert(
        "/",
        std::collections::HashMap::from([
            (axum::http::Method::GET,crate::db::auth::UserRole::Editor),
        ]),
        ).unwrap();

        Self {
            route: String::from("/user_info"),
            matcher
        }
    }
}

impl_application_path!(UserInfoPath);
    

#[derive(Default)]
pub struct PrivatePath {
   pub orders_path:OrdersPath,
   pub order_items_path:OrderItemsPath,
   pub registers_path:RegistersPath,
   pub inventory_path:InventoryPath,
   pub return_path:ReturnPath,
   pub shipment_path:ShipmentPath,
   pub transfer_path:TransferPath,
   pub control_path:ControlPath,
   pub health_check_path:HealthCheckPath,
   pub user_info_path:UserInfoPath,
}
