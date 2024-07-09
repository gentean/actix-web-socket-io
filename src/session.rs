use actix::prelude::*;

use actix_web_actors::ws::{self, Message};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap, sync::{Arc, RwLock}, time::Duration
};
use uuid::Uuid;

use crate::socketio::{EngineIOPacketType, MessageType, OpenPacket, SocketIOPacketType, EventData};


/// =============================================
/// 会话
pub struct Session {
    pub id: Uuid,
    session_store: Arc<RwLock<SessionStore>>,
    message_handler: Option<Box<dyn FnMut(MessageType) + 'static>>,
    pub heartbeat: bool,
}

impl Session {
    pub fn new(session_store: Arc<RwLock<SessionStore>>) -> Self {
        Self {
            id: Uuid::new_v4(),
            session_store,
            message_handler: None,
            heartbeat: true,
        }
    }

    /// 注册消息处理逻辑
    pub fn register_handler(&mut self, handler: impl FnMut(MessageType) + 'static) {
        self.message_handler = Some(Box::new(handler));
    }
}

impl Actor for Session {
    type Context = ws::WebsocketContext<Self>;

    /// 会话创建后
    fn started(&mut self, ctx: &mut Self::Context) {
        self.session_store
            .write()
            .unwrap()
            .sessions
            .insert(self.id, ctx.address());

        // 回应 engine.io
        let ping_interval = 25000;
        let ping_timeout = 20000;
        ctx.address().do_send(OpenPacket {
            sid: self.id.to_string(),
            upgrades: vec![],
            ping_interval,
            ping_timeout,
            max_payload: 1000000,
        });

        // 心跳
        ctx.run_interval(
            Duration::from_millis(ping_interval.into()),
            move |session, ctx| {
                // 发送 Ping
                ctx.text(EngineIOPacketType::Ping.to_value().to_string());
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
    fn stopping(&mut self, ctx: &mut Self::Context) -> Running {
        self.session_store
            .write()
            .unwrap()
            .sessions
            .remove(&self.id);
        Running::Stop
    }
}

impl<T: Serialize> Handler<Emiter<T>> for Session {
    type Result = ();

    fn handle(&mut self, msg: Emiter<T>, ctx: &mut Self::Context) -> Self::Result {
        ctx.text(format!(
            "{}{}[\"{}\",{}]",
            EngineIOPacketType::Message.to_value(),
            SocketIOPacketType::Event.to_value(),
            msg.event_name,
            serde_json::to_string(&msg.data).unwrap()
        ))
    }
}

/// 建立连接回应给客户端处理
impl Handler<ConnectPacket> for Session {
    type Result = ();
    fn handle(&mut self, msg: ConnectPacket, ctx: &mut Self::Context) -> Self::Result {
        ctx.text(format!(
            "{}{}",
            msg.r#type.to_value(),
            serde_json::to_string(&msg.data).unwrap()
        ))
    }
}

impl Handler<OpenPacket> for Session {
    type Result = ();
    fn handle(&mut self, msg: OpenPacket, ctx: &mut Self::Context) -> Self::Result {
        ctx.text(format!(
            "{}{}",
            EngineIOPacketType::Open.to_value(),
            serde_json::to_string(&msg).unwrap()
        ))
    }
}

impl<T: Serialize> Handler<AuthSuccess<T>> for Session {
    type Result = ();
    fn handle(&mut self, msg: AuthSuccess<T>, ctx: &mut Self::Context) -> Self::Result {
        ctx.text(format!(
            "{}{}{}",
            EngineIOPacketType::Message.to_value(),
            SocketIOPacketType::Connect.to_value(),
            serde_json::to_string(&msg.data).unwrap()
        ))
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

                if let Some(eg_type_str) = raw.get(0..1) {
                    if let Ok(eg_type_val) = eg_type_str.parse::<u8>() {
                        if let Some(type_enum) = EngineIOPacketType::from_value(eg_type_val) {
                            eg_type = Some(type_enum);
                        }
                    }
                }

                if let Some(sc_type_str) = raw.get(1..2) {
                    if let Ok(sc_type_val) = sc_type_str.parse::<u8>() {
                        if let Some(type_enum) = SocketIOPacketType::from_value(sc_type_val) {
                            sc_type = Some(type_enum);
                        }
                    }
                }

                if let Some(eg_type) = eg_type {
                    match eg_type {
                        EngineIOPacketType::Open => todo!(),
                        EngineIOPacketType::Close => todo!(),
                        EngineIOPacketType::Ping => todo!(),
                        EngineIOPacketType::Pong => {
                            // 客户端心跳上报
                            self.heartbeat = true;
                        }
                        EngineIOPacketType::Message => {
                            if let Some(sc_type) = sc_type {
                                if let Some(data_str) = data_str {
                                    if let Some(handler) = self.message_handler.as_mut() {
                                        match sc_type {
                                            SocketIOPacketType::Connect => {
                                                // 鉴权
                                                handler(MessageType::Auth(data_str));
                                            }
                                            SocketIOPacketType::Disconnect => todo!(),
                                            SocketIOPacketType::Event => {
                                                // 提取事件名，事件参数
                                                let event =
                                                    serde_json::from_str::<EventData>(data_str)
                                                        .unwrap();
                                                handler(MessageType::Event(event));
                                            }
                                            SocketIOPacketType::Ack => todo!(),
                                            SocketIOPacketType::ConnectError => todo!(),
                                            SocketIOPacketType::BinaryEvent => todo!(),
                                            SocketIOPacketType::BinaryAck => todo!(),
                                        }
                                    }
                                }
                            }
                        }
                        EngineIOPacketType::Upgrade => todo!(),
                        EngineIOPacketType::Noop => todo!(),
                    }
                }
            }
            // 收到二进制消息
            Message::Binary(bytes) => {
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
#[rtype(result = "()")]
pub struct ConnectPacket {
    r#type: SocketIOPacketType,
    data: Header,
}

#[derive(Message, Serialize)]
#[rtype(result = "()")]
pub struct AuthSuccess<T: Serialize> {
    pub data: T,
}

/// 发送客户端
#[derive(Message)]
#[rtype(result = "()")]
pub struct Emiter<T: Serialize> {
    pub event_name: String,
    pub data: T,
}

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