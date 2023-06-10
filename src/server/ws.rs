use crate::error_result::Result;
use std::{sync::Arc, time::Duration};

use axum::{
    extract::{
        ws::{Message, WebSocket},
        State, WebSocketUpgrade,
    },
    response::IntoResponse,
};
use futures::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::sync::broadcast::Sender;
use uuid::Uuid;

#[derive(Clone)]
pub enum ControlMessage {
    RefreshOrderList,
    RefreshInventory,
    Pong,
    Ping,
    RefreshOrderItem(Uuid),
    RefreshShipmentList,
    RefreshRegisterList,
    RefreshReturnList,
    RefreshTransferList,
    RefreshInventoryItemQuantity,
    RefreshWaitForShipmentItemList,
    RefreshNewShipmentBucket(Uuid),
    RefreshShipmentItem(Uuid),
}

pub async fn handle_ws(
    ws: WebSocketUpgrade,
    State(orders_sender): State<Arc<Sender<ControlMessage>>>,
) -> Result<impl IntoResponse> {
    Ok(ws.on_upgrade(|socket| handle_subscribe_change(socket, orders_sender)))
}
#[derive(Serialize, Deserialize)]
struct WsMsg {
    event: WsEvent,
    message: String,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
enum WsEvent {
    RefreshOrderList,
    RefreshInventory,
    Ping,
    Pong,
    RefreshOrderItem,
    RefreshShipmentList,
    RefreshShipmentItem,
    RefreshRegisterList,
    RefreshReturnList,
    RefreshTransferList,
    RefreshInventoryItemQuantity,
    RefreshWaitForShipmentItemList,
    RefreshNewShipmentBucket,
}

pub async fn handle_subscribe_change(stream: WebSocket, sender: Arc<Sender<ControlMessage>>) {
    let mut rx = sender.subscribe();
    let cloned_sender = sender.clone();
    let (mut ws_sender, mut ws_receiver) = stream.split();
    let mut recv_task = tokio::spawn(async move {
        while let Some(Ok(Message::Text(text))) = ws_receiver.next().await {
            // Add username before message.
            let msg = serde_json::from_str::<WsMsg>(&text).unwrap();
            if let WsEvent::Ping = msg.event {
                if cloned_sender.send(ControlMessage::Pong).is_err() {
                    break;
                }
            };
        }
    });
    let mut ping_task = tokio::spawn(async move {
        while sender.send(ControlMessage::Ping).is_ok() {
            tokio::time::sleep(Duration::from_secs(20)).await;
        }
    });
    let mut send_task = tokio::spawn(async move {
        while let Ok(message) = rx.recv().await {
            tokio::time::sleep(Duration::from_millis(10)).await;
            match message {
                ControlMessage::Ping => {
                    if ws_sender
                        .send(Message::Text(
                            json!(WsMsg {
                                event: WsEvent::Ping,
                                message: String::from(""),
                            })
                            .to_string(),
                        ))
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
                ControlMessage::RefreshOrderList => {
                    if ws_sender
                        .send(Message::Text(
                            json!(WsMsg {
                                event: WsEvent::RefreshOrderList,
                                message: String::from(""),
                            })
                            .to_string(),
                        ))
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
                ControlMessage::RefreshInventory => {
                    if ws_sender
                        .send(Message::Text(
                            json!(WsMsg {
                                event: WsEvent::RefreshInventory,
                                message: String::from(""),
                            })
                            .to_string(),
                        ))
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
                ControlMessage::Pong => {
                    if ws_sender
                        .send(Message::Text(
                            json!(WsMsg {
                                event: WsEvent::Pong,
                                message: String::from(""),
                            })
                            .to_string(),
                        ))
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
                ControlMessage::RefreshOrderItem(id) => {
                    if ws_sender
                        .send(Message::Text(
                            json!(WsMsg {
                                event: WsEvent::RefreshOrderItem,
                                message: id.to_string(),
                            })
                            .to_string(),
                        ))
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
                ControlMessage::RefreshShipmentList => {
                    if ws_sender
                        .send(Message::Text(
                            json!(WsMsg {
                                event: WsEvent::RefreshShipmentList,
                                message: String::from(""),
                            })
                            .to_string(),
                        ))
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
                ControlMessage::RefreshShipmentItem(id) => {
                    if ws_sender
                        .send(Message::Text(
                            json!(WsMsg {
                                event: WsEvent::RefreshShipmentItem,
                                message: id.to_string(),
                            })
                            .to_string(),
                        ))
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
                ControlMessage::RefreshRegisterList => {
                    if ws_sender
                        .send(Message::Text(
                            json!(WsMsg {
                                event: WsEvent::RefreshRegisterList,
                                message: String::from(""),
                            })
                            .to_string(),
                        ))
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
                ControlMessage::RefreshReturnList => {
                    if ws_sender
                        .send(Message::Text(
                            json!(WsMsg {
                                event: WsEvent::RefreshReturnList,
                                message: String::from(""),
                            })
                            .to_string(),
                        ))
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
                ControlMessage::RefreshTransferList => {
                    if ws_sender
                        .send(Message::Text(
                            json!(WsMsg {
                                event: WsEvent::RefreshTransferList,
                                message: String::from(""),
                            })
                            .to_string(),
                        ))
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
                ControlMessage::RefreshInventoryItemQuantity => {
                    if ws_sender
                        .send(Message::Text(
                            json!(WsMsg {
                                event: WsEvent::RefreshInventoryItemQuantity,
                                message: String::from(""),
                            })
                            .to_string(),
                        ))
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
                ControlMessage::RefreshWaitForShipmentItemList => {
                    if ws_sender
                        .send(Message::Text(
                            json!(WsMsg {
                                event: WsEvent::RefreshWaitForShipmentItemList,
                                message: String::from(""),
                            })
                            .to_string(),
                        ))
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
                ControlMessage::RefreshNewShipmentBucket(id) => {
                    if ws_sender
                        .send(Message::Text(
                            json!(WsMsg {
                                event: WsEvent::RefreshNewShipmentBucket,
                                message: id.to_string(),
                            })
                            .to_string(),
                        ))
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
            }
        }
    });
    // If any one of the tasks exit, abort the others.
    tokio::select! {
        _ = (&mut ping_task) => {recv_task.abort();send_task.abort();},
        _ = (&mut send_task) => {ping_task.abort();recv_task.abort();},
        _ = (&mut recv_task) => {ping_task.abort();send_task.abort();},
    };
    println!("closing connection...");
}

#[inline]
pub fn send_control_message(sender: &Arc<Sender<ControlMessage>>, message: ControlMessage) {
    if sender.receiver_count() != 0 && sender.send(message).is_err() {
        println!("no receiver")
    };
}

#[inline]
pub fn send_control_messages(sender: Arc<Sender<ControlMessage>>, messages: &[ControlMessage]) {
    for message in messages {
        if sender.receiver_count() != 0 && sender.send(message.to_owned()).is_err() {
            println!("no receiver")
        };
    }
}
