use crate::types::Blob;
use crate::types::Manifest;
use crate::types::RegistryAction;
use crate::types::{Digest, RepositoryName};
use crate::webhook::Event;
use pyo3::prelude::*;
use pyo3_asyncio::into_future_with_loop;

pub struct RegistryState {
    pub repository_path: String,
    pub machine_identifier: String,
    send_action: PyObject,
    state: PyObject,
    webhook_send: tokio::sync::mpsc::Sender<Event>,
    event_loop: PyObject,
}

impl RegistryState {
    pub fn new(
        state: PyObject,
        send_action: PyObject,
        repository_path: String,
        webhook_send: tokio::sync::mpsc::Sender<Event>,
        machine_identifier: String,
        event_loop: PyObject,
    ) -> RegistryState {
        RegistryState {
            state,
            send_action,
            repository_path,
            webhook_send,
            machine_identifier,
            event_loop,
        }
    }

    pub async fn send_webhook(&self, event: Event) -> bool {
        match self.webhook_send.send(event).await {
            Ok(_) => true,
            _ => false,
        }
    }

    pub async fn send_actions(&self, actions: Vec<RegistryAction>) -> bool {
        let result = Python::with_gil(|py| {
            let event_loop = self.event_loop.as_ref(py);

            into_future_with_loop(
                event_loop,
                self.send_action.call1(py, (actions,))?.as_ref(py),
            )
        });

        match result {
            Ok(result) => match result.await {
                Ok(value) => match Python::with_gil(|py| value.extract(py)) {
                    Ok(value) => value,
                    _ => false,
                },
                _ => false,
            },
            _ => false,
        }
    }

    pub fn is_blob_available(&self, repository: &RepositoryName, hash: &Digest) -> bool {
        Python::with_gil(|py| {
            let retval = self.state.call_method1(
                py,
                "is_blob_available",
                (repository.to_string(), hash.to_string()),
            );
            match retval {
                Ok(inner) => match inner.extract(py) {
                    Ok(extracted) => extracted,
                    _ => false,
                },
                _ => false,
            }
        })
    }

    pub fn get_blob(&self, repository: &RepositoryName, hash: &Digest) -> Option<Blob> {
        Python::with_gil(|py| {
            let retval =
                self.state
                    .call_method1(py, "get_blob", (repository.to_string(), hash.to_string()));
            match retval {
                Ok(inner) => match inner.extract(py) {
                    Ok(extracted) => Some(extracted),
                    _ => None,
                },
                _ => None,
            }
        })
    }
    pub fn get_manifest(&self, repository: &RepositoryName, hash: &Digest) -> Option<Manifest> {
        Python::with_gil(|py| {
            let retval = self.state.call_method1(
                py,
                "get_manifest",
                (repository.to_string(), hash.to_string()),
            );
            match retval {
                Ok(inner) => match inner.extract(py) {
                    Ok(extracted) => Some(extracted),
                    _ => None,
                },
                _ => None,
            }
        })
    }
    pub fn get_tag(&self, repository: &RepositoryName, tag: &String) -> Option<Digest> {
        Python::with_gil(|py| {
            let retval =
                self.state
                    .call_method1(py, "get_tag", (repository.to_string(), tag.to_string()));
            match retval {
                Ok(inner) => match inner.extract(py) {
                    Ok(extracted) => Some(extracted),
                    _ => None,
                },
                _ => None,
            }
        })
    }

    pub fn is_manifest_available(&self, repository: &RepositoryName, hash: &Digest) -> bool {
        Python::with_gil(|py| {
            let retval = self.state.call_method1(
                py,
                "is_manifest_available",
                (repository.to_string(), hash.to_string()),
            );
            match retval {
                Ok(inner) => match inner.extract(py) {
                    Ok(extracted) => extracted,
                    _ => false,
                },
                _ => false,
            }
        })
    }
}