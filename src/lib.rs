use actix_web::{
    web::{Bytes, Payload},
    HttpRequest, HttpResponse,
};
use actix_web_actors::ws::{self};
use async_trait::async_trait;
use crossbeam::channel::TryRecvError;
use serde::Serialize;
use serde_json::Value;
use session::{Emiter, Session, SessionStore};
use socketio::{EventData, MessageType};
use std::{sync::Arc, time::Duration};
use tokio::sync::RwLock;
use uuid::Uuid;

pub mod session;
pub mod socketio;

pub struct SocketIO {
    pub socket_server: Arc<SocketServer>,
    pub socket_config: Arc<SocketConfig>,
}

pub struct SocketIOResult {
    pub http_response: Result<HttpResponse, actix_web::error::Error>,
    pub session_receive: Arc<SessionReceive>,
    pub session_id: Uuid,
}

#[derive(Clone)]
pub struct SocketConfig {
    // 心跳间隔(毫秒), 默认 25000
    pub ping_interval: u64,
    // 心跳超时(毫秒), 默认 20000
    pub ping_timeout: u64,
    // 每个块的最大字节数, 默认 1000000 Byte
    pub max_payload: usize,
}

impl Default for SocketConfig {
    fn default() -> Self {
        Self {
            ping_interval: 25000,
            ping_timeout: 20000,
            max_payload: 1000000,
        }
    }
}

impl SocketIO {
    pub fn new() -> Self {
        Self {
            socket_config: Arc::new(SocketConfig::default()),
            socket_server: Arc::new(SocketServer::new()),
        }
    }

    pub fn config(&mut self, socket_config: SocketConfig) -> &mut Self {
        self.socket_config = Arc::new(socket_config);

        self
    }

    /// 建立连接
    pub fn connect(&self, req: &HttpRequest, stream: Payload) -> SocketIOResult {
        let session_store = self.socket_server.session_store.clone();
        // 创建一个新会话
        let session = Session::new(self.socket_config.clone(), session_store);

        let session_receive = Arc::new(SessionReceive::new(session.id, self.socket_server.clone()));

        let receiver = session.get_receiver();

        // 收到事件统一处理
        let inner_receive = session_receive.clone();
        actix_web::rt::spawn(async move {
            loop {
                match receiver.try_recv() {
                    Ok(message_data) => {
                        inner_receive.handle_receive_msg(message_data).await;
                    }
                    Err(TryRecvError::Disconnected) => {
                        break;
                    }
                    Err(TryRecvError::Empty) => {
                        actix_web::rt::time::sleep(Duration::from_millis(20)).await;
                    }
                }
            }
        });

        SocketIOResult {
            session_id: session.id,
            http_response: ws::start(session, req, stream),
            session_receive: session_receive.clone(),
        }
    }
}

#[async_trait]
pub trait MessageHandle: Sync + Send + 'static {
    async fn handler(&self, data: Value, session_id: Uuid);
}

/// 监听客户端
pub struct Listener {
    pub event_name: String,
    pub handler: Box<dyn MessageHandle>,
}

pub struct SocketServer {
    pub session_store: Arc<RwLock<SessionStore>>,
}

///
/// 数据接收对象
///
pub struct SessionReceive {
    session_id: Uuid,
    // 服务端监听的事件总线
    listeners: RwLock<Vec<Listener>>,
    socket_server: Arc<SocketServer>,
}

impl SessionReceive {
    pub fn new(session_id: Uuid, socket_server: Arc<SocketServer>) -> Self {
        Self {
            session_id,
            listeners: RwLock::new(vec![]),
            socket_server,
        }
    }

    /// 接收到客户端发来的事件
    async fn handle_receive_msg(&self, message_type: MessageType) {
        match message_type {
            MessageType::Event(message_data) => self.handler_trigger_on(message_data).await,
            MessageType::None => (),
        }
    }

    /// 触发事件
    async fn handler_trigger_on(&self, event: EventData) {
        let listeners = self.listeners.read().await;
        for listener in listeners.iter() {
            // 按事件名匹配
            if listener.event_name.eq(&event.0) {
                listener
                    .handler
                    .handler(event.1.clone(), self.session_id)
                    .await;
            }
        }
    }

    /// 处理二进制数据
    pub fn handle_receive_binary_msg(&mut self, _data_bin: Bytes) {
        // 触发监听
    }

    /// 监听客户端推来的事件
    pub async fn on(&self, listener: Listener) {
        self.listeners.write().await.push(listener);
    }
}

impl SocketServer {
    pub fn new() -> Self {
        Self {
            session_store: Arc::new(RwLock::new(SessionStore::new())),
        }
    }

    /// 发送事件给客户端
    pub async fn emit<D: Serialize + Send + 'static + Sync>(
        &self,
        emiter: Emiter<D>,
        session_id: Option<Uuid>,
    ) -> Result<(), String> {
        let emiter = Arc::new(emiter);
        if let Some(session_id) = session_id {
            if let Some(session) = self.session_store.read().await.sessions.get(&session_id) {
                session.do_send(emiter.clone());
            }
        } else {
            for session in self.session_store.read().await.sessions.values() {
                session.do_send(emiter.clone());
            }
        }

        Ok(())
    }
}
