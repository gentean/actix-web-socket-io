use actix_web::web::Bytes;
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
#[derive(Serialize)]
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

#[derive(Deserialize, Debug, Clone)]
pub struct EventData(pub String, pub Value);

#[derive(Debug, Clone)]
pub enum MessageType {
    None,
    // 请求连接
    Connect,
    // 事件
    Event(EventData),
    // 二进制事件头（后续跟 attachments 个二进制帧）
    BinaryEvent { attachments: usize, payload: Value },
    // 二进制附件 / 原始二进制
    Binary(Bytes),
}

/// 解析 BINARY_EVENT 文本载荷：`<#>-[<namespace>,][<ack id>][json]`
/// 例如 `1-["upload",{"_placeholder":true,"num":0}]`
pub fn parse_binary_event_payload(data_str: &str) -> Option<(usize, Value)> {
    let dash = data_str.find('-')?;
    let attachments = data_str[..dash].parse().ok()?;
    let mut rest = &data_str[dash + 1..];

    if rest.starts_with('/') {
        let comma = rest.find(',')?;
        rest = &rest[comma + 1..];
    }

    let json_start = rest.find('[')?;
    if !rest[..json_start].chars().all(|c| c.is_ascii_digit()) {
        return None;
    }

    serde_json::from_str(&rest[json_start..])
        .ok()
        .map(|payload| (attachments, payload))
}

pub fn buffer_to_value(data: &[u8]) -> Value {
    serde_json::json!({
        "type": "Buffer",
        "data": data,
    })
}

/// 把 `{"_placeholder":true,"num":n}` 还原成对应的二进制附件
pub fn replace_placeholders(value: &mut Value, buffers: &[Bytes]) {
    match value {
        Value::Object(map) => {
            let is_placeholder = map.get("_placeholder").and_then(Value::as_bool) == Some(true);
            if is_placeholder {
                if let Some(num) = map.get("num").and_then(Value::as_u64) {
                    if let Some(buf) = buffers.get(num as usize) {
                        *value = buffer_to_value(buf);
                        return;
                    }
                }
            }
            for child in map.values_mut() {
                replace_placeholders(child, buffers);
            }
        }
        Value::Array(items) => {
            for item in items {
                replace_placeholders(item, buffers);
            }
        }
        _ => {}
    }
}

/// 连接成功响应数据
#[derive(Serialize)]
pub struct ConnectSuccess<T: Serialize> {
    pub data: T,
}
