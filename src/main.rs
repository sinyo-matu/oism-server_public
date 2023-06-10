mod test;

use std::net::TcpListener;

use oism_server::{db::mongo::DbClient, server::auth::SETTINGS};
use oism_server::{
    error_result::Result,
    telemetry::{get_subscriber, init_subscriber},
};
use secrecy::ExposeSecret;
use tracing::info;

#[tokio::main]
async fn main() -> Result<()> {
    let file_appender = tracing_appender::rolling::daily("data/log/", "smt.log");
    let (non_blocking, _guard) = tracing_appender::non_blocking(file_appender);
    let subscriber = get_subscriber("oism-server".into(), "info".into(), non_blocking);
    init_subscriber(subscriber);
    info!(
        "token expiration: access = {}s refresh = {}s",
        SETTINGS.access_expiration, SETTINGS.refresh_expiration
    );
    let db = DbClient::init(
        SETTINGS.database.connection_string_cloud().expose_secret(),
        &SETTINGS.database.database_name,
    )
    .await?;
    let listener = TcpListener::bind(format!("0.0.0.0:{}", SETTINGS.application_port)).unwrap();
    oism_server::server::server_start(db, listener).await;
    Ok(())
}
