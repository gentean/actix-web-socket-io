use actix::Message;
use num_enum::{IntoPrimitive, TryFromPrimitive};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// 协议文档 https://github.com/socketio/socket.io-protocol/tree/main?tab=readme-ov-file#exchange-protocol
#[derive(IntoPrimitive, Clone, Eq, PartialEq, TryFromPrimitive)]
#[repr(u8)]
pub enum SocketIOPacketType {
    // 用于连接
    Connect = 0,
    // 用于断开连接
    Disconnect = 1,
    // 用于向对方发送数据
    Event = 2,
    // 用于数据确认
    Ack = 3,
    // 用于连接错误，如鉴权失败
    ConnectError = 4,
    // 用于二进制数据
    BinaryEvent = 5,
    // 用于二进制数据应答
    BinaryAck = 6,
}

/// 协议文档 https://github.com/socketio/engine.io-protocol/tree/main?tab=readme-ov-file#protocol
#[derive(IntoPrimitive, Clone, Eq, PartialEq, TryFromPrimitive)]
#[repr(u8)]
pub enum EngineIOPacketType {
    // 握手
    Open = 0,
    // 传输可以关闭
    Close = 1,
    // 心跳
    Ping = 2,
    // 心跳
    Pong = 3,
    // 发送有效载荷
    Message = 4,
    // 升级
    Upgrade = 5,
    // 升级
    Noop = 6,
}

/// 握手数据 https://github.com/socketio/engine.io-protocol/tree/main?tab=readme-ov-file#handshake
#[derive(Message, Serialize)]
#[rtype(result = "Result<(), &'static str>")]
#[serde(rename_all = "camelCase")]
pub struct OpenPacket {
    // 会话 ID
    pub sid: String,
    // 可以升级的 transport 列表，默认为 [websocket]
    pub upgrades: Vec<String>,
    // 心跳间隔(毫秒), 25000
    pub ping_interval: u64,
    // 心跳超时(毫秒), 20000
    pub ping_timeout: u64,
    // 每个块的最大字节数, 1000000
    pub max_payload: usize,
}

#[derive(Deserialize, Debug)]
pub struct EventData(pub String, pub Value);

pub enum MessageType {
    None,
    // 请求连接
    Connect,
    // 事件
    Event(EventData),
}

/// 连接成功响应数据
#[derive(Message, Serialize)]
#[rtype(result = "Result<(), &'static str>")]
pub struct ConnectSuccess<T: Serialize> {
    pub data: T,
}