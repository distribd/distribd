use crate::headers::Token;
use crate::types::Digest;
use crate::types::RegistryAction;
use crate::types::RegistryState;
use crate::types::RepositoryName;
use crate::utils::get_blob_path;
use crate::utils::get_upload_path;
use rocket::data::Data;
use rocket::http::Header;
use rocket::http::Status;
use rocket::request::Request;
use rocket::response::{Responder, Response};
use rocket::State;
pub(crate) enum Responses {
    AccessDenied {},
    DigestInvalid {},
    UploadInvalid {},
    Ok {
        repository: RepositoryName,
        digest: Digest,
    },
}

impl<'r> Responder<'r, 'static> for Responses {
    fn respond_to(self, _req: &Request) -> Result<Response<'static>, Status> {
        match self {
            Responses::AccessDenied {} => Response::build().status(Status::Forbidden).ok(),
            Responses::DigestInvalid {} => Response::build().status(Status::BadRequest).ok(),
            Responses::UploadInvalid {} => Response::build().status(Status::BadRequest).ok(),
            Responses::Ok { repository, digest } => {
                /*
                204 No Content
                Location: <blob location>
                Content-Range: <start of range>-<end of range, inclusive>
                Content-Length: 0
                Docker-Content-Digest: <digest>
                */

                Response::build()
                    .header(Header::new(
                        "Location",
                        format!("/v2/{repository}/blobs/{digest}"),
                    ))
                    .header(Header::new("Range", "0-0"))
                    .header(Header::new("Content-Length", "0"))
                    .header(Header::new("Docker-Content-Digest", digest.to_string()))
                    .status(Status::NoContent)
                    .ok()
            }
        }
    }
}

#[put("/<repository>/blobs/uploads/<upload_id>?<digest>", data = "<body>")]
pub(crate) async fn put(
    repository: RepositoryName,
    upload_id: String,
    digest: Digest,
    state: &State<RegistryState>,
    token: &State<Token>,
    body: Data<'_>,
) -> Responses {
    let state: &RegistryState = state.inner();

    let token: &Token = token.inner();
    if !token.has_permission(&repository, &"push".to_string()) {
        return Responses::AccessDenied {};
    }

    if digest.algo != "sha256" {
        return Responses::UploadInvalid {};
    }

    let filename = get_upload_path(&state.repository_path, &upload_id);

    if !filename.is_file() {
        return Responses::UploadInvalid {};
    }

    if !crate::views::utils::upload_part(&filename, body).await {
        return Responses::UploadInvalid {};
    }

    // Validate upload
    if !crate::views::utils::validate_hash(&filename, &digest).await {
        return Responses::DigestInvalid {};
    }

    let dest = get_blob_path(&state.repository_path, &digest);

    let stat = match tokio::fs::metadata(&filename).await {
        Ok(result) => result,
        Err(_) => {
            return Responses::UploadInvalid {};
        }
    };

    match std::fs::rename(filename, dest) {
        Ok(_) => {}
        Err(_) => {
            return Responses::UploadInvalid {};
        }
    }

    let actions = vec![
        RegistryAction::BlobMounted {
            digest: digest.clone(),
            repository: repository.clone(),
            user: token.sub.clone(),
        },
        RegistryAction::BlobStat {
            digest: digest.clone(),
            size: stat.len(),
        },
        RegistryAction::BlobStored {
            digest: digest.clone(),
            location: "FIXME".to_string(),
            user: token.sub.clone(),
        },
    ];

    if !state.send_actions(actions).await {
        return Responses::UploadInvalid {};
    }

    Responses::Ok { repository, digest }
}
