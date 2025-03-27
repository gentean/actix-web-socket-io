use actix_web::{App, HttpServer};

mod websocket;
mod infra;
mod validate;
mod service;

#[actix_web::main]
pub async fn main() -> std::io::Result<()> {
    let mut http_server = HttpServer::new(move || {
        let mut app = App::new();
        app = websocket::register(app);

        app
    });

    http_server = http_server.bind("0.0.0.0:8080")?;

    http_server.run().await
}
