/*
use openraft::error::CheckIsLeaderError;
use openraft::error::Infallible;
use openraft::raft::ClientWriteRequest;
use openraft::EntryPayload;

use crate::app::RegistryApp;
use crate::store::RegistryRequest;
use crate::NodeId;


 * Application API
 *
 * This is where you place your application, you can use the Registry below to create your
 * API. The current implementation:
 *
 *  - `POST - /write` saves a value in a key and sync the nodes.
 *  - `POST - /read` attempt to find a value from a given key.
#[post("/write")]
pub async fn write(app: Data<RegistryApp>, req: Json<RegistryRequest>) -> actix_web::Result<impl Responder> {
    let request = ClientWriteRequest::new(EntryPayload::Normal(req.0));
    let response = app.raft.client_write(request).await;
    Ok(Json(response))
}

#[post("/read")]
pub async fn read(app: Data<RegistryApp>, req: Json<String>) -> actix_web::Result<impl Responder> {
    let state_machine = app.store.state_machine.read().await;
    let key = req.0;
    let value = match key.as_str() {
        "orderbook_orders" => serde_json::to_string(&state_machine.to_content().orders).unwrap_or_default(),
        "orderbook_sequance" => state_machine.orderbook.sequance.to_string(),
        _ => state_machine.data.get(&key).cloned().unwrap_or_default(),
    };
    let res: Result<String, Infallible> = Ok(value);
    Ok(Json(res))
}

#[post("/consistent_read")]
pub async fn consistent_read(app: Data<RegistryApp>, req: Json<String>) -> actix_web::Result<impl Responder> {
    let ret = app.raft.is_leader().await;

    match ret {
        Ok(_) => {
            let state_machine = app.store.state_machine.read().await;
            let key = req.0;
            let value = state_machine.data.get(&key).cloned();

            let res: Result<String, CheckIsLeaderError<NodeId>> = Ok(value.unwrap_or_default());
            Ok(Json(res))
        }
        Err(e) => Ok(Json(Err(e))),
    }
}

*/
