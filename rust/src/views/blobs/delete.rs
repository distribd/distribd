use crate::types::Digest;
use crate::types::RegistryAction;
use crate::types::RegistryState;
use crate::types::RepositoryName;
use rocket::http::Header;
use rocket::http::Status;
use rocket::request::Request;
use rocket::response::{Responder, Response};
use rocket::State;

pub(crate) enum Responses {
    AccessDenied {},
    NotFound {},
    Failed {},
    Ok {},
}

impl<'r> Responder<'r, 'static> for Responses {
    fn respond_to(self, _req: &Request) -> Result<Response<'static>, Status> {
        match self {
            Responses::AccessDenied {} => Response::build().status(Status::Forbidden).ok(),
            Responses::NotFound {} => Response::build().status(Status::NotFound).ok(),
            Responses::Failed {} => Response::build().status(Status::NotFound).ok(),
            Responses::Ok {} => Response::build()
                .header(Header::new("Content-Length", "0"))
                .status(Status::Accepted)
                .ok(),
        }
    }
}

#[delete("/<repository>/blobs/<digest>")]
pub(crate) async fn delete(
    repository: RepositoryName,
    digest: Digest,
    state: &State<RegistryState>,
) -> Responses {
    let state: &RegistryState = state.inner();

    if !state.check_token(&repository, &"pull".to_string()) {
        return Responses::AccessDenied {};
    }

    if !state.is_blob_available(&repository, &digest) {
        return Responses::NotFound {};
    }

    let actions = vec![RegistryAction::BlobUnmounted {
        digest,
        repository,
        user: "FIXME".to_string(),
    }];

    if !state.send_actions(actions).await {
        return Responses::Failed {};
    }

    Responses::Ok {}
}
