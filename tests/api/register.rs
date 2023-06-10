use oism_server::db::Register;

use crate::helpers::spawn_app;

#[tokio::test]
async fn register_inventory_works() {
    let app = spawn_app().await;
    app.signup_and_login().await;
    let (registered, register_time) = app.register_inventory().await;
    let private_base_uri = app.private_base_uri();
    let response1 = app
        .request_client
        .get(format!("{private_base_uri}/registers/"))
        .query(&[
            ("from", register_time.timestamp() - 100),
            ("to", register_time.timestamp() + 100),
        ])
        .send()
        .await
        .expect("Failed to request");
    assert!(response1.status().is_success());
    let registers: Vec<Register> = response1
        .json()
        .await
        .expect("Failed to deserialize json data");
    assert_eq!(
        register_time.timestamp(),
        registers[0].arrival_date.timestamp()
    );
    for (index, response_item) in registers[0].items.iter().enumerate() {
        assert_eq!(response_item.item_code_ext, registered[index].0);
        assert_eq!(response_item.count, registered[index].1);
    }
    app.cleanup().await;
}
