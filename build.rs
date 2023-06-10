use convert_case::{Case, Casing};
use std::io::Write;
use std::{fs::File, io, path::Path};

use serde::Deserialize;

#[derive(Deserialize)]
struct AppPermissions {
    app_permissions: Vec<Route>,
}

#[derive(Deserialize)]
struct Route {
    route: String,
    sub_route: Vec<SubRoute>,
}

#[derive(Deserialize)]
struct SubRoute {
    path: String,
    permissions: Vec<Permissions>,
}

#[derive(Deserialize)]
struct Permissions {
    method: String,
    role: String,
}

fn main() -> io::Result<()> {
    let out = Path::new("./src").join("application_path_gen.rs");
    let mut out = File::create(out).unwrap();
    let permission_file = File::open("path_permission_cfg.json").unwrap();
    let app_permissions: AppPermissions = serde_json::from_reader(permission_file).unwrap();
    writeln!(out, "// auto generated application permissions")?;
    writeln!(
        out,
        "#[macro_export]
macro_rules! impl_application_path {{
    ($n:ident) => {{
        impl $crate::server::path_control::ApplicationPath for $n {{
            fn root_path(&self) -> String {{
                self.route.clone()
            }}
            fn inject_auth_router(
                self,
                router: axum::Router<$crate::server::AppState>,
            ) -> axum::Router<$crate::server::AppState> {{
                let cloned =
                    std::sync::Arc::new(self) as std::sync::Arc<dyn $crate::server::path_control::ApplicationPath>;
                router.route_layer(axum::middleware::from_fn_with_state(
                    cloned,
                    $crate::server::middleware::auth,
                ))
            }}
            fn get_matcher(
                &self,
            ) -> &matchit::Router<
                std::collections::HashMap<axum::http::Method, $crate::db::auth::UserRole>,
            > {{
                &self.matcher
            }}
        }}
    }};
}}"
    )?;
    let mut struct_names = vec![];
    for route in app_permissions.app_permissions {
        let path_struct_name = route.route.trim_start_matches('/').to_case(Case::Pascal)
            + String::from("Path").as_str();
        struct_names.push((
            route.route.trim_start_matches('/').to_owned(),
            path_struct_name.clone(),
        ));
        write!(out, "#[derive(Clone)]
pub struct {path_struct_name} {{
    pub route: String,
    matcher: matchit::Router<std::collections::HashMap<axum::http::Method, crate::db::auth::UserRole>> 
}}

impl Default for {path_struct_name} {{
fn default() -> Self {{
    let mut matcher = matchit::Router::new();
")?;
        for sub in route.sub_route {
            write!(
                out,
                "   matcher
    .insert(
        \"{}\",
        std::collections::HashMap::from([",
                sub.path
            )?;
            for permission in sub.permissions {
                write!(
                    out,
                    "
            (axum::http::Method::{},crate::db::auth::UserRole::{}),",
                    permission.method,
                    &permission.role.to_case(Case::Pascal)
                )?;
            }
            writeln!(
                out,
                "
        ]),
        ).unwrap();"
            )?;
        }
        writeln!(
            out,
            "
        Self {{
            route: String::from(\"{}\"),
            matcher
        }}
    }}
}}

impl_application_path!({path_struct_name});
    ",
            route.route
        )?;
    }
    write!(
        out,
        "
#[derive(Default)]
pub struct PrivatePath {{
"
    )?;
    for (field, struct_name) in struct_names {
        writeln!(out, "   pub {field}_path:{struct_name},")?;
    }
    writeln!(out, "}}")?;
    Ok(())
}
