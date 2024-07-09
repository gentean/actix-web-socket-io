use actix_web::{
    web::{Bytes, Payload},
    HttpRequest, HttpResponse,
};
use actix_web_actors::ws::{self};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use session::{AuthSuccess, Emiter, Session, SessionStore};
use socketio::{EventData, MessageType};
use std::sync::{Arc, RwLock};
use uuid::Uuid;

pub mod session;
pub mod socketio;

pub struct SocketIO {
    pub socket_server: Arc<SocketServer>,
}

pub struct SocketIOResult<T: HandleAuth> {
    pub http_response: Result<HttpResponse, actix_web::error::Error>,
    pub session_receive: Arc<SessionReceive<T>>,
    pub session_id: Uuid,
}

impl SocketIO {
    pub fn new() -> Self {
        Self {
            socket_server: Arc::new(SocketServer::new()),
        }
    }
    /// 建立连接
    pub fn connect<T: HandleAuth + 'static>(
        &self,
        req: &HttpRequest,
        stream: Payload,
        auth_handle: T,
    ) -> SocketIOResult<T> {
        // 创建一个新会话
        let mut session = Session::new(self.socket_server.clone().session_store.clone());

        let session_receive = Arc::new(SessionReceive::new(
            auth_handle,
            session.id,
            self.socket_server.clone(),
        ));

        let inner_receive = session_receive.clone();
        // 收到事件统一处理
        session.register_handler(move |message_data| {
            inner_receive.handle_receive_msg(message_data);
        });

        SocketIOResult {
            session_id: session.id,
            http_response: ws::start(session, req, stream),
            session_receive: session_receive.clone(),
        }
    }
}

pub trait MessageHandle: Sync + Send + 'static {
    fn handler(&self, data: Value, session_id: Uuid, socket_server: Arc<SocketServer>);
}

/// 监听客户端
pub struct Listener {
    pub event_name: String,
}

pub struct SocketServer {
    pub session_store: Arc<RwLock<SessionStore>>,
}

pub trait HandleAuth {
    type AuthReq: for<'a> Deserialize<'a>;
    type AuthRes: Send + Serialize + 'static;
    fn handler(&self, auth_data: Self::AuthReq) -> Option<Self::AuthRes>;
}

pub struct SessionReceive<T>
where
    T: HandleAuth,
{
    handle_auth: T,
    session_id: Uuid,
    // 服务端监听的事件总线
    listeners: RwLock<Vec<(Listener, Arc<Box<dyn MessageHandle>>)>>,
    socket_server: Arc<SocketServer>,
}

impl<T: HandleAuth> SessionReceive<T> {
    pub fn new(handle_auth: T, session_id: Uuid, socket_server: Arc<SocketServer>) -> Self {
        Self {
            handle_auth,
            session_id,
            listeners: RwLock::new(vec![]),
            socket_server,
        }
    }

    /// 接收到客户端发来的事件
    fn handle_receive_msg(&self, message_type: MessageType) {
        match message_type {
            MessageType::Auth(data_str) => self.handle_auth_msg(data_str),
            MessageType::Event(message_data) => self.handler_triger_on(message_data),
        }
    }

    /// 触发事件
    fn handler_triger_on(&self, event: EventData) {
        for (listener, handler) in self.listeners.read().unwrap().iter() {
            // 按事件名匹配
            if listener.event_name.eq(&event.0) {
                handler.handler(event.1.clone(), self.session_id, self.socket_server.clone());
            }
        }
    }

    /// 处理授权
    fn handle_auth_msg(&self, data_str: &str) {
        if let Some(result) = self
            .handle_auth
            .handler(serde_json::from_str::<T::AuthReq>(data_str).unwrap())
        {
            let session_store = self.socket_server.session_store.write().unwrap();
            let addr = session_store.sessions.get(&self.session_id);
            if let Some(addr) = addr {
                addr.do_send(AuthSuccess { data: result });
            }
        }
        self.handler_triger_on(EventData("connect".into(), Value::Null))
    }

    /// 处理二进制数据
    pub fn handle_receive_binary_msg(&mut self, data_bin: Bytes) {
        // 触发监听
    }

    /// 监听客户端推来的事件
    pub fn on<U: MessageHandle>(&self, listener: Listener, handler: U) {
        self.listeners
            .write()
            .unwrap()
            .push((listener, Arc::new(Box::new(handler))));
    }
}

impl SocketServer {
    pub fn new() -> Self {
        Self {
            session_store: Arc::new(RwLock::new(SessionStore::new())),
        }
    }

    /// 发送事件给客户端
    pub fn emit<D: Serialize + Send + 'static>(&self, emiter: Emiter<D>, session_id: Option<Uuid>) {
        if let Some(session_id) = session_id {
            self.session_store
                .read()
                .unwrap()
                .sessions
                .get(&session_id)
                .unwrap()
                .do_send(emiter);
        }
    }
}
