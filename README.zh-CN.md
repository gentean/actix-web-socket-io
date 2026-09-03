**中文** | [English](README.md)

## actix-web 的 Socket.IO 服务端实现

基于 [Socket.IO Protocol V5](https://github.com/socketio/socket.io-protocol/tree/main?tab=readme-ov-file#exchange-protocol) 和 [Engine.IO Protocol V4](https://github.com/socketio/engine.io-protocol/tree/main?tab=readme-ov-file#protocol)，传输层使用 [actix-ws](https://crates.io/crates/actix-ws)。

已支持：

- JSON 事件的接收与推送（`on` / `emit`）
- 二进制事件的接收与推送（`on` / `emit_binary`）
- 连接 / 断开（`connect` / `disconnect`）
- Engine.IO 心跳

### 安装

```toml
actix-web-socket-io = "0.1"
actix-web = "4"
```

### 示例

```rust
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

    // 订阅客户端 JSON 事件
    session_receive
        .on(Listener {
            event_name: "/subscribe/data".into(),
            handler: Box::new(SubscribeData),
        })
        .await;

    // 订阅客户端二进制事件（事件名与 socket.emit("upload", buffer) 一致）
    session_receive
        .on(Listener {
            event_name: "upload".into(),
            handler: Box::new(OnUpload),
        })
        .await;

    // 主动推送
    actix_web::rt::spawn(async move {
        let socket_server = socket::get_server();

        loop {
            actix_web::rt::time::sleep(Duration::from_millis(1000)).await;

            if let Err(msg) = socket_server
                .emit(
                    Emiter {
                        event_name: "/system/timestamp".into(),
                        data: Utc::now().timestamp_millis(),
                    },
                    Some(session_id), // None 则广播给所有会话
                )
                .await
            {
                log::error!("系统的时间刷出失败, msg: {}", msg);
            }

            // 推送二进制：先发 45N-[...] 文本头，再跟二进制帧
            let _ = socket_server
                .emit_binary(
                    BinaryEmiter::new("download", vec![Bytes::from_static(b"hello")]),
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
    }
}

pub struct SocketDisConnected;
#[async_trait]
impl MessageHandle for SocketDisConnected {
    async fn handler(&self, _: serde_json::Value, session_id: Uuid) {
        log::info!("有客户端断开连接，session_id={session_id}");
    }
}

pub struct SubscribeData;
#[async_trait]
impl MessageHandle for SubscribeData {
    async fn handler(&self, data: serde_json::Value, session_id: Uuid) {
        log::info!("收到订阅, session_id={session_id}, data={data}");
    }
}

pub struct OnUpload;
#[async_trait]
impl MessageHandle for OnUpload {
    async fn handler(&self, data: serde_json::Value, session_id: Uuid) {
        // 二进制会被还原为 { "type": "Buffer", "data": [u8...] }
        log::info!("收到二进制, session_id={session_id}, data={data}");
    }
}

pub fn add_route() -> Scope {
    web::scope("/system").service(listen_system)
}
```

没有对应文本头的裸二进制帧，会落到事件名 `binary`。

推送二进制时也可以带一段 JSON：

```rust
socket_server
    .emit_binary(
        BinaryEmiter::with_data(
            "download",
            serde_json::json!({ "name": "a.bin" }),
            vec![Bytes::from(content)],
        ),
        Some(session_id),
    )
    .await?;
```

客户端会收到 `socket.on("download", (meta, buffer) => { ... })`。

### 配置

```rust
let mut socket_io = SocketIO::new();
socket_io.config(SocketConfig {
    ping_interval: 25000, // 心跳间隔，毫秒
    ping_timeout: 20000,  // 心跳超时，毫秒
    max_payload: 1_000_000,
});
```

完整示例见 `examples/basic`。

## License

actix-web-socket-io may be used under your choice of the BSD 3-clause, Apache 2, or MIT license.
