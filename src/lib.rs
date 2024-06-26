#![allow(clippy::uninlined_format_args)]

use std::io::Cursor;
use std::sync::Arc;
use std::sync::Mutex;

use actix_request_identifier::RequestIdentifier;
use actix_web::middleware::Logger;
use actix_web::middleware::{Compress, NormalizePath};
use actix_web::web;
use actix_web::web::Data;
use actix_web::App;
use actix_web::HttpServer;
use certificate::ServerCertificate;
use config::Configuration;
use extractor::Extractor;
use middleware::prometheus::Port;
use middleware::prometheus::PrometheusHttpMetrics;
use openraft::storage::Adaptor;
use openraft::BasicNode;
use openraft::Config;
use openraft::Entry;
use openraft::Raft;
use rustls::ServerConfig;
use tokio::sync::Notify;
use webhook::start_webhook_worker;

use crate::app::RegistryApp;
use crate::network::api;
use crate::network::management;
use crate::network::raft;
use crate::network::raft_network_impl::RegistryNetwork;
use crate::store::RegistryRequest;
use crate::store::RegistryResponse;
use crate::store::RegistryStore;

pub mod app;
pub mod certificate;
pub mod client;
pub mod config;
pub mod extractor;
pub mod extractors;
pub mod garbage;
pub mod middleware;
pub mod mirror;
pub mod network;
pub mod prometheus;
pub mod registry;
pub mod store;
pub mod types;
pub mod utils;
pub mod webhook;

pub type RegistryNodeId = u64;

openraft::declare_raft_types!(
    /// Declare the type configuration for example K/V store.
    pub RegistryTypeConfig:
        D = RegistryRequest,
        R = RegistryResponse,
        NodeId = RegistryNodeId,
        Node = BasicNode,
        Entry = Entry<RegistryTypeConfig>,
        SnapshotData = Cursor<Vec<u8>>
);

pub type LogStore = Adaptor<RegistryTypeConfig, Arc<RegistryStore>>;
pub type StateMachineStore = Adaptor<RegistryTypeConfig, Arc<RegistryStore>>;
pub type RegistryRaft = Raft<RegistryTypeConfig>;

pub mod typ {
    use openraft::BasicNode;

    use crate::RegistryNodeId;
    use crate::RegistryTypeConfig;

    pub type RaftError<E = openraft::error::Infallible> =
        openraft::error::RaftError<RegistryNodeId, E>;
    pub type RPCError<E = openraft::error::Infallible> =
        openraft::error::RPCError<RegistryNodeId, BasicNode, RaftError<E>>;

    pub type ClientWriteError = openraft::error::ClientWriteError<RegistryNodeId, BasicNode>;
    pub type CheckIsLeaderError = openraft::error::CheckIsLeaderError<RegistryNodeId, BasicNode>;
    pub type ForwardToLeader = openraft::error::ForwardToLeader<RegistryNodeId, BasicNode>;
    pub type InitializeError = openraft::error::InitializeError<RegistryNodeId, BasicNode>;

    pub type ClientWriteResponse = openraft::raft::ClientWriteResponse<RegistryTypeConfig>;
}

fn create_dir(parent_dir: &str, child_dir: &str) -> std::io::Result<()> {
    let path = std::path::PathBuf::from(&parent_dir).join(child_dir);
    if !path.exists() {
        return std::fs::create_dir_all(path);
    }
    Ok(())
}

pub async fn start_raft_node(conf: Configuration) -> anyhow::Result<Arc<Notify>> {
    let _guard = conf.sentry.as_ref().map(|config| {
        sentry::init((
            config.endpoint.clone(),
            sentry::ClientOptions {
                release: sentry::release_name!(),
                ..Default::default()
            },
        ))
    });

    let node_id = conf.id()?;

    create_dir(&conf.storage, "uploads")?;
    create_dir(&conf.storage, "manifests")?;
    create_dir(&conf.storage, "blobs")?;

    let mut registry = <prometheus_client::registry::Registry>::default();

    // Create a configuration for the raft instance.
    let config = Config {
        heartbeat_interval: 500,
        election_timeout_min: 1500,
        election_timeout_max: 3000,
        ..Default::default()
    };

    let config = Arc::new(config.validate().unwrap());

    let mut path = std::path::Path::new(&conf.storage).to_path_buf();
    path.push("db");

    let db: sled::Db = sled::open(&path).unwrap_or_else(|_| panic!("could not open: {:?}", path));

    // Create a instance of where the Raft data will be stored.
    let store = RegistryStore::new(Arc::new(db), node_id, &mut registry).await;

    // Create the network layer that will connect and communicate the raft instances and
    // will be used in conjunction with the store created above.
    let network = RegistryNetwork {};

    let (log_store, state_machine) = Adaptor::new(store.clone());

    // Create a local raft instance.
    let raft = Raft::new(node_id, config.clone(), network, log_store, state_machine)
        .await
        .unwrap();

    let extractor = Arc::new(Extractor::new());

    let webhook_queue = start_webhook_worker(conf.webhooks.clone(), &mut registry);

    // Create an application that will store all the instances created above, this will
    // be later used on the actix-web services.
    let app = Data::new(RegistryApp {
        id: node_id,
        raft,
        store,
        config: conf.clone(),
        extractor,
        webhooks: Arc::new(webhook_queue),
        registry: Mutex::new(registry),
    });

    let app1 = app.clone();
    let app2 = app.clone();
    let app3 = app.clone();
    let app4 = app.clone();

    // Start the actix-web server.
    let server = HttpServer::new(move || {
        let json_config = web::JsonConfig::default().limit(4096 * 1024 * 10);

        App::new()
            .app_data(json_config)
            .wrap(RequestIdentifier::with_uuid())
            .wrap(PrometheusHttpMetrics::new(app1.clone(), Port::Raft))
            .wrap(sentry_actix::Sentry::new())
            .wrap(Logger::default())
            .wrap(Logger::new("%a %{User-Agent}i"))
            .wrap(Compress::default())
            .app_data(app1.clone())
            // raft internal RPC
            .service(raft::append)
            .service(raft::snapshot)
            .service(raft::vote)
            .service(raft::get_blob)
            .service(raft::get_manifest)
            // admin API
            .service(management::init)
            .service(management::add_learner)
            .service(management::change_membership)
            .service(management::metrics)
            .service(management::import)
            .service(management::export)
            // application API
            .service(api::write)
    })
    .disable_signals();

    let server = match conf.raft.tls.clone() {
        Some(tls) => {
            let certificate = ServerCertificate::new(tls.key.clone(), tls.chain.clone()).await?;
            let config = ServerConfig::builder()
                .with_no_client_auth()
                .with_cert_resolver(Arc::new(certificate));

            server.bind_rustls_0_23(
                (
                    app2.config.raft.address.clone().as_str(),
                    app2.config.raft.port,
                ),
                config,
            )?
        }
        None => server.bind((
            app2.config.raft.address.clone().as_str(),
            app2.config.raft.port,
        ))?,
    }
    .run();

    let registry_server = HttpServer::new(move || {
        let registry_api = web::scope("/v2")
            //   blob upload
            .service(registry::blobs::uploads::delete::delete)
            .service(registry::blobs::uploads::get::get)
            .service(registry::blobs::uploads::patch::patch)
            .service(registry::blobs::uploads::post::post)
            .service(registry::blobs::uploads::put::put)
            // blobs
            .service(registry::blobs::head::head)
            .service(registry::blobs::get::get)
            .service(registry::blobs::delete::delete)
            // manifests
            .service(registry::manifests::put::put)
            .service(registry::manifests::head::head)
            .service(registry::manifests::head::head_by_tag)
            .service(registry::manifests::get::get)
            .service(registry::manifests::get::get_by_tag)
            .service(registry::manifests::delete::delete)
            .service(registry::manifests::delete::delete_by_tag)
            // tags
            .service(registry::tags::get::get)
            // roots
            .service(registry::get::get)
            .service(registry::head::head);

        App::new()
            .wrap(RequestIdentifier::with_uuid())
            .wrap(PrometheusHttpMetrics::new(app.clone(), Port::Registry))
            .wrap(sentry_actix::Sentry::new())
            .wrap(Logger::default())
            .wrap(Logger::new("%a %{User-Agent}i"))
            .wrap(NormalizePath::trim())
            // we can't use compression because it enables transfer-encoding: chunked which breaks content-length which breaks containerd
            // .wrap(middleware::Compress::default())
            .app_data(app.clone())
            .service(registry_api)
    })
    .bind((
        app2.config.registry.address.as_str(),
        app2.config.registry.port,
    ))?
    .disable_signals()
    .run();

    let raft_handle = server.handle();
    let _handle1 = tokio::spawn(server);

    let registry_handle = registry_server.handle();
    let _handle2 = tokio::spawn(registry_server);

    // Start the actix-web server.
    let prometheus = HttpServer::new(move || {
        App::new()
            .wrap(RequestIdentifier::with_uuid())
            .wrap(PrometheusHttpMetrics::new(app4.clone(), Port::Prometheus))
            .wrap(sentry_actix::Sentry::new())
            .wrap(Logger::default())
            .wrap(Logger::new("%a %{User-Agent}i"))
            .wrap(Compress::default())
            .app_data(app4.clone())
            .service(prometheus::metrics)
            .service(prometheus::healthz)
    })
    .bind((
        app2.config.prometheus.address.clone().as_str(),
        app2.config.prometheus.port,
    ))?
    .disable_signals()
    .run();

    let prometheus_handle = prometheus.handle();
    let _handle3 = tokio::spawn(prometheus);

    let sender = Arc::new(Notify::new());
    let receiver = sender.clone();

    let _mirrorer = tokio::spawn(crate::mirror::do_miroring(app3.clone()));

    self::store::metrics::start_watching_metrics(app3.clone());

    tokio::spawn(async move {
        receiver.notified().await;
        drop(receiver);
        app3.raft.shutdown().await.unwrap();
        registry_handle.stop(false).await;
        raft_handle.stop(false).await;
        prometheus_handle.stop(false).await;
    });

    Ok(sender)
}
