use chrono::{DateTime, Utc};
use fake::Fake;
use oism_server::{
    db::{mongo::DbClient, RegisterItemInput, StockRegisterInput},
    telemetry::{get_subscriber, init_subscriber},
};
use once_cell::sync::Lazy;

use std::net::TcpListener;
pub struct TestApp {
    pub address: String,
    pub db: DbClient,
    pub request_client: reqwest::Client,
}
static TRACING: Lazy<()> = Lazy::new(|| {
    let subscriber = get_subscriber("test".into(), "debug".into(), std::io::stdout);
    init_subscriber(subscriber);
});

pub async fn spawn_app() -> TestApp {
    Lazy::force(&TRACING);
    let listener = TcpListener::bind("127.0.0.1:0").expect("Failed to bind random port");
    let port = listener.local_addr().unwrap().port();
    let address = format!("http://127.0.0.1:{port}");
    let database_name = uuid::Uuid::new_v4().to_string();
    let connect_string = "mongodb://127.0.0.1:27017";
    let db_client = DbClient::init(connect_string, &database_name)
        .await
        .expect("Failed to connect to mongodb");
    tokio::spawn(oism_server::server::server_start(
        db_client.clone(),
        listener,
    ));
    TestApp {
        address,
        db: db_client,
        request_client: reqwest::Client::builder()
            .cookie_store(true)
            .build()
            .unwrap(),
    }
}

impl TestApp {
    pub fn public_base_uri(&self) -> String {
        format!("{}/api/v1/public", self.address)
    }
    pub fn private_base_uri(&self) -> String {
        format!("{}/api/v1/private", self.address)
    }
    pub async fn signup_test_user(&self) -> (&str, &str) {
        let (username, password) = ("test1", "123456");
        let body = serde_json::json!(
            {
                "username":username,
                "password":password,
                "role":"full",
                "sub_role":"{}",
                "secret":"eliamo_daidaidai"
            }
        );
        self.request_client
            .post(format!("{}/signup", self.public_base_uri()))
            .json(&body)
            .send()
            .await
            .expect("Failed execute request");
        (username, password)
    }

    pub async fn signup_and_login(&self) {
        let (username, password) = ("test1", "123456");
        let body = serde_json::json!(
            {
                "username":username,
                "password":password,
                "role":"full",
                "sub_role":"{}",
                "secret":"eliamo_daidaidai"
            }
        );
        let public_base_uri = self.public_base_uri();
        let response1 = self
            .request_client
            .post(format!("{public_base_uri}/signup"))
            .json(&body)
            .send()
            .await
            .expect("Failed execute request");
        assert_eq!(201, response1.status().as_u16());
        let body = serde_json::json!(
            {
                "username":username,
                "password":password
            }
        );
        let response2 = self
            .request_client
            .post(format!("{public_base_uri}/login"))
            .json(&body)
            .send()
            .await
            .expect("Failed to execute request");
        assert!(response2.status().is_success());
        assert!(response2.cookies().any(|c| c.name() == "smt_token"));
        assert!(response2.cookies().any(|c| c.name() == "smt_id"));
    }

    pub async fn register_inventory(&self) -> (Vec<(String, u32)>, DateTime<Utc>) {
        let item_seeds = vec![
            ("A2121FSY06693".to_string(), 1),
            ("A2121FSY00991".to_string(), 2),
            ("A2121FSY07292".to_string(), 2),
        ];
        let create_register_res = create_register_input(&item_seeds);
        let body = serde_json::json!(create_register_res.inputs);
        let private_base_uri = self.private_base_uri();
        let response1 = self
            .request_client
            .post(format!("{private_base_uri}/registers/"))
            .json(&body)
            .send()
            .await
            .expect("Failed execute request");
        assert_eq!(201, response1.status().as_u16());
        (item_seeds, create_register_res.register_time)
    }

    pub async fn cleanup(self) {
        self.db
            .ph_db
            .drop(None)
            .await
            .expect("Failed to drop database");
    }
}
pub struct CreateRegisterRes {
    register_time: DateTime<Utc>,
    inputs: StockRegisterInput,
}

fn create_register_input(inputs: &[(String, u32)]) -> CreateRegisterRes {
    let mut items = Vec::new();
    for (item_code_ext, q) in inputs {
        items.push(RegisterItemInput {
            item_code_ext: item_code_ext.into(),
            price: (1000..50000).fake::<u32>(),
            count: *q,
            is_manual: false,
        })
    }
    let register_time = Utc::now();
    CreateRegisterRes {
        register_time,
        inputs: StockRegisterInput {
            arrival_date: register_time,
            no: 5.fake::<String>(),
            items,
        },
    }
}
