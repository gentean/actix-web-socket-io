use std::{collections::HashMap, sync::Arc, time::Duration};

use async_trait::async_trait;
use lazy_static::lazy_static;

use actix_web::{
    get,
    web::{self, Payload},
    HttpRequest, Responder, Scope,
};
use actix_web_socket_io::{session::Emiter, Listener, MessageHandle, SocketIOResult};
use serde::Deserialize;
use tokio::sync::RwLock;
use uuid::Uuid;

use crate::{
    infra::socket,
    service::realtime::{watch_change, ValData},
    validate::SocketAuth,
};

struct SessionData {
    un_watcher: Box<(dyn FnOnce() + Sync + Send)>,
    value_cache: RwLock<HashMap<i32, ValData>>,
}

lazy_static! {
    // 所有连接的数据变更缓存数据
    static ref DATA_CACHE: RwLock<HashMap<Uuid, SessionData>> = RwLock::new(HashMap::new());
}

#[get("/socket.io")]
async fn listen_device(req: HttpRequest, stream: Payload) -> impl Responder {
    let SocketIOResult {
        http_response,
        session_receive,
        session_id,
    } = socket::socket_io().connect(&req, stream, SocketAuth);

    session_receive
        .on(Listener {
            event_name: "connect".into(),
            handler: Box::new(SocketConnected),
        })
        .await;

    session_receive
        .on(Listener {
            event_name: "disconnect".into(),
            handler: Box::new(SocketDisConnected),
        })
        .await;

    session_receive
        .on(Listener {
            event_name: "/subscribe/data".into(),
            handler: Box::new(SubscribeData),
        })
        .await;

    session_receive
        .on(Listener {
            event_name: "/unsubscribe/data".into(),
            handler: Box::new(UnSubscribeData),
        })
        .await;

    // 刷出缓存区
    actix_web::rt::spawn(async move {
        let socket_server = socket::get_server();

        loop {
            // 每 500 ms 刷出一次数据
            actix_web::rt::time::sleep(Duration::from_millis(500)).await;

            let data_cache = DATA_CACHE.read().await;

            // 已经断开连接
            if data_cache.get(&session_id).is_none() {
                break;
            }

            // 刷出设备状态
            if let Some(session_data) = data_cache.get(&session_id) {
                let hash_map = session_data.value_cache.read().await;
                let result = hash_map.values().cloned().collect::<Vec<_>>();

                if result.len() > 0 {
                    let result = socket_server
                        .emit(
                            Emiter {
                                event_name: "/change/device_status".into(),
                                data: result,
                            },
                            Some(session_id),
                        )
                        .await;

                    if let Err(msg) = result {
                        log::error!("设备状态刷出失败, msg: {}", msg);
                    }
                }
            }
        }
    });

    http_response
}

/// 建立连接，分配缓存区
pub struct SocketConnected;
#[async_trait]
impl MessageHandle for SocketConnected {
    async fn handler(&self, _: serde_json::Value, session_id: Uuid) {}
}

/// 断开连接，回收缓存区
pub struct SocketDisConnected;
#[async_trait]
impl MessageHandle for SocketDisConnected {
    async fn handler(&self, _: serde_json::Value, session_id: Uuid) {
        let mut data_cache = DATA_CACHE.write().await;
        if let Some(session_data) = data_cache.remove(&session_id) {
            (session_data.un_watcher)();
        }
    }
}

#[derive(Debug, Deserialize)]
struct SubscribeDataDto {
    ids: Vec<i32>,
}
/// 遥测订阅逻辑
pub struct SubscribeData;
#[async_trait]
impl MessageHandle for SubscribeData {
    async fn handler(&self, data: serde_json::Value, session_id: Uuid) {
        if let Ok(subscribe_data_dto) = serde_json::from_value::<SubscribeDataDto>(data) {
            // 去订阅
            let (receiver, un_watch) = watch_change(subscribe_data_dto.ids).await;

            // 当前场景加入缓存区，释放锁
            {
                let mut data_cache = DATA_CACHE.write().await;
                data_cache.insert(
                    session_id,
                    SessionData {
                        un_watcher: Box::new(un_watch),
                        value_cache: RwLock::new(HashMap::new()),
                    },
                );
            }

            receiver_val(receiver, session_id).await;
        } else {
            log::info!("没有要监听 id");
        }
    }
}

async fn receiver_val(receiver: crossbeam::channel::Receiver<Arc<ValData>>, session_id: Uuid) {
    actix_web::rt::spawn(async move {
        loop {
            match receiver.try_recv() {
                Ok(val_data) => {
                    // 把点号推入缓存区
                    let data_cache = DATA_CACHE.read().await;
                    if let Some(session_data) = data_cache.get(&session_id) {
                        let mut value_cache = session_data.value_cache.write().await;
                        let val_data = val_data.as_ref();
                        value_cache.insert(val_data.id, val_data.clone());
                    }
                }
                Err(crossbeam::channel::TryRecvError::Disconnected) => {
                    return;
                }
                Err(crossbeam::channel::TryRecvError::Empty) => {
                    actix_web::rt::time::sleep(Duration::from_millis(20)).await;
                }
            }
        }
    });
}

/// 取消某个场景的点号订阅
pub struct UnSubscribeData;
#[async_trait]
impl MessageHandle for UnSubscribeData {
    async fn handler(&self, data: serde_json::Value, session_id: Uuid) {
        if let Ok(un_subscribe) = serde_json::from_value::<SubscribeDataDto>(data) {
            let data_cache = DATA_CACHE.write().await;
            if let Some(session_data) = data_cache.get(&session_id) {
                let mut hash_map = session_data.value_cache.write().await;
                for id in un_subscribe.ids {
                    hash_map.remove(&id);
                }
            }
        }
    }
}

/// 注册 API 接口到 /foo 路径下
pub fn add_route() -> Scope {
    web::scope("/foo").service(listen_device)
}
