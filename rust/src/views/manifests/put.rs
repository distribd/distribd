use crate::extractor::Extractor;
use crate::headers::ContentType;
use crate::headers::Token;
use crate::types::Digest;
use crate::types::RegistryAction;
use crate::types::RegistryState;
use crate::types::RepositoryName;
use crate::utils::get_manifest_path;
use crate::views::utils::get_hash;
use crate::webhook::Event;
use rocket::data::Data;
use rocket::http::Header;
use rocket::http::Status;
use rocket::request::Request;
use rocket::response::{Responder, Response};
use rocket::State;
pub(crate) enum Responses {
    AccessDenied {},
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
            Responses::UploadInvalid {} => Response::build().status(Status::BadRequest).ok(),
            Responses::Ok { repository, digest } => {
                /*
                201 Created
                Location: <url>
                Content-Length: 0
                Docker-Content-Digest: <digest>
                */
                let location = Header::new(
                    "Location",
                    format!("/v2/{}/manifests/{}", repository, digest),
                );
                let length = Header::new("Content-Length", "0");
                let upload_uuid = Header::new("Docker-Content-Digest", digest.to_string());

                Response::build()
                    .header(location)
                    .header(length)
                    .header(upload_uuid)
                    .status(Status::Created)
                    .ok()
            }
        }
    }
}

#[put("/<repository>/manifests/<tag>", data = "<body>")]
pub(crate) async fn put(
    repository: RepositoryName,
    tag: String,
    state: &State<RegistryState>,
    extractor: &State<Extractor>,
    content_type: ContentType,
    token: &State<Token>,
    body: Data<'_>,
) -> Responses {
    let state: &RegistryState = state.inner();
    let extractor: &Extractor = extractor.inner();

    let token: &Token = token.inner();
    if !token.has_permission(&repository, &"push".to_string()) {
        return Responses::AccessDenied {};
    }

    let upload_path = crate::utils::get_temp_path(&state.repository_path);

    if !crate::views::utils::upload_part(&upload_path, body).await {
        return Responses::UploadInvalid {};
    }

    let size = match tokio::fs::metadata(&upload_path).await {
        Ok(result) => result.len(),
        Err(_) => {
            return Responses::UploadInvalid {};
        }
    };

    let digest = match get_hash(&upload_path).await {
        Some(digest) => digest,
        _ => {
            return Responses::UploadInvalid {};
        }
    };
    let extracted = extractor
        .extract(
            state,
            &repository,
            &digest,
            &content_type.content_type,
            &"".to_string(),
        )
        .await;

    let mut actions = vec![
        RegistryAction::ManifestMounted {
            digest: digest.clone(),
            repository: repository.clone(),
            user: token.sub.clone(),
        },
        RegistryAction::ManifestStored {
            digest: digest.clone(),
            location: "FIXME".to_string(),
            user: token.sub.clone(),
        },
        RegistryAction::ManifestStat {
            digest: digest.clone(),
            size,
        },
    ];

    let extracted = match extracted {
        Ok(extracted_actions) => extracted_actions,
        _ => {
            return Responses::UploadInvalid {};
        }
    };
    actions.append(&mut extracted.clone());
    actions.append(&mut vec![RegistryAction::HashTagged {
        repository: repository.clone(),
        digest: digest.clone(),
        tag: tag.clone(),
        user: token.sub.clone(),
    }]);

    let dest = get_manifest_path(&state.repository_path, &digest);

    match std::fs::rename(upload_path, dest) {
        Ok(_) => {}
        Err(_) => {
            return Responses::UploadInvalid {};
        }
    }

    if !state.send_actions(actions).await {
        return Responses::UploadInvalid {};
    }

    state
        .send_webhook(Event {
            repository: repository.clone(),
            digest: digest.clone(),
            tag,
            content_type: "".to_string(),
        })
        .await;

    Responses::Ok { repository, digest }
}
