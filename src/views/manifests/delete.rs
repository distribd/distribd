use crate::headers::Token;
use crate::types::Digest;
use crate::types::RegistryAction;
use crate::types::RegistryState;
use crate::types::RepositoryName;
use rocket::http::Header;
use rocket::http::Status;
use rocket::request::Request;
use rocket::response::{Responder, Response};
use rocket::State;
use std::io::Cursor;
pub(crate) enum Responses {
    MustAuthenticate { challenge: String },
    AccessDenied {},
    NotFound {},
    Failed {},
    Ok {},
}

impl<'r> Responder<'r, 'static> for Responses {
    fn respond_to(self, _req: &Request) -> Result<Response<'static>, Status> {
        match self {
            Responses::MustAuthenticate { challenge } => {
                let body = crate::views::utils::simple_oci_error(
                    "UNAUTHORIZED",
                    "authentication required",
                );
                Response::build()
                    .header(Header::new("Content-Length", body.len().to_string()))
                    .header(Header::new("Www-Authenticate", challenge))
                    .sized_body(body.len(), Cursor::new(body))
                    .status(Status::Unauthorized)
                    .ok()
            }
            Responses::AccessDenied {} => {
                let body = crate::views::utils::simple_oci_error(
                    "DENIED",
                    "requested access to the resource is denied",
                );
                Response::build()
                    .header(Header::new("Content-Length", body.len().to_string()))
                    .sized_body(body.len(), Cursor::new(body))
                    .status(Status::Forbidden)
                    .ok()
            }
            Responses::NotFound {} => Response::build().status(Status::NotFound).ok(),
            Responses::Failed {} => Response::build().status(Status::NotFound).ok(),
            Responses::Ok {} => Response::build()
                .header(Header::new("Content-Length", "0"))
                .status(Status::Accepted)
                .ok(),
        }
    }
}
#[delete("/<repository>/manifests/<digest>")]
pub(crate) async fn delete(
    repository: RepositoryName,
    digest: Digest,
    state: &State<RegistryState>,
    token: Token,
) -> Responses {
    let state: &RegistryState = state.inner();

    if !token.validated_token {
        return Responses::MustAuthenticate {
            challenge: token.get_push_challenge(repository),
        };
    }

    if !token.has_permission(&repository, &"push".to_string()) {
        return Responses::AccessDenied {};
    }

    if !state.is_manifest_available(&repository, &digest) {
        return Responses::NotFound {};
    }

    let actions = vec![RegistryAction::ManifestUnmounted {
        digest,
        repository,
        user: token.sub.clone(),
    }];

    if !state.send_actions(actions).await {
        return Responses::Failed {};
    }

    Responses::Ok {}
}

#[delete("/<repository>/manifests/<tag>", rank = 2)]
pub(crate) async fn delete_by_tag(
    repository: RepositoryName,
    tag: String,
    state: &State<RegistryState>,
    token: Token,
) -> Responses {
    let state: &RegistryState = state.inner();

    if !token.validated_token {
        return Responses::MustAuthenticate {
            challenge: token.get_push_challenge(repository),
        };
    }

    if !token.has_permission(&repository, &"push".to_string()) {
        return Responses::AccessDenied {};
    }

    let digest = match state.get_tag(&repository, &tag) {
        Some(tag) => tag,
        None => return Responses::NotFound {},
    };

    if !state.is_manifest_available(&repository, &digest) {
        return Responses::NotFound {};
    }

    let actions = vec![RegistryAction::ManifestUnmounted {
        digest,
        repository,
        user: token.sub.clone(),
    }];

    if !state.send_actions(actions).await {
        return Responses::Failed {};
    }

    Responses::Ok {}
}
