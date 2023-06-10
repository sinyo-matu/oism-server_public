use serde::Serialize;
use std::sync::Arc;
use tracing::{error, instrument};
use uuid::Uuid;

use crate::server::auth::SETTINGS;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InsertTaskRequestBody {
    user_ex_id: uuid::Uuid,
    task: NotifyTask,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NotifyTask {
    list_name: String,
    title: String,
    notes: String,
}
#[derive(Debug)]
pub struct GoogleService {
    http_client: Arc<reqwest::Client>,
}

impl Default for GoogleService {
    fn default() -> Self {
        Self {
            http_client: Arc::new(reqwest::Client::new()),
        }
    }
}

impl GoogleService {
    #[instrument(name = "call outdated order notify", skip(self, user_ex_id, list_name))]
    pub async fn call_notify(
        &self,
        user_ex_id: Uuid,
        list_name: String,
        title: String,
        notes: String,
    ) {
        let http_client = self.http_client.clone();
        tokio::task::spawn(async move {
            let notify_task = NotifyTask {
                list_name,
                title,
                notes,
            };
            let body = InsertTaskRequestBody {
                user_ex_id,
                task: notify_task,
            };
            let res = http_client
                .post(format!(
                    "{}/google/insert_task",
                    SETTINGS.google_service.get_service_url()
                ))
                .json(&body)
                .send()
                .await;
            if res.is_err() {
                error!("http error:{:?}", res.unwrap());
                return;
            }
            let resp = res.unwrap();
            if resp.status().as_u16() >= 400 {
                let err = resp.text().await.unwrap();
                error!("http got bad response error: {}", err)
            }
        });
    }
}
