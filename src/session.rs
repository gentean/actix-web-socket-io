use actix::prelude::*;

use actix_web_actors::ws::{self, Message};
use crossbeam::channel::{self, Receiver, Sender};
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, sync::Arc, time::Duration};
use tokio::sync::RwLock;
use uuid::Uuid;

use crate::{
    socketio::{
        ConnectSuccess, EngineIOPacketType, EventData, MessageType, OpenPacket, SocketIOPacketType,
    },
    SocketConfig,
};

/// 会话，每创建一个连接，生成一个会话
pub struct Session {
    pub id: Uuid,
    session_store: Arc<RwLock<SessionStore>>,
    sender: Sender<MessageType>,
    receiver: Receiver<MessageType>,
    pub heartbeat: bool,
    socket_config: Arc<SocketConfig>,
}

impl Session {
    pub fn new(socket_config: Arc<SocketConfig>, session_store: Arc<RwLock<SessionStore>>) -> Self {
        let (sender, receiver) = channel::unbounded::<MessageType>();
        Self {
            id: Uuid::new_v4(),
            session_store,
            sender,
            receiver,
            heartbeat: true,
            socket_config,
        }
    }

    /// 注册消息处理逻辑
    pub fn get_receiver(&self) -> Receiver<MessageType> {
        self.receiver.clone()
    }
}

impl Actor for Session {
    type Context = ws::WebsocketContext<Self>;

    /// 会话创建后
    fn started(&mut self, ctx: &mut Self::Context) {
        actix_web::rt::spawn({
            let session_store = self.session_store.clone();
            let id = self.id;
            let address = ctx.address();
            async move {
                session_store.write().await.sessions.insert(id, address);
            }
        });

        // 回应 engine.io
        let ping_interval = self.socket_config.ping_interval;
        let ping_timeout = self.socket_config.ping_timeout;
        ctx.address().do_send(OpenPacket {
            sid: self.id.to_string(),
            upgrades: vec![],
            ping_interval,
            ping_timeout,
            max_payload: self.socket_config.max_payload,
        });

        // 心跳
        ctx.run_interval(
            Duration::from_millis(ping_interval.into()),
            move |session, ctx| {
                // 发送 Ping
                ctx.text((EngineIOPacketType::Ping as u8).to_string());
                session.heartbeat = false;

                ctx.run_later(
                    Duration::from_millis(ping_timeout.into()),
                    |session, ctx| {
                        // 没有收到心跳回应，断开连接
                        if !session.heartbeat {
                            ctx.close(None);
                        }
                    },
                );
            },
        );
    }

    /// 会话将要断开时
    fn stopping(&mut self, _ctx: &mut Self::Context) -> Running {
        let _ = self.sender.send(MessageType::Event(EventData(
            "disconnect".to_string(),
            serde_json::Value::Null,
        )));

        actix_web::rt::spawn({
            let session_store = self.session_store.clone();
            let id = self.id;
            async move {
                session_store.write().await.sessions.remove(&id);
            }
        });
        Running::Stop
    }
}

impl<T: Serialize> Handler<ConnectSuccess<T>> for Session {
    type Result = Result<(), &'static str>;
    fn handle(&mut self, msg: ConnectSuccess<T>, ctx: &mut Self::Context) -> Self::Result {
        let Ok(json_str) = serde_json::to_string(&msg.data) else {
            return Err("json 序列化失败");
        };
        ctx.text(format!(
            "{}{}{}",
            EngineIOPacketType::Message as u8,
            SocketIOPacketType::Connect as u8,
            json_str
        ));

        Ok(())
    }
}

impl<T: Serialize> Handler<Arc<Emiter<T>>> for Session {
    type Result = Result<(), &'static str>;
    fn handle(&mut self, msg: Arc<Emiter<T>>, ctx: &mut Self::Context) -> Self::Result {
        let Ok(json_str) = serde_json::to_string(&msg.data) else {
            return Err("json 序列化失败");
        };
        ctx.text(format!(
            "{}{}[\"{}\",{}]",
            EngineIOPacketType::Message as u8,
            SocketIOPacketType::Event as u8,
            msg.event_name,
            json_str
        ));

        Ok(())
    }
}

/// 建立连接回应给客户端处理
impl Handler<ConnectPacket> for Session {
    type Result = Result<(), &'static str>;
    fn handle(&mut self, msg: ConnectPacket, ctx: &mut Self::Context) -> Self::Result {
        let Ok(json_str) = serde_json::to_string(&msg.data) else {
            return Err("json 序列化失败");
        };
        ctx.text(format!("{}{}", msg.r#type as u8, json_str));

        Ok(())
    }
}

impl Handler<OpenPacket> for Session {
    type Result = Result<(), &'static str>;
    fn handle(&mut self, msg: OpenPacket, ctx: &mut Self::Context) -> Self::Result {
        let Ok(json_str) = serde_json::to_string(&msg) else {
            return Err("json 序列化失败");
        };

        ctx.text(format!("{}{}", EngineIOPacketType::Open as u8, json_str));

        Ok(())
    }
}

impl<T: Serialize> Handler<AuthSuccess<T>> for Session {
    type Result = Result<(), &'static str>;
    fn handle(&mut self, msg: AuthSuccess<T>, ctx: &mut Self::Context) -> Self::Result {
        let Ok(json_str) = serde_json::to_string(&msg) else {
            return Err("json 序列化失败");
        };

        ctx.text(format!(
            "{}{}{}",
            EngineIOPacketType::Message as u8,
            SocketIOPacketType::Connect as u8,
            json_str
        ));

        Ok(())
    }
}

impl StreamHandler<Result<ws::Message, ws::ProtocolError>> for Session {
    /// 收到消息后的处理
    fn handle(&mut self, item: Result<ws::Message, ws::ProtocolError>, ctx: &mut Self::Context) {
        // 提取消息
        let msg = match item {
            Err(_) => {
                ctx.stop();
                return;
            }
            Ok(msg) => msg,
        };
        match msg {
            // 收到文本消息
            Message::Text(byte_string) => {
                let raw = byte_string.to_string();
                let mut eg_type = None;
                let mut sc_type = None;
                let data_str = raw.get(2..);

                eg_type = raw
                    .get(0..1)
                    .and_then(|f| f.parse::<u8>().ok())
                    .and_then(|f| EngineIOPacketType::try_from(f).ok());

                sc_type = raw
                    .get(1..2)
                    .and_then(|f| f.parse::<u8>().ok())
                    .and_then(|f| SocketIOPacketType::try_from(f).ok());

                if let Some(eg_type) = eg_type {
                    match eg_type {
                        EngineIOPacketType::Open => (),
                        EngineIOPacketType::Close => (),
                        EngineIOPacketType::Ping => (),
                        EngineIOPacketType::Pong => {
                            // 客户端心跳上报
                            self.heartbeat = true;
                        }
                        EngineIOPacketType::Message => {
                            if let Some(sc_type) = sc_type {
                                if let Some(data_str) = data_str {
                                    let sended = self.sender.send(match sc_type {
                                        SocketIOPacketType::Connect => MessageType::Connect,
                                        SocketIOPacketType::Disconnect => MessageType::None,
                                        SocketIOPacketType::Event => {
                                            serde_json::from_str::<EventData>(data_str)
                                                .map_or(MessageType::None, |event| {
                                                    MessageType::Event(event)
                                                })
                                        }
                                        SocketIOPacketType::Ack => MessageType::None,
                                        SocketIOPacketType::ConnectError => MessageType::None,
                                        SocketIOPacketType::BinaryEvent => MessageType::None,
                                        SocketIOPacketType::BinaryAck => MessageType::None,
                                    });

                                    if sended.is_err() {
                                        log::error!("socket-io 发送数据失败{sended:?}");
                                    }
                                }
                            }
                        }
                        EngineIOPacketType::Upgrade => (),
                        EngineIOPacketType::Noop => (),
                    }
                }
            }
            // 收到二进制消息
            Message::Binary(_bytes) => {
                // data_binary = bytes;
            }
            _ => {}
        }
    }
}

/// 建立连接 header 头
#[derive(Serialize, Deserialize, Clone)]
struct Header {
    sid: Option<String>,
    token: Option<String>,
}

/// 建立连接结构体
#[derive(Message)]
#[rtype(result = "Result<(), &'static str>")]
pub struct ConnectPacket {
    r#type: SocketIOPacketType,
    data: Header,
}

/// 鉴权响应数据
#[derive(Message, Serialize)]
#[rtype(result = "Result<(), &'static str>")]
pub struct AuthSuccess<T: Serialize> {
    pub data: T,
}

/// 发送客户端
#[derive(Message)]
#[rtype(result = "Result<(), &'static str>")]
pub struct Emiter<T: Serialize> {
    pub event_name: String,
    pub data: T,
}

/// 存储所有客户端会话的 store
pub struct SessionStore {
    // 存储的客户端会话
    pub sessions: HashMap<Uuid, Addr<Session>>,
}
impl SessionStore {
    pub fn new() -> Self {
        Self {
            sessions: HashMap::new(),
        }
    }
}
