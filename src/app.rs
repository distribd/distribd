use std::path::Path;
use std::sync::Arc;

use openraft::Config;
use uuid::Uuid;

use crate::extractor::Extractor;
use crate::store::RegistryRequest;
use crate::types::Blob;
use crate::types::Digest;
use crate::types::Manifest;
use crate::types::RegistryAction;
use crate::RegistryNodeId;
use crate::RegistryRaft;
use crate::RegistryStore;

// Representation of an application state. This struct can be shared around to share
// instances of raft, store and more.
pub struct RegistryApp {
    pub id: RegistryNodeId,
    pub addr: String,
    pub raft: RegistryRaft,
    pub store: Arc<RegistryStore>,
    pub config: Arc<Config>,
    pub extractor: Arc<Extractor>,
}

impl RegistryApp {
    pub async fn submit(&self, actions: Vec<RegistryAction>) -> bool {
        let req = RegistryRequest::Transaction { actions: actions };
        let _response = self.raft.client_write(req).await;

        true
    }

    pub async fn get_blob(&self, digest: &Digest) -> Option<Blob> {
        let sm = self.store.state_machine.read().await;
        sm.get_blob(&digest).unwrap()
    }

    pub async fn get_manifest(&self, digest: &Digest) -> Option<Manifest> {
        let sm = self.store.state_machine.read().await;
        sm.get_manifest(&digest).unwrap()
    }

    pub fn get_blob_path(&self, digest: &Digest) -> std::path::PathBuf {
        // FIXME: Hookup to settings
        let images_directory = "tmp".to_string();

        let mut path = std::path::Path::new(&images_directory).to_path_buf();
        let digest_string = &digest.hash;

        path.push("blobs");
        path.push(&digest_string[0..2]);
        path.push(&digest_string[2..4]);
        path.push(&digest_string[4..6]);

        std::fs::create_dir_all(path.clone()).unwrap();

        path.push(&digest_string[6..]);

        path
    }

    pub fn get_manifest_path(&self, digest: &Digest) -> std::path::PathBuf {
        // FIXME: Hookup to settings
        let images_directory = "tmp".to_string();

        let mut path = std::path::Path::new(&images_directory).to_path_buf();
        let digest_string = &digest.hash;

        path.push("manifests");
        path.push(&digest_string[0..2]);
        path.push(&digest_string[2..4]);
        path.push(&digest_string[4..6]);

        std::fs::create_dir_all(path.clone()).unwrap();

        path.push(&digest_string[6..]);

        path
    }

    pub fn get_upload_path(&self, upload_id: &str) -> std::path::PathBuf {
        // FIXME: Hookup to settings
        let images_directory = "tmp".to_string();

        let mut path = Path::new(&images_directory).to_path_buf();
        path.push("uploads");
        path.push(format!("blob-{upload_id}"));

        path
    }

    pub fn get_temp_path(&self) -> std::path::PathBuf {
        // FIXME: Hookup to settings
        let images_directory = "tmp".to_string();

        let upload_id = Uuid::new_v4().as_hyphenated().to_string();

        let mut path = Path::new(&images_directory).to_path_buf();
        path.push("uploads");
        path.push(format!("manifest-{upload_id}"));

        path
    }
}
