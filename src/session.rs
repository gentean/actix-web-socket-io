use actix_web::web::Bytes;
use actix_ws::{Message, MessageStream, Session as WsSession};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};
use tokio::sync::{
    broadcast::{self, Receiver, Sender},
    RwLock,
};
use uuid::Uuid;

use crate::{
    socketio::{
        parse_binary_event_payload, EngineIOPacketType, EventData, MessageType, OpenPacket,
        SocketIOPacketType,
    },
    SocketConfig,
};

/// 会话，每创建一个连接，生成一个会话
pub struct Session {
    pub id: Uuid,
    session_store: Arc<RwLock<SessionStore>>,
    sender: Sender<MessageType>,
    pub heartbeat: Arc<AtomicBool>,
    socket_config: Arc<SocketConfig>,
}

impl Session {
    pub fn new(socket_config: Arc<SocketConfig>, session_store: Arc<RwLock<SessionStore>>) -> Self {
        let (sender, _) = broadcast::channel::<MessageType>(1024);
        Self {
            id: Uuid::new_v4(),
            session_store,
            sender,
            heartbeat: Arc::new(AtomicBool::new(true)),
            socket_config,
        }
    }

    /// 注册消息处理逻辑
    pub fn get_receiver(&self) -> Receiver<MessageType> {
        self.sender.subscribe()
    }

    /// 启动 WebSocket 读写循环（替代原先的 Actor）
    pub fn start(self, ws_session: WsSession, msg_stream: MessageStream) {
        actix_web::rt::spawn(async move {
            self.run(ws_session, msg_stream).await;
        });
    }

    async fn run(self, mut ws_session: WsSession, mut msg_stream: MessageStream) {
        self.session_store
            .write()
            .await
            .sessions
            .insert(self.id, ws_session.clone());

        if send_open_packet(&mut ws_session, &self).await.is_err() {
            self.cleanup().await;
            return;
        }

        let heartbeat_task = spawn_heartbeat(
            ws_session.clone(),
            self.heartbeat.clone(),
            self.socket_config.ping_interval,
            self.socket_config.ping_timeout,
        );

        while let Some(item) = msg_stream.next().await {
            let msg = match item {
                Ok(msg) => msg,
                Err(_) => break,
            };

            match msg {
                Message::Text(byte_string) => {
                    self.handle_text(&byte_string);
                }
                Message::Ping(bytes) => {
                    if ws_session.pong(&bytes).await.is_err() {
                        break;
                    }
                }
                Message::Close(reason) => {
                    let _ = ws_session.clone().close(reason).await;
                    break;
                }
                Message::Binary(bytes) => {
                    self.handle_binary(bytes);
                }
                _ => {}
            }
        }

        heartbeat_task.abort();
        self.cleanup().await;
        let _ = ws_session.close(None).await;
    }

    fn handle_text(&self, raw: &str) {
        let data_str = raw.get(2..);

        let eg_type = raw
            .get(0..1)
            .and_then(|f| f.parse::<u8>().ok())
            .and_then(|f| EngineIOPacketType::try_from(f).ok());

        let sc_type = raw
            .get(1..2)
            .and_then(|f| f.parse::<u8>().ok())
            .and_then(|f| SocketIOPacketType::try_from(f).ok());

        let Some(eg_type) = eg_type else {
            return;
        };

        match eg_type {
            EngineIOPacketType::Pong => {
                self.heartbeat.store(true, Ordering::Relaxed);
            }
            EngineIOPacketType::Message => {
                if let (Some(sc_type), Some(data_str)) = (sc_type, data_str) {
                    let message_type = match sc_type {
                        SocketIOPacketType::Connect => MessageType::Connect,
                        SocketIOPacketType::Disconnect => MessageType::None,
                        SocketIOPacketType::Event => serde_json::from_str::<EventData>(data_str)
                            .map_or(MessageType::None, MessageType::Event),
                        SocketIOPacketType::BinaryEvent => parse_binary_event_payload(data_str)
                            .map(|(attachments, payload)| MessageType::BinaryEvent {
                                attachments,
                                payload,
                            })
                            .unwrap_or(MessageType::None),
                        SocketIOPacketType::Ack
                        | SocketIOPacketType::ConnectError
                        | SocketIOPacketType::BinaryAck => MessageType::None,
                    };

                    if let Err(err) = self.sender.send(message_type) {
                        log::error!("socket-io 发送数据失败{err:?}");
                    }
                }
            }
            EngineIOPacketType::Open
            | EngineIOPacketType::Close
            | EngineIOPacketType::Ping
            | EngineIOPacketType::Upgrade
            | EngineIOPacketType::Noop => {}
        }
    }

    fn handle_binary(&self, bytes: Bytes) {
        let message_type = match bytes
            .first()
            .copied()
            .and_then(|b| EngineIOPacketType::try_from(b).ok())
        {
            Some(EngineIOPacketType::Pong) => {
                self.heartbeat.store(true, Ordering::Relaxed);
                return;
            }
            Some(EngineIOPacketType::Message) => MessageType::Binary(bytes.slice(1..)),
            _ => MessageType::Binary(bytes),
        };

        if let Err(err) = self.sender.send(message_type) {
            log::error!("socket-io 发送二进制数据失败{err:?}");
        }
    }

    async fn cleanup(&self) {
        let _ = self.sender.send(MessageType::Event(EventData(
            "disconnect".to_string(),
            serde_json::Value::Null,
        )));

        self.session_store.write().await.sessions.remove(&self.id);
    }
}

async fn send_open_packet(ws_session: &mut WsSession, session: &Session) -> Result<(), ()> {
    let packet = OpenPacket {
        sid: session.id.to_string(),
        upgrades: vec![],
        ping_interval: session.socket_config.ping_interval,
        ping_timeout: session.socket_config.ping_timeout,
        max_payload: session.socket_config.max_payload,
    };
    let json_str = serde_json::to_string(&packet).map_err(|_| ())?;
    ws_session
        .text(format!("{}{}", EngineIOPacketType::Open as u8, json_str))
        .await
        .map_err(|_| ())
}

fn spawn_heartbeat(
    mut ws_session: WsSession,
    heartbeat: Arc<AtomicBool>,
    ping_interval: u64,
    ping_timeout: u64,
) -> tokio::task::JoinHandle<()> {
    actix_web::rt::spawn(async move {
        loop {
            actix_web::rt::time::sleep(Duration::from_millis(ping_interval)).await;

            if ws_session
                .text((EngineIOPacketType::Ping as u8).to_string())
                .await
                .is_err()
            {
                break;
            }

            heartbeat.store(false, Ordering::Relaxed);
            actix_web::rt::time::sleep(Duration::from_millis(ping_timeout)).await;

            if !heartbeat.load(Ordering::Relaxed) {
                let _ = ws_session.close(None).await;
                break;
            }
        }
    })
}

pub(crate) async fn send_text(session: &mut WsSession, text: String) -> Result<(), &'static str> {
    session.text(text).await.map_err(|_| "session closed")
}

pub(crate) async fn send_binary(
    session: &mut WsSession,
    payload: impl AsRef<[u8]>,
) -> Result<(), &'static str> {
    let payload = payload.as_ref();
    let mut frame = Vec::with_capacity(1 + payload.len());
    frame.push(EngineIOPacketType::Message as u8);
    frame.extend_from_slice(payload);
    session.binary(frame).await.map_err(|_| "session closed")
}

pub(crate) fn encode_connect_success<T: Serialize>(data: &T) -> Result<String, &'static str> {
    let json_str = serde_json::to_string(data).map_err(|_| "json 序列化失败")?;
    Ok(format!(
        "{}{}{}",
        EngineIOPacketType::Message as u8,
        SocketIOPacketType::Connect as u8,
        json_str
    ))
}

pub(crate) fn encode_event<T: Serialize>(
    event_name: &str,
    data: &T,
) -> Result<String, &'static str> {
    let json_str = serde_json::to_string(data).map_err(|_| "json 序列化失败")?;
    Ok(format!(
        "{}{}[\"{}\",{}]",
        EngineIOPacketType::Message as u8,
        SocketIOPacketType::Event as u8,
        event_name,
        json_str
    ))
}

fn placeholder(num: usize) -> serde_json::Value {
    serde_json::json!({
        "_placeholder": true,
        "num": num,
    })
}

/// 编码 BINARY_EVENT 文本头：`45N-["event", <data?>, {placeholder...}]`
pub(crate) fn encode_binary_event<T: Serialize>(
    event_name: &str,
    data: Option<&T>,
    attachment_count: usize,
) -> Result<String, &'static str> {
    let mut args = Vec::with_capacity(attachment_count + 2);
    args.push(serde_json::Value::String(event_name.to_string()));
    if let Some(data) = data {
        args.push(serde_json::to_value(data).map_err(|_| "json 序列化失败")?);
    }
    for i in 0..attachment_count {
        args.push(placeholder(i));
    }
    let json_str = serde_json::to_string(&args).map_err(|_| "json 序列化失败")?;
    Ok(format!(
        "{}{}{}-{}",
        EngineIOPacketType::Message as u8,
        SocketIOPacketType::BinaryEvent as u8,
        attachment_count,
        json_str
    ))
}

/// 建立连接 header 头
#[allow(dead_code)]
#[derive(Serialize, Deserialize, Clone)]
struct Header {
    sid: Option<String>,
    token: Option<String>,
}

/// 建立连接结构体
#[allow(dead_code)]
pub struct ConnectPacket {
    r#type: SocketIOPacketType,
    data: Header,
}

/// 鉴权响应数据
#[allow(dead_code)]
#[derive(Serialize)]
pub struct AuthSuccess<T: Serialize> {
    pub data: T,
}

/// 发送客户端
pub struct Emiter<T: Serialize> {
    pub event_name: String,
    pub data: T,
}

/// 发送二进制给客户端
pub struct BinaryEmiter<T: Serialize = serde_json::Value> {
    pub event_name: String,
    /// 跟在事件名后的 JSON 参数，没有则填 `None`
    pub data: Option<T>,
    pub buffers: Vec<Bytes>,
}

impl BinaryEmiter {
    pub fn new(event_name: impl Into<String>, buffers: Vec<Bytes>) -> Self {
        Self {
            event_name: event_name.into(),
            data: None,
            buffers,
        }
    }
}

impl<T: Serialize> BinaryEmiter<T> {
    pub fn with_data(event_name: impl Into<String>, data: T, buffers: Vec<Bytes>) -> Self {
        Self {
            event_name: event_name.into(),
            data: Some(data),
            buffers,
        }
    }
}

/// 存储所有客户端会话的 store
pub struct SessionStore {
    // 存储的客户端会话
    pub sessions: HashMap<Uuid, WsSession>,
}
impl SessionStore {
    pub fn new() -> Self {
        Self {
            sessions: HashMap::new(),
        }
    }
}
