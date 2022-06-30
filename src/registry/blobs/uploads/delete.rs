use crate::config::Configuration;
use crate::headers::Token;
use crate::types::RepositoryName;
use crate::utils::get_upload_path;
use log::warn;
use rocket::delete;
use rocket::http::Header;
use rocket::http::Status;
use rocket::request::Request;
use rocket::response::{Responder, Response};
use rocket::State;
use std::io::Cursor;

pub(crate) enum Responses {
    MustAuthenticate { challenge: String },
    AccessDenied {},
    UploadInvalid {},
    Ok {},
}

impl<'r> Responder<'r, 'static> for Responses {
    fn respond_to(self, _req: &Request) -> Result<Response<'static>, Status> {
        match self {
            Responses::MustAuthenticate { challenge } => {
                let body = crate::registry::utils::simple_oci_error(
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
                let body = crate::registry::utils::simple_oci_error(
                    "DENIED",
                    "requested access to the resource is denied",
                );
                Response::build()
                    .header(Header::new("Content-Length", body.len().to_string()))
                    .sized_body(body.len(), Cursor::new(body))
                    .status(Status::Forbidden)
                    .ok()
            }
            Responses::UploadInvalid {} => Response::build().status(Status::BadRequest).ok(),
            Responses::Ok {} => {
                /*
                204 No Content
                Content-Length: 0
                */
                Response::build()
                    .header(Header::new("Content-Length", "0"))
                    .status(Status::NoContent)
                    .ok()
            }
        }
    }
}

#[delete("/<repository>/blobs/uploads/<upload_id>")]
pub(crate) async fn delete(
    repository: RepositoryName,
    upload_id: String,
    config: &State<Configuration>,
    token: Token,
) -> Responses {
    let config: &Configuration = config.inner();

    if !token.validated_token {
        return Responses::MustAuthenticate {
            challenge: token.get_push_challenge(repository),
        };
    }

    if !token.has_permission(&repository, "push") {
        return Responses::AccessDenied {};
    }

    let filename = get_upload_path(&config.storage, &upload_id);

    if !filename.is_file() {
        return Responses::UploadInvalid {};
    }

    if let Err(err) = tokio::fs::remove_file(filename).await {
        warn!("Error whilst deleting file: {err:?}");
        return Responses::UploadInvalid {};
    }

    Responses::Ok {}
}
