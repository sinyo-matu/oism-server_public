use crate::helpers::spawn_app;

#[tokio::test]
async fn signup_login_refresh_works() {
    let app = spawn_app().await;
    let public_base_uri = format!("{}/api/v1/public", app.address);
    app.signup_and_login().await;
    // Act 3 check access token refresh success
    let response3 = app
        .request_client
        .get(format!("{public_base_uri}/refresh_token"))
        .send()
        .await
        .expect("Failed to execute request");
    assert!(response3.status().is_success());
    assert!(response3.cookies().any(|c| c.name() == "smt_token"));
    assert!(response3.cookies().any(|c| c.name() == "smt_id"));
    app.cleanup().await;
}

#[tokio::test]
async fn signup_failed() {
    let app = spawn_app().await;
    let body = serde_json::json!(
        {
            "username":"random-username",
            "password":"random-password",
            "role":"full",
            "sub_role":"{}",
        }
    );
    let public_base_uri = format!("{}/api/v1/public", app.address);
    let response1 = app
        .request_client
        .post(format!("{public_base_uri}/signup"))
        .json(&body)
        .send()
        .await
        .expect("Failed execute request");
    assert_eq!(422, response1.status().as_u16());
    let body = serde_json::json!(
        {
            "username":"random-username",
            "password":"random-password",
            "role":"full",
            "sub_role":"{}",
            "secret":"invalid-secret"
        }
    );
    let public_base_uri = format!("{}/api/v1/public", app.address);
    let response2 = app
        .request_client
        .post(format!("{public_base_uri}/signup"))
        .json(&body)
        .send()
        .await
        .expect("Failed execute request");
    assert_eq!(400, response2.status().as_u16());
    assert_eq!("invalid signup secret", &response2.text().await.unwrap());
    let (username, password) = app.signup_test_user().await;
    let body = serde_json::json!(
        {
            "username":username,
            "password":password,
            "role":"full",
            "sub_role":"{}",
            "secret":"eliamo_daidaidai"
        }
    );
    let response3 = app
        .request_client
        .post(format!("{public_base_uri}/signup"))
        .json(&body)
        .send()
        .await
        .expect("Failed execute request");
    assert_eq!(400, response3.status().as_u16());
    assert_eq!("username is occupied", &response3.text().await.unwrap());
}

#[tokio::test]
async fn login_failed() {
    let app = spawn_app().await;
    let (username, _) = app.signup_test_user().await;
    let public_base_uri = format!("{}/api/v1/public", app.address);
    let body = serde_json::json!(
        {
            "username":"invalid-username",
            "password":"invalid-password"
        }
    );
    let response = app
        .request_client
        .post(format!("{public_base_uri}/login"))
        .json(&body)
        .send()
        .await
        .expect("Failed to send request");
    assert_eq!(401, response.status().as_u16());
    let body = serde_json::json!(
        {
            "username":username,
            "password":"invalid-password"
        }
    );
    let response = app
        .request_client
        .post(format!("{public_base_uri}/login"))
        .json(&body)
        .send()
        .await
        .expect("Failed to send request");
    assert_eq!(401, response.status().as_u16());
    app.cleanup().await;
}

#[tokio::test]
async fn private_health_check_works() {
    let app = spawn_app().await;
    let private_base_uri = format!("{}/api/v1/private", app.address);
    app.signup_and_login().await;
    let response = app
        .request_client
        .get(format!("{private_base_uri}/health_check"))
        .send()
        .await
        .expect("Failed to execute request");
    assert!(response.status().is_success())
}
