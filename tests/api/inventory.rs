use oism_server::db::{inventory::InventoryLocation, InventoryOutput};

use crate::helpers::spawn_app;

#[tokio::test]
async fn query_inventory_works() {
    let app = spawn_app().await;
    app.signup_and_login().await;
    let (mut registered, _) = app.register_inventory().await;
    let private_base_uri = app.private_base_uri();
    let query = vec![
        ("keyword", ""),
        ("category", ""),
        ("showZeroQuantity", "true"),
    ];
    let response1 = app
        .request_client
        .get(format!("{private_base_uri}/inventory"))
        .query(&query)
        .send()
        .await
        .expect("Failed to request");
    assert!(response1.status().is_success());
    let mut inventory: Vec<InventoryOutput> =
        response1.json().await.expect("Failed to deserialize json");
    inventory.sort_by(|a, b| a.item_code_ext.cmp(&b.item_code_ext));
    registered.sort_by(|a, b| a.0.cmp(&b.0));
    for (index, inventory_item) in inventory.iter().enumerate() {
        assert_eq!(inventory_item.item_code_ext, registered[index].0);
        assert_eq!(inventory_item.quantity[0].location, InventoryLocation::JP);
        assert_eq!(inventory_item.quantity[0].quantity, registered[index].1);
    }
    app.cleanup().await;
}
