use openraft::error::InstallSnapshotError;
use openraft::error::NetworkError;
use openraft::error::RPCError;
use openraft::error::RaftError;
use openraft::error::RemoteError;
use openraft::network::RPCOption;
use openraft::raft::AppendEntriesRequest;
use openraft::raft::AppendEntriesResponse;
use openraft::raft::InstallSnapshotRequest;
use openraft::raft::InstallSnapshotResponse;
use openraft::raft::VoteRequest;
use openraft::raft::VoteResponse;
use openraft::BasicNode;
use openraft::RaftNetwork;
use openraft::RaftNetworkFactory;
use serde::de::DeserializeOwned;
use serde::Serialize;

use crate::RegistryNodeId;
use crate::RegistryTypeConfig;

pub struct RegistryNetwork {}

impl RegistryNetwork {
    pub async fn send_rpc<Req, Resp, Err>(
        &self,
        target: RegistryNodeId,
        target_node: &BasicNode,
        uri: &str,
        req: Req,
    ) -> Result<Resp, RPCError<RegistryNodeId, BasicNode, Err>>
    where
        Req: Serialize,
        Err: std::error::Error + DeserializeOwned,
        Resp: DeserializeOwned,
    {
        let addr = &target_node.addr;

        let url = format!("http://{}/{}", addr, uri);

        tracing::debug!("send_rpc to url: {}", url);

        let client = reqwest::Client::new();

        tracing::debug!("client is created for: {}", url);

        let resp = client
            .post(url)
            .json(&req)
            .send()
            .await
            .map_err(|e| RPCError::Network(NetworkError::new(&e)))?;

        tracing::debug!("client.post() is sent");

        let res: Result<Resp, Err> = resp
            .json()
            .await
            .map_err(|e| RPCError::Network(NetworkError::new(&e)))?;

        res.map_err(|e| RPCError::RemoteError(RemoteError::new(target, e)))
    }
}

// NOTE: This could be implemented also on `Arc<RegistryNetwork>`, but since it's empty, implemented
// directly.
impl RaftNetworkFactory<RegistryTypeConfig> for RegistryNetwork {
    type Network = RegistryNetworkConnection;

    async fn new_client(&mut self, target: RegistryNodeId, node: &BasicNode) -> Self::Network {
        RegistryNetworkConnection {
            owner: RegistryNetwork {},
            target,
            target_node: node.clone(),
        }
    }
}

pub struct RegistryNetworkConnection {
    owner: RegistryNetwork,
    target: RegistryNodeId,
    target_node: BasicNode,
}

impl RaftNetwork<RegistryTypeConfig> for RegistryNetworkConnection {
    async fn append_entries(
        &mut self,
        req: AppendEntriesRequest<RegistryTypeConfig>,
        _option: RPCOption,
    ) -> Result<
        AppendEntriesResponse<RegistryNodeId>,
        RPCError<RegistryNodeId, BasicNode, RaftError<RegistryNodeId>>,
    > {
        self.owner
            .send_rpc(self.target, &self.target_node, "raft-append", req)
            .await
    }

    async fn install_snapshot(
        &mut self,
        req: InstallSnapshotRequest<RegistryTypeConfig>,
        _option: RPCOption,
    ) -> Result<
        InstallSnapshotResponse<RegistryNodeId>,
        RPCError<RegistryNodeId, BasicNode, RaftError<RegistryNodeId, InstallSnapshotError>>,
    > {
        self.owner
            .send_rpc(self.target, &self.target_node, "raft-snapshot", req)
            .await
    }

    async fn vote(
        &mut self,
        req: VoteRequest<RegistryNodeId>,
        _option: RPCOption,
    ) -> Result<
        VoteResponse<RegistryNodeId>,
        RPCError<RegistryNodeId, BasicNode, RaftError<RegistryNodeId>>,
    > {
        self.owner
            .send_rpc(self.target, &self.target_node, "raft-vote", req)
            .await
    }
}
