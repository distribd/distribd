/*
use std::collections::BTreeMap;
use std::collections::BTreeSet;

use openraft::error::Infallible;
use openraft::Node;
use openraft::RaftMetrics;

use crate::app::ExampleApp;
use crate::ExampleNodeId;
use crate::ExampleTypeConfig;

#[post("/add-learner")]
pub async fn add_learner(
    app: Data<ExampleApp>,
    req: Json<(ExampleNodeId, String)>,
) -> actix_web::Result<impl Responder> {
    let node_id = req.0 .0;
    let node = Node {
        addr: req.0 .1.clone(),
        ..Default::default()
    };
    let res = app.raft.add_learner(node_id, Some(node), true).await;
    Ok(Json(res))
}

/// Changes specified learners to members, or remove members.
#[post("/change-membership")]
pub async fn change_membership(
    app: Data<ExampleApp>,
    req: Json<BTreeSet<ExampleNodeId>>,
) -> actix_web::Result<impl Responder> {
    let res = app.raft.change_membership(req.0, true, false).await;
    Ok(Json(res))
}

/// Initialize a single-node cluster.
#[post("/init")]
pub async fn init(app: Data<ExampleApp>) -> actix_web::Result<impl Responder> {
    let mut nodes = BTreeMap::new();
    nodes.insert(app.id, Node {
        addr: app.addr.clone(),
        data: Default::default(),
    });
    let res = app.raft.initialize(nodes).await;
    Ok(Json(res))
}

/// Get the latest metrics of the cluster
#[get("/metrics")]
pub async fn metrics(app: Data<ExampleApp>) -> actix_web::Result<impl Responder> {
    let metrics = app.raft.metrics().borrow().clone();

    let res: Result<RaftMetrics<ExampleTypeConfig>, Infallible> = Ok(metrics);
    Ok(Json(res))
}
*/
