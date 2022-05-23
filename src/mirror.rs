use crate::config::Configuration;
use crate::mint::Mint;
use crate::raft::RaftEvent;
use crate::rpc::RpcClient;
use crate::types::{Broadcast, Digest, RegistryAction, RegistryState};
use chrono::Utc;
use log::{debug, warn};
use rand::seq::SliceRandom;
use std::path::PathBuf;
use std::{collections::HashSet, sync::Arc};
use tokio::io::AsyncWriteExt;
use tokio::select;

#[derive(Hash, PartialEq, std::cmp::Eq, Debug)]
pub enum MirrorRequest {
    Blob { digest: Digest },
    Manifest { digest: Digest },
}

pub enum MirrorResult {
    Retry {
        request: MirrorRequest,
    },
    Success {
        request: MirrorRequest,
        action: RegistryAction,
    },
    None,
}

impl MirrorRequest {
    pub fn success(self, location: String) -> MirrorResult {
        let action = match self {
            MirrorRequest::Blob { ref digest } => RegistryAction::BlobStored {
                timestamp: Utc::now(),
                digest: digest.clone(),
                location,
                user: String::from("$internal"),
            },
            MirrorRequest::Manifest { ref digest } => RegistryAction::ManifestStored {
                timestamp: Utc::now(),
                digest: digest.clone(),
                location,
                user: String::from("$internal"),
            },
        };

        MirrorResult::Success {
            request: self,
            action,
        }
    }

    pub fn storage_path(&self, images_directory: &str) -> PathBuf {
        match self {
            MirrorRequest::Blob { digest } => crate::utils::get_blob_path(images_directory, digest),
            MirrorRequest::Manifest { digest } => {
                crate::utils::get_manifest_path(images_directory, digest)
            }
        }
    }
}

async fn do_transfer(
    config: Configuration,
    state: Arc<RegistryState>,
    mint: Mint,
    client: reqwest::Client,
    request: MirrorRequest,
) -> MirrorResult {
    let (digest, repository, locations, object_type) = match request {
        MirrorRequest::Blob { ref digest } => match state.get_blob_directly(digest).await {
            Some(blob) => {
                let repository = match blob.repositories.iter().next() {
                    Some(repository) => repository.clone(),
                    None => {
                        debug!("Mirroring: {digest:?}: Digest pending deletion; nothing to do");
                        return MirrorResult::None;
                    }
                };
                if blob.locations.contains(&config.identifier) {
                    debug!("Mirroring: {digest:?}: Already downloaded by this node; nothing to do");
                    return MirrorResult::None;
                }

                (digest, repository, blob.locations, "blobs")
            }
            None => {
                debug!("Mirroring: {digest:?}: missing from graph; nothing to mirror");
                return MirrorResult::None;
            }
        },
        MirrorRequest::Manifest { ref digest } => match state.get_manifest_directly(digest).await {
            Some(manifest) => {
                let repository = match manifest.repositories.iter().next() {
                    Some(repository) => repository.clone(),
                    None => {
                        debug!("Mirroring: {digest:?}: Digest pending deletion; nothing to do");
                        return MirrorResult::None;
                    }
                };
                if manifest.locations.contains(&config.identifier) {
                    debug!("Mirroring: {digest:?}: Already downloaded by this node; nothing to do");
                    return MirrorResult::None;
                }

                (digest, repository, manifest.locations, "manifests")
            }
            None => {
                debug!("Mirroring: {digest:?}: missing from graph; nothing to mirror");
                return MirrorResult::None;
            }
        },
    };

    let mut urls = vec![];
    for peer in &config.peers {
        if !locations.contains(&peer.name) {
            continue;
        }

        let address = &peer.registry.address;
        let port = &peer.registry.port;

        let url = format!("http://{address}:{port}/v2/{repository}/{object_type}/{digest}");
        urls.push(url);
    }

    let url = match urls.choose(&mut rand::thread_rng()) {
        Some(url) => url,
        None => {
            debug!("Mirroring: {digest:?}: Failed to pick a node to mirror from");
            return MirrorResult::None;
        }
    };

    let builder = match mint.enrich_request(client.get(url), repository).await {
        Ok(builder) => builder,
        Err(err) => {
            warn!("Mirroring: Unable to fetch {url} as minting failed: {err}");
            return MirrorResult::Retry { request };
        }
    };

    info!("Mirroring: Will download: {url}");

    let mut resp = match builder.send().await {
        Ok(resp) => resp,
        Err(err) => {
            warn!("Mirroring: Unable to fetch {url}: {err}");
            return MirrorResult::Retry { request };
        }
    };

    let status_code = resp.status();

    if status_code != reqwest::StatusCode::OK {
        warn!("Mirroring: Unable to fetch {url}: {status_code}");
        return MirrorResult::Retry { request };
    }

    let file_name = crate::utils::get_temp_mirror_path(&config.storage);

    let mut file = match tokio::fs::File::create(&file_name).await {
        Ok(file) => {
            debug!("Mirroring: {file_name:?}: Created new file for writing");
            file
        }
        Err(err) => {
            warn!("Mirroring: Failed creating output file for {url}: {err}");
            return MirrorResult::Retry { request };
        }
    };

    let mut hasher = ring::digest::Context::new(&ring::digest::SHA256);

    loop {
        match resp.chunk().await {
            Ok(Some(chunk)) => {
                if let Err(err) = file.write_all(&chunk).await {
                    debug!("Mirroring: Failed write output chunk for {url}: {err}");
                    return MirrorResult::Retry { request };
                }

                debug!("Mirroring: Downloaded {} bytes", chunk.len());
                hasher.update(&chunk);
            }
            Ok(None) => {
                debug!("Mirroring: Finished streaming");
                break;
            }
            Err(err) => {
                debug!("Mirroring: Failed reading chunk for {url}: {err}");
                return MirrorResult::Retry { request };
            }
        };
    }

    if let Err(err) = file.flush().await {
        debug!("Mirroring: Failed to flush output file for {url}: {err}");
        return MirrorResult::Retry { request };
    }

    debug!("Mirroring: Output flushed");

    if let Err(err) = file.sync_all().await {
        debug!("Mirroring: Failed to sync_all output file for {url}: {err}");
        return MirrorResult::Retry { request };
    }

    debug!("Mirroring: Output synced");

    drop(file);

    debug!("Mirroring: File handle dropped");

    let download_digest = Digest::from_sha256(&hasher.finish());

    if digest != &download_digest {
        debug!("Mirroring: Download of {url} complete but wrong digest: {download_digest}");
        return MirrorResult::Retry { request };
    }

    debug!("Mirroring: Download has correct hash ({download_digest} vs {digest})");

    if !crate::views::utils::validate_hash(&file_name, digest).await {
        debug!("Mirroring: Downloaded file for {url} is corrupt");
        return MirrorResult::Retry { request };
    };

    let storage_path = request.storage_path(&config.storage);
    if let Err(err) = tokio::fs::rename(file_name, storage_path).await {
        debug!("Mirroring: Failed to store file for {url}: {err}");
        return MirrorResult::Retry { request };
    }

    debug!("Mirroring: Mirrored {digest}");

    request.success(config.identifier.clone())
}

fn get_tasks_from_raft_event(event: RaftEvent) -> Vec<MirrorRequest> {
    let mut tasks = vec![];

    match event {
        RaftEvent::Committed {
            start_index: _,
            entries,
        } => {
            for entry in &entries {
                match &entry.entry {
                    RegistryAction::BlobStored {
                        timestamp: _,
                        digest,
                        location: _,
                        user: _,
                    } => tasks.push(MirrorRequest::Blob {
                        digest: digest.clone(),
                    }),
                    RegistryAction::ManifestStored {
                        timestamp: _,
                        digest,
                        location: _,
                        user: _,
                    } => {
                        tasks.push(MirrorRequest::Manifest {
                            digest: digest.clone(),
                        });
                    }
                    _ => {}
                }
            }
        }
    }

    tasks
}

pub(crate) fn start_mirroring(
    config: Configuration,
    state: Arc<RegistryState>,
    submission: Arc<RpcClient>,
    mut broadcasts: tokio::sync::broadcast::Receiver<Broadcast>,
) {
    let mut rx = state.events.subscribe();

    let mint = Mint::new(config.mirroring.clone());

    let client = reqwest::Client::builder()
        .user_agent("distribd/mirror")
        .build()
        .unwrap();

    let mut requests = HashSet::<MirrorRequest>::new();

    tokio::spawn(async move {
        loop {
            select! {
                _ = broadcasts.recv() => {
                    debug!("Mirroring: Stopping in response to SIGINT");
                    return;
                },
                _ = tokio::time::sleep(core::time::Duration::from_secs(10)) => {},
                Ok(event) = rx.recv() => {
                    requests.extend(get_tasks_from_raft_event(event));
                }
            };

            info!("Mirroring: There are {} mirroring tasks", requests.len());

            // FIXME: Ideally we'd have some worker pool here are download a bunch
            // of objects in parallel.

            let tasks: Vec<MirrorRequest> = requests.drain().collect();
            for task in tasks {
                let client = client.clone();
                let result =
                    do_transfer(config.clone(), state.clone(), mint.clone(), client, task).await;

                match result {
                    MirrorResult::Retry { request } => {
                        requests.insert(request);
                    }
                    MirrorResult::Success { action, request } => {
                        if !submission.send(vec![action]).await {
                            requests.insert(request);
                            debug!("Mirroring: Raft transaction failed");
                        } else {
                            debug!("Mirroring: Download logged to raft");
                        };
                    }
                    MirrorResult::None => {}
                }
            }
        }
    });
}
