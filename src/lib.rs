use actix_web::{
    web::{Bytes, Payload},
    HttpRequest, HttpResponse,
};
use async_trait::async_trait;
use serde::Serialize;
use serde_json::Value;
use session::{
    encode_binary_event, encode_connect_success, encode_event, send_binary, send_text,
    BinaryEmiter, Emiter, Session, SessionStore,
};
use socketio::{buffer_to_value, replace_placeholders, EventData, MessageType};
use std::{collections::HashMap, sync::Arc};
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

        let mut receiver = session.get_receiver();

        // 收到事件统一处理
        let inner_receive = session_receive.clone();
        actix_web::rt::spawn(async move {
            while let Ok(message_data) = receiver.recv().await {
                inner_receive.handle_receive_msg(message_data).await;
            }
        });

        let session_id = session.id;
        let http_response = match actix_ws::handle(req, stream) {
            Ok((response, ws_session, msg_stream)) => {
                session.start(ws_session, msg_stream);
                Ok(response)
            }
            Err(err) => Err(err),
        };

        SocketIOResult {
            session_id,
            http_response,
            session_receive,
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

struct PendingBinary {
    remaining: usize,
    payload: Value,
    buffers: Vec<Bytes>,
}

///
/// 数据接收对象
///
pub struct SessionReceive {
    session_id: Uuid,
    // 服务端监听的事件总线：事件名 -> 处理方法集
    listeners: RwLock<HashMap<String, Vec<Box<dyn MessageHandle>>>>,
    pending_binary: RwLock<Option<PendingBinary>>,
    socket_server: Arc<SocketServer>,
}

impl SessionReceive {
    pub fn new(session_id: Uuid, socket_server: Arc<SocketServer>) -> Self {
        Self {
            session_id,
            listeners: RwLock::new(HashMap::new()),
            pending_binary: RwLock::new(None),
            socket_server,
        }
    }

    /// 接收到客户端发来的事件
    async fn handle_receive_msg(&self, message_type: MessageType) {
        match message_type {
            MessageType::Connect => self.accept_connect().await,
            MessageType::Event(message_data) => self.handler_trigger_on(message_data).await,
            MessageType::BinaryEvent {
                attachments,
                payload,
            } => self.begin_binary_event(attachments, payload).await,
            MessageType::Binary(data_bin) => self.handle_receive_binary_msg(data_bin).await,
            MessageType::None => (),
        }
    }

    /// 同意建立连接
    async fn accept_connect(&self) {
        let mut session = {
            let session_store = self.socket_server.session_store.read().await;
            session_store.sessions.get(&self.session_id).cloned()
        };
        if let Some(session) = session.as_mut() {
            if let Ok(text) = encode_connect_success(&HashMap::from([("sid", "accept")])) {
                let _ = send_text(session, text).await;
            }
        }

        self.handler_trigger_on(EventData("connect".into(), Value::Null))
            .await;
    }

    /// 触发事件
    async fn handler_trigger_on(&self, event: EventData) {
        let listeners = self.listeners.read().await;
        if let Some(handlers) = listeners.get(&event.0) {
            for handler in handlers {
                handler.handler(event.1.clone(), self.session_id).await;
            }
        }
    }

    async fn begin_binary_event(&self, attachments: usize, payload: Value) {
        if attachments == 0 {
            self.dispatch_binary_payload(payload).await;
            return;
        }

        *self.pending_binary.write().await = Some(PendingBinary {
            remaining: attachments,
            payload,
            buffers: Vec::with_capacity(attachments),
        });
    }

    /// 处理二进制附件：拼到当前 BINARY_EVENT，收齐后触发对应事件
    pub async fn handle_receive_binary_msg(&self, data_bin: Bytes) {
        let completed = {
            let mut pending = self.pending_binary.write().await;
            let Some(current) = pending.as_mut() else {
                drop(pending);
                self.handler_trigger_on(EventData("binary".into(), buffer_to_value(&data_bin)))
                    .await;
                return;
            };

            current.buffers.push(data_bin);
            current.remaining = current.remaining.saturating_sub(1);
            if current.remaining == 0 {
                pending.take()
            } else {
                None
            }
        };

        if let Some(mut packet) = completed {
            replace_placeholders(&mut packet.payload, &packet.buffers);
            self.dispatch_binary_payload(packet.payload).await;
        }
    }

    async fn dispatch_binary_payload(&self, payload: Value) {
        let event = match payload {
            Value::Array(mut items) if !items.is_empty() => {
                let event_name = items.remove(0).as_str().unwrap_or("binary").to_string();
                let data = match items.len() {
                    0 => Value::Null,
                    1 => items.remove(0),
                    _ => Value::Array(items),
                };
                EventData(event_name, data)
            }
            other => EventData("binary".into(), other),
        };

        self.handler_trigger_on(event).await;
    }

    /// 监听客户端推来的事件
    pub async fn on(&self, listener: Listener) {
        self.listeners
            .write()
            .await
            .entry(listener.event_name)
            .or_default()
            .push(listener.handler);
    }
}

impl SocketServer {
    pub fn new() -> Self {
        Self {
            session_store: Arc::new(RwLock::new(SessionStore::new())),
        }
    }

    async fn collect_sessions(&self, session_id: Option<Uuid>) -> Vec<actix_ws::Session> {
        let store = self.session_store.read().await;
        if let Some(session_id) = session_id {
            store
                .sessions
                .get(&session_id)
                .cloned()
                .into_iter()
                .collect()
        } else {
            store.sessions.values().cloned().collect()
        }
    }

    /// 发送 JSON 事件给客户端
    pub async fn emit<D: Serialize + Send + 'static + Sync>(
        &self,
        emiter: Emiter<D>,
        session_id: Option<Uuid>,
    ) -> Result<(), String> {
        let text = encode_event(&emiter.event_name, &emiter.data).map_err(|err| err.to_string())?;
        let sessions = self.collect_sessions(session_id).await;
        let single = session_id.is_some();

        for mut session in sessions {
            let result = send_text(&mut session, text.clone()).await;
            if single {
                result.map_err(|err| err.to_string())?;
            }
        }

        Ok(())
    }

    /// 发送二进制事件给客户端
    pub async fn emit_binary<D: Serialize + Send + Sync>(
        &self,
        emiter: BinaryEmiter<D>,
        session_id: Option<Uuid>,
    ) -> Result<(), String> {
        if emiter.buffers.is_empty() {
            return Err("buffers 不能为空".into());
        }

        let header = encode_binary_event(
            &emiter.event_name,
            emiter.data.as_ref(),
            emiter.buffers.len(),
        )
        .map_err(|err| err.to_string())?;
        let sessions = self.collect_sessions(session_id).await;
        let single = session_id.is_some();

        for mut session in sessions {
            let result = async {
                send_text(&mut session, header.clone()).await?;
                for buffer in &emiter.buffers {
                    send_binary(&mut session, buffer).await?;
                }
                Ok::<(), &'static str>(())
            }
            .await;

            if single {
                result.map_err(|err| err.to_string())?;
            }
        }

        Ok(())
    }
}
