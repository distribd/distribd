use crate::reducer::ReducerDispatch;
use crate::types::Blob;
use crate::types::Manifest;
use crate::types::RegistryAction;
use crate::types::{Digest, RepositoryName};
use crate::webhook::Event;
use chrono::DateTime;
use chrono::Utc;
use pyo3::prelude::*;
use pyo3::types;
use pyo3::types::PyDict;
use pyo3::types::PyTuple;
use pyo3_asyncio::{into_future_with_locals, TaskLocals};
use std::collections::HashMap;
use std::collections::HashSet;
use std::future::Future;
use std::sync::Arc;
use tokio::sync::oneshot;
use tokio::sync::Mutex;

use super::{BlobEntry, ManifestEntry};

pub fn into_future_with_loop(
    event_loop: &PyAny,
    awaitable: &PyAny,
) -> PyResult<impl Future<Output = PyResult<PyObject>> + Send> {
    into_future_with_locals(
        &TaskLocals::new(event_loop).copy_context(event_loop.py())?,
        awaitable,
    )
}

#[derive(PartialEq, Eq, Hash)]
struct Tag {
    repository: RepositoryName,
    tag: String,
}

#[derive(Default)]
struct Store {
    blobs: HashMap<Digest, Blob>,
    manifests: HashMap<Digest, Manifest>,
    tags: HashMap<Tag, Digest>,
}

impl Store {
    fn get_mut_blob(&mut self, digest: &Digest, timestamp: DateTime<Utc>) -> Option<&mut Blob> {
        if let Some(mut blob) = self.blobs.get_mut(digest) {
            blob.updated = timestamp;
            return Some(blob);
        }

        None
    }

    fn get_or_insert_blob(&mut self, digest: Digest, timestamp: DateTime<Utc>) -> &mut Blob {
        let mut blob = self.blobs.entry(digest).or_insert_with(|| Blob {
            created: timestamp,
            updated: timestamp,
            content_type: None,
            size: None,
            dependencies: Some(vec![]),
            locations: HashSet::new(),
            repositories: HashSet::new(),
        });

        blob.updated = timestamp;

        blob
    }

    fn get_mut_manifest(
        &mut self,
        digest: &Digest,
        timestamp: DateTime<Utc>,
    ) -> Option<&mut Manifest> {
        if let Some(mut manifest) = self.manifests.get_mut(digest) {
            manifest.updated = timestamp;
            return Some(manifest);
        }

        None
    }

    fn get_or_insert_manifest(
        &mut self,
        digest: Digest,
        timestamp: DateTime<Utc>,
    ) -> &mut Manifest {
        let mut manifest = self.manifests.entry(digest).or_insert_with(|| Manifest {
            created: timestamp,
            updated: timestamp,
            content_type: None,
            size: None,
            dependencies: Some(vec![]),
            locations: HashSet::new(),
            repositories: HashSet::new(),
        });

        manifest.updated = timestamp;

        manifest
    }
}

pub struct RegistryState {
    state: Mutex<Store>,
    pub machine_identifier: String,
    send_action: PyObject,
    webhook_send: tokio::sync::mpsc::Sender<Event>,
    event_loop: PyObject,
    manifest_waiters: Mutex<HashMap<Digest, Vec<tokio::sync::oneshot::Sender<()>>>>,
    blob_waiters: Mutex<HashMap<Digest, Vec<tokio::sync::oneshot::Sender<()>>>>,
}

impl RegistryState {
    pub fn new(
        send_action: PyObject,
        webhook_send: tokio::sync::mpsc::Sender<Event>,
        machine_identifier: String,
        event_loop: PyObject,
    ) -> RegistryState {
        RegistryState {
            state: Mutex::new(Store::default()),
            send_action,
            webhook_send,
            machine_identifier,
            event_loop,
            manifest_waiters: Mutex::new(HashMap::new()),
            blob_waiters: Mutex::new(HashMap::new()),
        }
    }

    pub fn blob_available(&self, digest: &Digest) {
        let mut waiters = self.blob_waiters.blocking_lock();

        if let Some(blobs) = waiters.remove(digest) {
            info!(
                "State: Wait for blob: {digest} now available. {} waiters to process",
                blobs.len()
            );
            for sender in blobs {
                if sender.send(()).is_err() {
                    warn!("Some blob waiters may have failed: {digest}");
                }
            }
        } else {
            info!("State: Wait for blob: {digest} now available - no active waiters");
        }
    }

    pub async fn wait_for_blob(&self, digest: &Digest) {
        let mut waiters = self.blob_waiters.lock().await;

        if let Some(blob) = self.get_blob_directly(digest) {
            if blob.locations.contains(&self.machine_identifier) {
                // Blob already exists at this endpoint, no need to wait
                info!("State: Wait for blob: {digest} already available");
                return;
            }
        }

        // FIXME: There is a tiny race that we need to fix after registry state is rust native
        // Can the registry state update already be in flight so we won't get a tx but the blob
        // store won't be up to date yet?
        // We can be certain of this when the whole struct is in rust and we can wrap it in a lock.

        let (tx, rx) = oneshot::channel::<()>();

        let values = waiters
            .entry(digest.clone())
            .or_insert_with(std::vec::Vec::new);
        values.push(tx);

        info!("State: Wait for blob: Waiting for {digest} to download");

        match rx.await {
            Ok(_) => {
                info!("State: Wait for blob: {digest}: Download complete");
            }
            Err(err) => {
                warn!("State: Failure whilst waiting for blob to be downloaded: {digest}: {err}");
            }
        }
    }

    pub fn manifest_available(&self, digest: &Digest) {
        let mut waiters = self.manifest_waiters.blocking_lock();

        if let Some(manifests) = waiters.remove(digest) {
            for sender in manifests {
                if sender.send(()).is_err() {
                    warn!("Some manifest waiters may have failed: {digest}");
                }
            }
        }
    }

    pub async fn wait_for_manifest(&self, digest: &Digest) {
        let mut waiters = self.manifest_waiters.lock().await;

        if let Some(manifest) = self.get_manifest_directly(digest) {
            if manifest.locations.contains(&self.machine_identifier) {
                // manifest already exists at this endpoint, no need to wait
                return;
            }
        }

        // FIXME: There is a tiny race that we need to fix after registry state is rust native
        // Can the registry state update already be in flight so we won't get a tx but the manifest
        // store won't be up to date yet?
        // We can be certain of this when the whole struct is in rust and we can wrap it in a lock.

        let (tx, rx) = oneshot::channel::<()>();

        let values = waiters
            .entry(digest.clone())
            .or_insert_with(std::vec::Vec::new);
        values.push(tx);

        match rx.await {
            Ok(_) => (),
            Err(err) => {
                warn!("Failure whilst waiting for manifest to be downloaded: {digest}: {err}");
            }
        }
    }

    pub async fn send_webhook(&self, event: Event) -> bool {
        matches!(self.webhook_send.send(event).await, Ok(_))
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
        let store = self.state.blocking_lock();
        match store.blobs.get(hash) {
            None => false,
            Some(blob) => blob.repositories.contains(repository),
        }
    }

    pub fn get_blob_directly(&self, hash: &Digest) -> Option<Blob> {
        let store = self.state.blocking_lock();
        match store.blobs.get(hash) {
            None => None,
            Some(blob) => Some(blob.clone()),
        }
    }

    pub fn get_blob(&self, repository: &RepositoryName, hash: &Digest) -> Option<Blob> {
        let store = self.state.blocking_lock();
        match store.blobs.get(hash) {
            None => None,
            Some(blob) => {
                if blob.repositories.contains(repository) {
                    return Some(blob.clone());
                }
                None
            }
        }
    }

    pub fn get_manifest_directly(&self, hash: &Digest) -> Option<Manifest> {
        let store = self.state.blocking_lock();
        match store.manifests.get(hash) {
            None => None,
            Some(manifest) => {
                return Some(manifest.clone());
            }
        }
    }
    pub fn get_manifest(&self, repository: &RepositoryName, hash: &Digest) -> Option<Manifest> {
        let store = self.state.blocking_lock();
        match store.manifests.get(hash) {
            None => None,
            Some(manifest) => {
                if manifest.repositories.contains(repository) {
                    return Some(manifest.clone());
                }
                None
            }
        }
    }
    pub fn get_tag(&self, repository: &RepositoryName, tag: &str) -> Option<Digest> {
        let store = self.state.blocking_lock();
        store
            .tags
            .get(&Tag {
                repository: repository.clone(),
                tag: tag.to_string(),
            })
            .cloned()
    }

    pub fn get_tags(&self, _repository: &RepositoryName) -> Option<Vec<String>> {
        // FIXME: Implement this
        Some(vec![])
    }

    pub fn is_manifest_available(&self, repository: &RepositoryName, hash: &Digest) -> bool {
        let store = self.state.blocking_lock();
        match store.manifests.get(hash) {
            None => false,
            Some(manifest) => manifest.repositories.contains(repository),
        }
    }

    pub fn get_orphaned_blobs(&self) -> Vec<BlobEntry> {
        // FIXME: Implement this
        vec![]
    }

    pub fn get_orphaned_manifests(&self) -> Vec<ManifestEntry> {
        // FIXME: Implement this
        vec![]
    }

    pub fn dispatch_actions(&self, actions: Vec<RegistryAction>) {
        let mut store = self.state.blocking_lock();

        for action in actions {
            match action {
                RegistryAction::Empty {} => {}
                RegistryAction::BlobStored {
                    timestamp,
                    user: _,
                    digest,
                    location,
                } => {
                    let blob = store.get_or_insert_blob(digest, timestamp);
                    blob.locations.insert(location);
                }
                RegistryAction::BlobUnstored {
                    timestamp,
                    user: _,
                    digest,
                    location,
                } => {
                    if let Some(blob) = store.get_mut_blob(&digest, timestamp) {
                        blob.locations.remove(&location);
                    }
                }
                RegistryAction::BlobMounted {
                    timestamp,
                    user: _,
                    digest,
                    repository,
                } => {
                    let blob = store.get_or_insert_blob(digest, timestamp);
                    blob.repositories.insert(repository);
                }
                RegistryAction::BlobUnmounted {
                    timestamp,
                    user: _,
                    digest,
                    repository,
                } => {
                    if let Some(blob) = store.get_mut_blob(&digest, timestamp) {
                        blob.repositories.remove(&repository);
                        if blob.repositories.is_empty() {
                            store.blobs.remove(&digest);
                        }
                    }
                }
                RegistryAction::BlobInfo {
                    timestamp,
                    digest,
                    dependencies,
                    content_type,
                } => {
                    if let Some(mut blob) = store.get_mut_blob(&digest, timestamp) {
                        blob.dependencies = Some(dependencies);
                        blob.content_type = Some(content_type);
                    }
                }
                RegistryAction::BlobStat {
                    timestamp,
                    digest,
                    size,
                } => {
                    if let Some(mut blob) = store.get_mut_blob(&digest, timestamp) {
                        blob.size = Some(size);
                    }
                }
                RegistryAction::ManifestStored {
                    timestamp,
                    user: _,
                    digest,
                    location,
                } => {
                    let manifest = store.get_or_insert_manifest(digest, timestamp);
                    manifest.locations.insert(location);
                }
                RegistryAction::ManifestUnstored {
                    timestamp,
                    user: _,
                    digest,
                    location,
                } => {
                    if let Some(manifest) = store.get_mut_manifest(&digest, timestamp) {
                        manifest.locations.remove(&location);
                    }
                }
                RegistryAction::ManifestMounted {
                    timestamp,
                    user: _,
                    digest,
                    repository,
                } => {
                    let manifest = store.get_or_insert_manifest(digest, timestamp);
                    manifest.repositories.insert(repository);
                }
                RegistryAction::ManifestUnmounted {
                    timestamp,
                    user: _,
                    digest,
                    repository,
                } => {
                    if let Some(manifest) = store.get_mut_manifest(&digest, timestamp) {
                        manifest.repositories.remove(&repository);
                        if manifest.repositories.is_empty() {
                            store.manifests.remove(&digest);
                        }
                    }
                }
                RegistryAction::ManifestInfo {
                    timestamp,
                    digest,
                    dependencies,
                    content_type,
                } => {
                    if let Some(mut manifest) = store.get_mut_manifest(&digest, timestamp) {
                        manifest.dependencies = Some(dependencies);
                        manifest.content_type = Some(content_type);
                    }
                }
                RegistryAction::ManifestStat {
                    timestamp,
                    digest,
                    size,
                } => {
                    if let Some(mut manifest) = store.get_mut_manifest(&digest, timestamp) {
                        manifest.size = Some(size);
                    }
                }
                RegistryAction::HashTagged {
                    timestamp: _,
                    user: _,
                    digest,
                    repository,
                    tag,
                } => {
                    store.tags.insert(Tag { repository, tag }, digest);
                }
            }
        }
    }

    pub fn dispatch_entries(&self, entries: Vec<crate::reducer::ReducerDispatch>) {
        for entry in &entries {
            match &entry.1 {
                RegistryAction::BlobStored {
                    timestamp: _,
                    digest,
                    location,
                    user: _,
                } => {
                    if location == &self.machine_identifier {
                        self.blob_available(digest);
                    }
                }
                RegistryAction::ManifestStored {
                    timestamp: _,
                    digest,
                    location,
                    user: _,
                } => {
                    if location == &self.machine_identifier {
                        self.manifest_available(digest);
                    }
                }
                _ => {}
            }
        }
    }
}

pub(crate) fn add_side_effect(reducers: &PyObject, state: Arc<RegistryState>) {
    Python::with_gil(|py| {
        let dispatch_entries = move |args: &PyTuple, _kwargs: Option<&PyDict>| -> PyResult<_> {
            let entries: Vec<ReducerDispatch> = args.get_item(1)?.extract()?;
            state.dispatch_entries(entries);
            Ok(true)
        };
        let dispatch_entries = types::PyCFunction::new_closure(dispatch_entries, py).unwrap();

        let result = reducers.call_method1(py, "add_side_effects", (dispatch_entries,));

        if result.is_err() {
            panic!("Boot failure: Could not registry state side effects")
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup_state() -> RegistryState {
        let evloop = Python::with_gil(|py| PyDict::new(py).to_object(py));
        let send_action = evloop.clone();
        let (tx, _) = tokio::sync::mpsc::channel::<crate::webhook::Event>(100);

        RegistryState::new(
            send_action,
            tx,
            "foo".to_string(),
            evloop,
        )
    }

    // BLOB TESTS

    #[test]
    fn blob_not_available_initially() {
        let state = setup_state();

        let repository = "myrepo".parse().unwrap();
        let digest = "sha256:abcdefg".parse().unwrap();

        assert!(!state.is_blob_available(&repository, &digest))
    }

    #[test]
    fn blob_becomes_available() {
        let state = setup_state();

        let repository = "myrepo".parse().unwrap();
        let digest = "sha256:abcdefg".parse().unwrap();

        state.dispatch_actions(vec![RegistryAction::BlobMounted {
            timestamp: Utc::now(),
            user: "test".to_string(),
            repository: repository,
            digest: digest,
        }]);

        let repository = "myrepo".parse().unwrap();
        let digest = "sha256:abcdefg".parse().unwrap();

        assert!(state.is_blob_available(&repository, &digest));
    }

    #[test]
    fn blob_metadata() {
        let state = setup_state();

        let repository: RepositoryName = "myrepo".parse().unwrap();
        let digest: Digest = "sha256:abcdefg".parse().unwrap();
        let dependency: Digest = "sha256:zxyjkl".parse().unwrap();

        state.dispatch_actions(vec![
            RegistryAction::BlobMounted {
                timestamp: Utc::now(),
                user: "test".to_string(),
                repository: repository.clone(),
                digest: digest.clone(),
            },
            RegistryAction::BlobInfo {
                timestamp: Utc::now(),
                digest: digest,
                content_type: "application/json".to_string(),
                dependencies: vec![dependency],
            },
        ]);

        let digest: Digest = "sha256:abcdefg".parse().unwrap();

        let item = state.get_blob_directly(&digest).unwrap();
        assert_eq!(item.content_type, Some("application/json".to_string()));
        assert_eq!(item.dependencies.as_ref().unwrap().len(), 1);

        let dependencies = vec!["sha256:zxyjkl".parse().unwrap()];
        assert_eq!(item.dependencies, Some(dependencies));
    }

    #[test]
    fn blob_size() {
        let state = setup_state();

        let repository = "myrepo".parse().unwrap();
        let digest = "sha256:abcdefg".parse().unwrap();

        state.dispatch_actions(vec![RegistryAction::BlobMounted {
            timestamp: Utc::now(),
            user: "test".to_string(),
            repository: repository,
            digest: digest,
        }]);

        let digest = "sha256:abcdefg".parse().unwrap();

        state.dispatch_actions(vec![RegistryAction::BlobStat {
            timestamp: Utc::now(),
            digest: digest,
            size: 1234,
        }]);

        let digest: Digest = "sha256:abcdefg".parse().unwrap();
        let item = state.get_blob_directly(&digest).unwrap();

        assert_eq!(item.size, Some(1234));
    }

    #[test]
    fn blob_becomes_unavailable() {
        let state = setup_state();

        let repository = "myrepo".parse().unwrap();
        let digest = "sha256:abcdefg".parse().unwrap();

        state.dispatch_actions(vec![RegistryAction::BlobMounted {
            timestamp: Utc::now(),
            user: "test".to_string(),
            repository: repository,
            digest: digest,
        }]);

        let repository = "myrepo".parse().unwrap();
        let digest = "sha256:abcdefg".parse().unwrap();

        state.dispatch_actions(vec![RegistryAction::BlobUnmounted {
            timestamp: Utc::now(),
            user: "test".to_string(),
            repository: repository,
            digest: digest,
        }]);

        let repository = "myrepo".parse().unwrap();
        let digest = "sha256:abcdefg".parse().unwrap();

        assert!(!state.is_blob_available(&repository, &digest));
    }

    #[test]
    fn blob_becomes_available_again() {
        let state = setup_state();

        // Create node
        let repository = "myrepo".parse().unwrap();
        let digest = "sha256:abcdefg".parse().unwrap();

        state.dispatch_actions(vec![RegistryAction::BlobMounted {
            timestamp: Utc::now(),
            user: "test".to_string(),
            repository: repository,
            digest: digest,
        }]);

        // Make node unavailable
        let repository = "myrepo".parse().unwrap();
        let digest = "sha256:abcdefg".parse().unwrap();

        state.dispatch_actions(vec![RegistryAction::BlobUnmounted {
            timestamp: Utc::now(),
            user: "test".to_string(),
            repository: repository,
            digest: digest,
        }]);

        // Make node available again
        let repository = "myrepo".parse().unwrap();
        let digest = "sha256:abcdefg".parse().unwrap();

        state.dispatch_actions(vec![RegistryAction::BlobMounted {
            timestamp: Utc::now(),
            user: "test".to_string(),
            repository: repository,
            digest: digest,
        }]);

        // Should be visible...
        let repository = "myrepo".parse().unwrap();
        let digest = "sha256:abcdefg".parse().unwrap();

        assert!(state.is_blob_available(&repository, &digest));
    }

    // MANIFEST TESTS

    #[test]
    fn manifest_not_available_initially() {
        let state = setup_state();

        let repository = "myrepo".parse().unwrap();
        let digest = "sha256:abcdefg".parse().unwrap();

        assert!(!state.is_manifest_available(&repository, &digest))
    }

    #[test]
    fn manifest_becomes_available() {
        let state = setup_state();

        let repository = "myrepo".parse().unwrap();
        let digest = "sha256:abcdefg".parse().unwrap();

        state.dispatch_actions(vec![RegistryAction::ManifestMounted {
            timestamp: Utc::now(),
            user: "test".to_string(),
            repository: repository,
            digest: digest,
        }]);

        let repository = "myrepo".parse().unwrap();
        let digest = "sha256:abcdefg".parse().unwrap();

        assert!(state.is_manifest_available(&repository, &digest));
    }

    #[test]
    fn manifest_metadata() {
        let state = setup_state();

        let repository = "myrepo".parse().unwrap();
        let digest = "sha256:abcdefg".parse().unwrap();

        state.dispatch_actions(vec![RegistryAction::ManifestMounted {
            timestamp: Utc::now(),
            user: "test".to_string(),
            repository: repository,
            digest: digest,
        }]);

        let digest = "sha256:abcdefg".parse().unwrap();
        let dependency: Digest = "sha256:zxyjkl".parse().unwrap();

        state.dispatch_actions(vec![RegistryAction::ManifestInfo {
            timestamp: Utc::now(),
            digest: digest,
            content_type: "application/json".to_string(),
            dependencies: vec![dependency],
        }]);

        let digest: Digest = "sha256:abcdefg".parse().unwrap();
        let item = state.get_manifest_directly(&digest).unwrap();

        assert_eq!(item.content_type, Some("application/json".to_string()));
        assert_eq!(item.dependencies.as_ref().unwrap().len(), 1);

        let dependencies = vec!["sha256:zxyjkl".parse().unwrap()];
        assert_eq!(item.dependencies, Some(dependencies));
    }

    #[test]
    fn manifest_size() {
        let state = setup_state();

        let repository = "myrepo".parse().unwrap();
        let digest = "sha256:abcdefg".parse().unwrap();

        state.dispatch_actions(vec![RegistryAction::ManifestMounted {
            timestamp: Utc::now(),
            user: "test".to_string(),
            repository: repository,
            digest: digest,
        }]);

        let digest = "sha256:abcdefg".parse().unwrap();

        state.dispatch_actions(vec![RegistryAction::ManifestStat {
            timestamp: Utc::now(),
            digest: digest,
            size: 1234,
        }]);

        let digest: Digest = "sha256:abcdefg".parse().unwrap();
        let item = state.get_manifest_directly(&digest).unwrap();

        assert_eq!(item.size, Some(1234));
    }

    #[test]
    fn manifest_becomes_unavailable() {
        let state = setup_state();

        let repository = "myrepo".parse().unwrap();
        let digest = "sha256:abcdefg".parse().unwrap();

        state.dispatch_actions(vec![RegistryAction::ManifestMounted {
            timestamp: Utc::now(),
            user: "test".to_string(),
            repository: repository,
            digest: digest,
        }]);

        let repository = "myrepo".parse().unwrap();
        let digest = "sha256:abcdefg".parse().unwrap();

        state.dispatch_actions(vec![RegistryAction::ManifestUnmounted {
            timestamp: Utc::now(),
            user: "test".to_string(),
            repository: repository,
            digest: digest,
        }]);

        let repository = "myrepo".parse().unwrap();
        let digest = "sha256:abcdefg".parse().unwrap();

        assert!(!state.is_manifest_available(&repository, &digest));
    }

    #[test]
    fn manifest_becomes_available_again() {
        let state = setup_state();

        // Create node
        let repository = "myrepo".parse().unwrap();
        let digest = "sha256:abcdefg".parse().unwrap();

        state.dispatch_actions(vec![RegistryAction::ManifestMounted {
            timestamp: Utc::now(),
            user: "test".to_string(),
            repository: repository,
            digest: digest,
        }]);

        // Make node unavailable
        let repository = "myrepo".parse().unwrap();
        let digest = "sha256:abcdefg".parse().unwrap();

        state.dispatch_actions(vec![RegistryAction::ManifestUnmounted {
            timestamp: Utc::now(),
            user: "test".to_string(),
            repository: repository,
            digest: digest,
        }]);

        // Make node available again
        let repository = "myrepo".parse().unwrap();
        let digest = "sha256:abcdefg".parse().unwrap();

        state.dispatch_actions(vec![RegistryAction::ManifestMounted {
            timestamp: Utc::now(),
            user: "test".to_string(),
            repository: repository,
            digest: digest,
        }]);

        // Should be visible...
        let repository = "myrepo".parse().unwrap();
        let digest = "sha256:abcdefg".parse().unwrap();

        assert!(state.is_manifest_available(&repository, &digest));
    }

    #[test]
    fn can_tag_manifest() {
        let state = setup_state();

        // Create node
        let repository = "myrepo".parse().unwrap();
        let digest = "sha256:abcdefg".parse().unwrap();

        state.dispatch_actions(vec![RegistryAction::HashTagged {
            timestamp: Utc::now(),
            user: "test".to_string(),
            repository: repository,
            digest: digest,
            tag: "latest".to_string(),
        }]);

        let repository = "myrepo2".parse().unwrap();
        assert!(matches!(state.get_tags(&repository), None));

        let repository = "myrepo".parse().unwrap();
        assert_eq!(
            state.get_tags(&repository).unwrap(),
            vec!["latest".to_string()]
        );
    }
}
