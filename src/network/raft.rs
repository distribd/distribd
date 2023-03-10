use actix_web::post;
use actix_web::web;
use actix_web::web::Data;
use actix_web::Responder;
use openraft::raft::AppendEntriesRequest;
use openraft::raft::InstallSnapshotRequest;
use openraft::raft::VoteRequest;
use web::Json;

use crate::app::RegistryApp;
use crate::RegistryNodeId;
use crate::RegistryTypeConfig;

// --- Raft communication

#[post("/raft-vote")]
pub async fn vote(
    app: Data<RegistryApp>,
    req: Json<VoteRequest<RegistryNodeId>>,
) -> actix_web::Result<impl Responder> {
    let res = app.raft.vote(req.0).await;
    Ok(Json(res))
}

#[post("/raft-append")]
pub async fn append(
    app: Data<RegistryApp>,
    req: Json<AppendEntriesRequest<RegistryTypeConfig>>,
) -> actix_web::Result<impl Responder> {
    let res = app.raft.append_entries(req.0).await;
    Ok(Json(res))
}

#[post("/raft-snapshot")]
pub async fn snapshot(
    app: Data<RegistryApp>,
    req: Json<InstallSnapshotRequest<RegistryTypeConfig>>,
) -> actix_web::Result<impl Responder> {
    let res = app.raft.install_snapshot(req.0).await;
    Ok(Json(res))
}
