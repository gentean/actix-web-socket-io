use actix_web::{
    dev::{ServiceFactory, ServiceRequest},
    web, App, Error,
};

mod bar;
mod foo;

pub fn register<T>(app: App<T>) -> App<T>
where
    T: ServiceFactory<ServiceRequest, Config = (), Error = Error, InitError = ()>,
{
    app.service(
        web::scope("/socket")
            .service(bar::add_route())
            .service(foo::add_route()),
    )
}
