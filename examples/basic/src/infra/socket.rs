use std::sync::Arc;

use actix_web_socket_io::{SocketConfig, SocketIO, SocketServer};
use lazy_static::lazy_static;

lazy_static! {
    static ref WEB_SOCKET: SocketIO = {
        let mut socket_io = SocketIO::new();

        socket_io.config(SocketConfig {
            ping_interval: 15000,
            ..SocketConfig::default()
        });

        socket_io
    };
}

pub fn socket_io() -> &'static SocketIO {
    &WEB_SOCKET
}

pub fn get_server() -> Arc<SocketServer> {
    WEB_SOCKET.socket_server.clone()
}
