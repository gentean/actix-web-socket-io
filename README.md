[中文](README.zh-CN.md) | **English**

## Socket.IO server for actix-web

Implements [Socket.IO Protocol V5](https://github.com/socketio/socket.io-protocol/tree/main?tab=readme-ov-file#exchange-protocol) and [Engine.IO Protocol V4](https://github.com/socketio/engine.io-protocol/tree/main?tab=readme-ov-file#protocol), with [actix-ws](https://crates.io/crates/actix-ws) as the transport.

Supported:

- JSON events (`on` / `emit`)
- Binary events (`on` / `emit_binary`)
- Rooms (`join` / `leave` / `to(room).emit`)
- Connect / disconnect (`connect` / `disconnect`)
- Engine.IO heartbeat

### Install

```toml
actix-web-socket-io = "0.1"
actix-web = "4"
```

### Example

```rust
#[get("/socket.io")]
async fn listen_system(req: HttpRequest, stream: Payload) -> impl Responder {
    let SocketIOResult {
        http_response,
        session_receive,
        session_id,
    } = socket::socket_io().connect(&req, stream);

    session_receive
        .on(Listener {
            event_name: "connect".into(),
            handler: Box::new(SocketConnected),
        })
        .await;

    session_receive
        .on(Listener {
            event_name: "disconnect".into(),
            handler: Box::new(SocketDisConnected),
        })
        .await;

    session_receive
        .on(Listener {
            event_name: "/subscribe/data".into(),
            handler: Box::new(SubscribeData),
        })
        .await;

    // Event name matches the client: socket.emit("upload", buffer)
    session_receive
        .on(Listener {
            event_name: "upload".into(),
            handler: Box::new(OnUpload),
        })
        .await;

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
                    Some(session_id), // None broadcasts to every session
                )
                .await
            {
                log::error!("failed to emit system time: {msg}");
            }

            // Binary push: text header 45N-[...] followed by binary frames
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
        log::info!("client connected, session_id={session_id}");
    }
}

pub struct SocketDisConnected;
#[async_trait]
impl MessageHandle for SocketDisConnected {
    async fn handler(&self, _: serde_json::Value, session_id: Uuid) {
        log::info!("client disconnected, session_id={session_id}");
    }
}

pub struct SubscribeData;
#[async_trait]
impl MessageHandle for SubscribeData {
    async fn handler(&self, data: serde_json::Value, session_id: Uuid) {
        log::info!("subscribe received, session_id={session_id}, data={data}");
    }
}

pub struct OnUpload;
#[async_trait]
impl MessageHandle for OnUpload {
    async fn handler(&self, data: serde_json::Value, session_id: Uuid) {
        // Binary attachments are restored as { "type": "Buffer", "data": [u8...] }
        log::info!("binary received, session_id={session_id}, data={data}");
    }
}

pub fn add_route() -> Scope {
    web::scope("/system").service(listen_system)
}
```

A bare binary frame with no matching text header is dispatched as the `binary` event.

You can also attach a JSON argument when pushing binary:

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

The client receives `socket.on("download", (meta, buffer) => { ... })`.

### Rooms

Rooms are server-side only. Existing `emit` / `on` / `MessageHandle` stay unchanged.

```rust
session_receive.join("lobby").await;
session_receive.leave("lobby").await;

socket_server
    .to("lobby")
    .except(session_id) // optional
    .emit(Emiter {
        event_name: "/chat/message".into(),
        data: "hello",
    })
    .await?;

// Union of rooms
socket_server.to("lobby").to("vip").emit(emiter).await?;
```

The session leaves every room automatically on disconnect.

### Config

```rust
let mut socket_io = SocketIO::new();
socket_io.config(SocketConfig {
    ping_interval: 25000, // heartbeat interval, milliseconds
    ping_timeout: 20000,  // heartbeat timeout, milliseconds
    max_payload: 1_000_000,
});
```

See `examples/basic` for a full example.

## License

actix-web-socket-io may be used under your choice of the BSD 3-clause, Apache 2, or MIT license.
