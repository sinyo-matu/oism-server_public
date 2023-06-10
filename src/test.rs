#![allow(unused_imports)]
use std::io::{stdout, Write};

use oism_server::db::mongo::DbClient;

use super::*;

#[tokio::test]
#[ignore = "just for starting dev server"]
async fn test_main_locally() -> Result<()> {
    let subscriber = get_subscriber("test".into(), "info".into(), std::io::stdout);
    init_subscriber(subscriber);
    let listener = TcpListener::bind("0.0.0.0:24463").unwrap();
    info!(
        "token expiration: access = {}s refresh = {}s",
        SETTINGS.access_expiration, SETTINGS.refresh_expiration
    );
    let db = DbClient::init(
        SETTINGS.database.connection_string_cloud().expose_secret(),
        &SETTINGS.database.database_name,
    )
    .await?;
    oism_server::server::server_start(db, listener).await;
    Ok(())
}
