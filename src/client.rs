use std::collections::BTreeSet;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;

use openraft::error::ForwardToLeader;
use openraft::error::NetworkError;
use openraft::error::RPCError;
use openraft::error::RemoteError;
use openraft::BasicNode;
use openraft::RaftMetrics;
use openraft::TryAsRef;
use reqwest_middleware::ClientBuilder;
use reqwest_middleware::ClientWithMiddleware;
use reqwest_retry::policies::ExponentialBackoff;
use reqwest_retry::RetryTransientMiddleware;
use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde::Serialize;
use tokio::time::timeout;

use crate::network::management::ImportBody;
use crate::typ;
use crate::RegistryNodeId;
use crate::RegistryRequest;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Empty {}

pub struct RegistryClient {
    /// The leader node to send request to.
    ///
    /// All traffic should be sent to the leader in a cluster.
    pub leader: Arc<Mutex<(RegistryNodeId, String)>>,

    pub inner: ClientWithMiddleware,
}

impl RegistryClient {
    /// Create a client with a leader node id and a node manager to get node address by node id.
    pub fn new(
        leader_id: RegistryNodeId,
        leader_addr: String,
        retry_policy: Option<ExponentialBackoff>,
    ) -> Self {
        let mut builder = ClientBuilder::new(reqwest::Client::new());
        if let Some(retry_policy) = retry_policy {
            builder = builder.with(RetryTransientMiddleware::new_with_policy(retry_policy));
        }
        let client = builder.build();

        Self {
            leader: Arc::new(Mutex::new((leader_id, leader_addr))),
            inner: client,
        }
    }

    // --- Application API

    /// Submit a write request to the raft cluster.
    ///
    /// The request will be processed by raft protocol: it will be replicated to a quorum and then
    /// will be applied to state machine.
    ///
    /// The result of applying the request will be returned.
    pub async fn write(
        &self,
        req: &RegistryRequest,
    ) -> Result<typ::ClientWriteResponse, typ::RPCError<typ::ClientWriteError>> {
        self.send_rpc_to_leader("write", Some(req)).await
    }

    /// Read value by key, in an inconsistent mode.
    ///
    /// This method may return stale value because it does not force to read on a legal leader.
    pub async fn read(&self, req: &String) -> Result<String, typ::RPCError> {
        self.do_send_rpc_to_leader("read", Some(req)).await
    }

    /// Consistent Read value by key, in an inconsistent mode.
    ///
    /// This method MUST return consistent value or CheckIsLeaderError.
    pub async fn consistent_read(
        &self,
        req: &String,
    ) -> Result<String, typ::RPCError<typ::CheckIsLeaderError>> {
        self.do_send_rpc_to_leader("consistent_read", Some(req))
            .await
    }

    // --- Cluster management API

    /// Initialize a cluster of only the node that receives this request.
    ///
    /// This is the first step to initialize a cluster.
    /// With a initialized cluster, new node can be added with [`write`].
    /// Then setup replication with [`add_learner`].
    /// Then make the new node a member with [`change_membership`].
    pub async fn init(&self, req: String) -> Result<(), typ::RPCError<typ::InitializeError>> {
        self.do_send_rpc_to_leader("init", Some(&req)).await
    }

    /// Add a node as learner.
    ///
    /// The node to add has to exist, i.e., being added with `write(RegistryRequest::AddNode{})`
    pub async fn add_learner(
        &self,
        req: (RegistryNodeId, String),
    ) -> Result<typ::ClientWriteResponse, typ::RPCError<typ::ClientWriteError>> {
        self.send_rpc_to_leader("add-learner", Some(&req)).await
    }

    /// Change membership to the specified set of nodes.
    ///
    /// All nodes in `req` have to be already added as learner with [`add_learner`],
    /// or an error [`LearnerNotFound`] will be returned.
    pub async fn change_membership(
        &self,
        req: &BTreeSet<RegistryNodeId>,
    ) -> Result<typ::ClientWriteResponse, typ::RPCError<typ::ClientWriteError>> {
        self.send_rpc_to_leader("change-membership", Some(req))
            .await
    }

    pub async fn import(
        &self,
        req: &ImportBody,
    ) -> Result<(), typ::RPCError<typ::ClientWriteError>> {
        self.send_rpc_to_leader("import", Some(req)).await
    }

    pub async fn export(&self) -> Result<ImportBody, typ::RPCError<typ::ClientWriteError>> {
        self.send_rpc_to_leader("export", None::<&()>).await
    }

    /// Get the metrics about the cluster.
    ///
    /// Metrics contains various information about the cluster, such as current leader,
    /// membership config, replication status etc.
    /// See [`RaftMetrics`].
    pub async fn metrics(&self) -> Result<RaftMetrics<RegistryNodeId, BasicNode>, typ::RPCError> {
        self.do_send_rpc_to_leader("metrics", None::<&()>).await
    }

    // --- Internal methods

    /// Send RPC to specified node.
    ///
    /// It sends out a POST request if `req` is Some. Otherwise a GET request.
    /// The remote endpoint must respond a reply in form of `Result<T, E>`.
    /// An `Err` happened on remote will be wrapped in an [`RPCError::RemoteError`].
    async fn do_send_rpc_to_leader<Req, Resp, Err>(
        &self,
        uri: &str,
        req: Option<&Req>,
    ) -> Result<Resp, typ::RPCError<Err>>
    where
        Req: Serialize + 'static,
        Resp: Serialize + DeserializeOwned,
        Err: std::error::Error + Serialize + DeserializeOwned,
    {
        let (leader_id, url) = {
            let t = self.leader.lock().unwrap();
            let target_addr = &t.1;
            (t.0, format!("http://{}/{}", target_addr, uri))
        };

        let fu = if let Some(r) = req {
            tracing::debug!(
                ">>> client send request to {}: {}",
                url,
                serde_json::to_string_pretty(&r).unwrap()
            );
            self.inner.post(url.clone()).json(r)
        } else {
            tracing::debug!(">>> client send request to {}", url,);
            self.inner.get(url.clone())
        }
        .send();

        let res = timeout(Duration::from_millis(3_000), fu).await;
        let resp = match res {
            Ok(x) => x.map_err(|e| RPCError::Network(NetworkError::new(&e)))?,
            Err(timeout_err) => {
                tracing::error!("timeout {} to url: {}", timeout_err, url);
                return Err(RPCError::Network(NetworkError::new(&timeout_err)));
            }
        };

        let res: Result<Resp, typ::RaftError<Err>> = resp
            .json()
            .await
            .map_err(|e| RPCError::Network(NetworkError::new(&e)))?;
        tracing::debug!(
            "<<< client recv reply from {}: {}",
            url,
            serde_json::to_string_pretty(&res).unwrap()
        );

        res.map_err(|e| RPCError::RemoteError(RemoteError::new(leader_id, e)))
    }

    /// Try the best to send a request to the leader.
    ///
    /// If the target node is not a leader, a `ForwardToLeader` error will be
    /// returned and this client will retry at most 3 times to contact the updated leader.
    async fn send_rpc_to_leader<Req, Resp, Err>(
        &self,
        uri: &str,
        req: Option<&Req>,
    ) -> Result<Resp, typ::RPCError<Err>>
    where
        Req: Serialize + 'static,
        Resp: Serialize + DeserializeOwned,
        Err: std::error::Error
            + Serialize
            + DeserializeOwned
            + TryAsRef<typ::ForwardToLeader>
            + Clone,
    {
        // Retry at most 3 times to find a valid leader.
        let mut n_retry = 3;

        loop {
            let res: Result<Resp, typ::RPCError<Err>> = self.do_send_rpc_to_leader(uri, req).await;

            let rpc_err = match res {
                Ok(x) => return Ok(x),
                Err(rpc_err) => rpc_err,
            };

            if let Some(ForwardToLeader {
                leader_id: Some(leader_id),
                leader_node: Some(leader_node),
            }) = rpc_err.forward_to_leader()
            {
                // Update target to the new leader.
                {
                    let mut t = self.leader.lock().unwrap();
                    *t = (*leader_id, leader_node.addr.clone());
                }

                n_retry -= 1;
                if n_retry > 0 {
                    continue;
                }
            }

            return Err(rpc_err);
        }
    }
}
