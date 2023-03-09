use std::sync::{Arc, Mutex};

use prometheus_client::encoding::text::encode;
use prometheus_client::registry::Registry;
use rocket::http::Status;
use rocket::request::Request;
use rocket::response::{Responder, Response};
use rocket::{get, routes, State};
use rocket::{Build, Rocket, Route};

use crate::app::RegistryApp;
use crate::middleware::prometheus::{HttpMetrics, Port};
use crate::middleware::shutdown::Lifecycle;

pub(crate) enum Responses {
    Ok {},
}

impl<'r> Responder<'r, 'static> for Responses {
    fn respond_to(self, _req: &Request) -> Result<Response<'static>, Status> {
        match self {
            Responses::Ok {} => Response::build().status(Status::Ok).ok(),
        }
    }
}

#[derive(Responder)]
#[response(
    status = 200,
    content_type = "application/openmetrics-text; version=1.0.0; charset=utf-8"
)]
struct Metrics(String);

#[get("/metrics")]
async fn metrics(registry: &State<Arc<Mutex<Registry>>>) -> Metrics {
    let mut encoded = String::new();
    encode(&mut encoded, &registry.lock().unwrap()).unwrap();
    Metrics(encoded)
}

#[get("/healthz")]
pub(crate) async fn healthz() -> Responses {
    Responses::Ok {}
}

fn routes() -> Vec<Route> {
    routes![metrics, healthz]
}

pub(crate) fn configure(app: Arc<RegistryApp>, mut registry: Registry) -> Rocket<Build> {
    let fig = rocket::Config::figment()
        .merge(("port", app.settings.prometheus.port))
        .merge(("address", app.settings.prometheus.address.clone()));

    rocket::custom(fig)
        .mount("/", routes())
        .attach(HttpMetrics::new(&mut registry, Port::Prometheus))
        .attach(Lifecycle {})
        .manage(app)
        .manage(Arc::new(Mutex::new(registry)))
}

#[cfg(test)]
mod test {
    /*
        use prometheus_client::registry::Registry;
        use rocket::{http::Status, local::blocking::Client};

    fn client() -> Client {
        let server = super::configure(<Registry>::default());
        Client::tracked(server).expect("valid rocket instance")
    }

    #[test]
    fn test_404() {
        // check that server is actually 404ing, not just 200ing everything
        let client = client();
        let response = client.get("/404").dispatch();
        assert_eq!(response.status(), Status::NotFound);
    }

    #[test]
    fn test_metrics() {
        let client = client();
        let response = client.get("/metrics").dispatch();
        assert_eq!(response.status(), Status::Ok);
        assert!(matches!(
            response.headers().get_one("Content-Type"),
            Some("application/openmetrics-text; version=1.0.0; charset=utf-8")
        ));
    }

    #[test]
    fn test_healthz() {
        let client = client();
        let response = client.get("/healthz").dispatch();
        assert_eq!(response.status(), Status::Ok);
    }
    */
}