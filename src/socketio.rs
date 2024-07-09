use actix::Message;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// 协议文档 https://github.com/socketio/socket.io-protocol/tree/main?tab=readme-ov-file#exchange-protocol
pub enum SocketIOPacketType {
    // 用于连接
    Connect,
    // 用于断开连接
    Disconnect,
    // 用于向对方发送数据
    Event,
    // 用于数据确认
    Ack,
    // 用于连接错误，如鉴权失败
    ConnectError,
    // 用于二进制数据
    BinaryEvent,
    // 用于二进制数据应答
    BinaryAck,
}

impl SocketIOPacketType {
    pub fn to_value(&self) -> u8 {
        match self {
            SocketIOPacketType::Connect => 0,
            SocketIOPacketType::Disconnect => 1,
            SocketIOPacketType::Event => 2,
            SocketIOPacketType::Ack => 3,
            SocketIOPacketType::ConnectError => 4,
            SocketIOPacketType::BinaryEvent => 5,
            SocketIOPacketType::BinaryAck => 6,
        }
    }
    pub fn from_value(val: u8) -> Option<Self> {
        match val {
            0 => Some(Self::Connect),
            1 => Some(Self::Disconnect),
            2 => Some(Self::Event),
            3 => Some(Self::Ack),
            4 => Some(Self::ConnectError),
            5 => Some(Self::BinaryEvent),
            6 => Some(Self::BinaryAck),
            _ => None,
        }
    }
}


/// 协议文档 https://github.com/socketio/engine.io-protocol/tree/main?tab=readme-ov-file#protocol
pub enum EngineIOPacketType {
    // 握手
    Open,
    // 传输可以关闭
    Close,
    // 心跳
    Ping,
    // 心跳
    Pong,
    // 发送有效载荷
    Message,
    // 升级
    Upgrade,
    // 升级
    Noop,
}
impl EngineIOPacketType {
    pub fn to_value(&self) -> u8 {
        match self {
            EngineIOPacketType::Open => 0,
            EngineIOPacketType::Close => 1,
            EngineIOPacketType::Ping => 2,
            EngineIOPacketType::Pong => 3,
            EngineIOPacketType::Message => 4,
            EngineIOPacketType::Upgrade => 5,
            EngineIOPacketType::Noop => 6,
        }
    }
    pub fn from_value(val: u8) -> Option<Self> {
        match val {
            0 => Some(EngineIOPacketType::Open),
            1 => Some(EngineIOPacketType::Close),
            2 => Some(EngineIOPacketType::Ping),
            3 => Some(EngineIOPacketType::Pong),
            4 => Some(EngineIOPacketType::Message),
            5 => Some(EngineIOPacketType::Upgrade),
            6 => Some(EngineIOPacketType::Noop),
            _ => None,
        }
    }
}


/// 握手数据 https://github.com/socketio/engine.io-protocol/tree/main?tab=readme-ov-file#handshake
#[derive(Message, Serialize)]
#[rtype(result = "()")]
#[serde(rename_all = "camelCase")]
pub struct OpenPacket {
    // 会话 ID
    pub sid: String,
    // 可以升级的 transport 列表，默认为 [websocket]
    pub upgrades: Vec<String>,
    // 心跳间隔(毫秒), 25000
    pub ping_interval: u32,
    // 心跳超时(毫秒), 20000
    pub ping_timeout: u32,
    // 每个块的最大字节数, 1000000
    pub max_payload: u32,
}

#[derive(Deserialize, Debug)]
pub struct EventData(pub String, pub Value);

pub enum MessageType<'a> {
    // 鉴权
    Auth(&'a str),
    // 事件
    Event(EventData),
}
