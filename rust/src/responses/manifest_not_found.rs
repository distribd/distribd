use crate::types::Digest;
use rocket::http::{Header, Status};
use rocket::request::Request;
use rocket::response::{Responder, Response};
use rocket::tokio::fs::File;

pub(crate) struct ManifestNotFound {}

/*
404 Not Found
Content-Type: application/json; charset=utf-8

{
    "errors:" [
        {
            "code": <error code>,
            "message": "<error message>",
            "detail": ...
        },
        ...
    ]
}
*/

impl<'r> Responder<'r, 'static> for ManifestNotFound {
    fn respond_to(self, _req: &Request) -> Result<Response<'static>, Status> {
        let content_type = Header::new("Content-Type", "application/json; charset=utf-8");

        Response::build()
            .header(content_type)
            .status(Status::NotFound)
            .ok()
    }
}
