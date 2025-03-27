use actix_web_socket_io::HandleAuth;
use serde::{Deserialize, Serialize};

#[derive(Deserialize, Debug)]
pub struct AuthReq {
    token: String,
}

#[derive(Serialize)]
pub struct AuthRes {
    sid: String,
}

pub struct SocketAuth;
impl HandleAuth for SocketAuth {
    type AuthReq = AuthReq;
    type AuthRes = AuthRes;
    fn handler(&self, _auth_data: Self::AuthReq) -> Option<Self::AuthRes> {
        Some(AuthRes { sid: "1".into() })
    }
}