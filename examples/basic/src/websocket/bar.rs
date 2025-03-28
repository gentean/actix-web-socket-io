use std::time::Duration;

use async_trait::async_trait;

use actix_web::{
    get,
    web::{self, Payload},
    HttpRequest, Responder, Scope,
};
use actix_web_socket_io::{session::Emiter, Listener, MessageHandle, SocketIOResult};
use chrono::Utc;
use uuid::Uuid;

use crate::infra::socket;

#[get("/socket.io")]
async fn listen_system(req: HttpRequest, stream: Payload) -> impl Responder {
    // 创建 socket 连接
    let SocketIOResult {
        http_response,
        session_receive,
        session_id,
    } = socket::socket_io().connect(&req, stream);

    // 订阅建立连接
    session_receive
        .on(Listener {
            event_name: "connect".into(),
            handler: Box::new(SocketConnected),
        })
        .await;

    // 订阅断开连接
    session_receive
        .on(Listener {
            event_name: "disconnect".into(),
            handler: Box::new(SocketDisConnected),
        })
        .await;

    // 主动推送 任务一
    actix_web::rt::spawn(async move {
        let socket_server = socket::get_server();

        loop {
            // 每 1000ms 刷出一次数据
            actix_web::rt::time::sleep(Duration::from_millis(1000)).await;

            // 刷出系统时间
            if let Err(msg) = socket_server
                .emit(
                    Emiter {
                        event_name: "/system/timestamp".into(),
                        data: Utc::now().timestamp_millis(),
                    },
                    Some(session_id),
                )
                .await
            {
                log::error!("系统的时间刷出失败, msg: {}", msg);
            }
        }
    });

    // 主动推送 任务二
    actix_web::rt::spawn(async move {
        let socket_server = socket::get_server();
        loop {
            // 每 10s 刷出一次数据
            actix_web::rt::time::sleep(Duration::from_secs(10)).await;

            let _ = socket_server
                .emit(
                    Emiter {
                        event_name: "/hello".into(),
                        data: "world",
                    },
                    Some(session_id),
                )
                .await;
        }
    });

    http_response
}


pub struct SocketConnected;
#[async_trait]
impl MessageHandle for SocketConnected {
    async fn handler(&self, _: serde_json::Value, session_id: Uuid) {
        log::info!("有客户端建立连接成功，session_id={session_id}");
        // 做一些初始化任务
    }
}
pub struct SocketDisConnected;
#[async_trait]
impl MessageHandle for SocketDisConnected {
    async fn handler(&self, _: serde_json::Value, session_id: Uuid) {
        log::info!("有客户端断开连接成功，session_id={session_id}");
        // 做一些资源回收任务
    }
}

/// 注册 API 接口到 /bar 路径下      
pub fn add_route() -> Scope {
    web::scope("/bar").service(listen_system)
}
