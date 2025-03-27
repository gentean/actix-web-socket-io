use std::sync::Arc;

use actix_web_socket_io::{SocketIO, SocketServer};
use lazy_static::lazy_static;

lazy_static! {
    static ref WEB_SOCKET: SocketIO = SocketIO::new();
}

pub fn socket_io() -> &'static SocketIO {
    &WEB_SOCKET
}

pub fn get_server() -> Arc<SocketServer> {
    WEB_SOCKET.socket_server.clone()
}
