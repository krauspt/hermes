use std::ops::Add;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crossbeam_channel as channel;
use prost_types::Any;
use tendermint::account::Id;
use tendermint_testgen::light_block::TMLightBlock;
use tokio::runtime::Runtime;

use ibc::downcast;
use ibc::ics02_client::client_def::AnyClientState;
use ibc::ics07_tendermint::client_state::ClientState as TendermintClientState;
use ibc::ics07_tendermint::consensus_state::ConsensusState as TendermintConsensusState;
use ibc::ics07_tendermint::header::Header as TendermintHeader;
use ibc::ics18_relayer::context::ICS18Context;
use ibc::ics23_commitment::commitment::CommitmentPrefix;
use ibc::ics23_commitment::merkle::MerkleProof;
use ibc::ics24_host::identifier::{ChainId, ClientId};
use ibc::ics24_host::Path;
use ibc::mock::context::MockContext;
use ibc::mock::host::HostType;
use ibc::test_utils::{default_consensus_params, get_dummy_account_id};
use ibc::Height;

use crate::chain::{Chain, QueryResponse};
use crate::config::ChainConfig;
use crate::error::{Error, Kind};
use crate::event::monitor::EventBatch;
use crate::keyring::store::{KeyEntry, KeyRing};
use crate::light_client::{mock::LightClient as MockLightClient, LightClient};
use std::thread;

/// The representation of a mocked chain as the relayer sees it.
/// The relayer runtime and the light client will engage with the MockChain to query/send tx; the
/// primary interface for doing so is captured by `ICS18Context` which this struct can access via
/// the `context` field.
pub struct MockChain {
    config: ChainConfig,
    context: MockContext,
}

impl Chain for MockChain {
    type LightBlock = TMLightBlock;
    type Header = TendermintHeader;
    type ConsensusState = TendermintConsensusState;
    type ClientState = TendermintClientState;

    fn bootstrap(config: ChainConfig, _rt: Arc<Mutex<Runtime>>) -> Result<Self, Error> {
        Ok(MockChain {
            config: config.clone(),
            context: MockContext::new(
                config.id.clone(),
                HostType::SyntheticTendermint,
                50,
                Height::new(config.id.version(), 20),
            ),
        })
    }

    #[allow(clippy::type_complexity)]
    fn init_light_client(
        &self,
    ) -> Result<(Box<dyn LightClient<Self>>, Option<thread::JoinHandle<()>>), Error> {
        let light_client = MockLightClient::new(self);

        Ok((Box::new(light_client), None))
    }

    fn init_event_monitor(
        &self,
        _rt: Arc<Mutex<Runtime>>,
    ) -> Result<
        (
            channel::Receiver<EventBatch>,
            Option<thread::JoinHandle<()>>,
        ),
        Error,
    > {
        let (_, rx) = channel::unbounded();
        Ok((rx, None))
    }

    fn id(&self) -> &ChainId {
        &self.config.id
    }

    fn keybase(&self) -> &KeyRing {
        unimplemented!()
    }

    fn query(&self, _data: Path, _height: Height, _prove: bool) -> Result<QueryResponse, Error> {
        unimplemented!()
    }

    fn send_tx(&mut self, proto_msgs: Vec<Any>) -> Result<String, Error> {
        // Use the ICS18Context interface to submit the set of messages.
        self.context
            .send(proto_msgs)
            .map(|_| "OK".to_string()) // TODO: establish success return codes.
            .map_err(|e| Kind::Rpc.context(e).into())
    }

    fn get_signer(&mut self) -> Result<Id, Error> {
        Ok(get_dummy_account_id())
    }

    fn get_key(&mut self) -> Result<KeyEntry, Error> {
        unimplemented!()
    }

    fn build_client_state(&self, height: Height) -> Result<Self::ClientState, Error> {
        let client_state = Self::ClientState::new(
            self.id().to_string(),
            self.config.trust_threshold,
            self.config.trusting_period,
            self.config.trusting_period.add(Duration::from_secs(1000)),
            Duration::from_millis(3000),
            height,
            Height::zero(),
            default_consensus_params(),
            "upgrade/upgradedClient".to_string(),
            false,
            false,
        )
        .map_err(|e| Kind::BuildClientStateFailure.context(e))?;

        Ok(client_state)
    }

    fn build_consensus_state(
        &self,
        light_block: Self::LightBlock,
    ) -> Result<Self::ConsensusState, Error> {
        Ok(Self::ConsensusState::from(light_block.signed_header.header))
    }

    fn build_header(
        &self,
        _trusted_light_block: Self::LightBlock,
        _target_light_block: Self::LightBlock,
    ) -> Result<Self::Header, Error> {
        unimplemented!()
    }

    fn query_latest_height(&self) -> Result<Height, Error> {
        Ok(self.context.query_latest_height())
    }

    fn query_client_state(
        &self,
        client_id: &ClientId,
        _height: Height,
    ) -> Result<Self::ClientState, Error> {
        // TODO: unclear what are the scenarios where we need to take height into account.
        let any_state = self
            .context
            .query_client_full_state(client_id)
            .ok_or(Kind::EmptyResponseValue)?;
        let client_state = downcast!(any_state => AnyClientState::Tendermint)
            .ok_or_else(|| Kind::Query.context("unexpected client state type"))?;
        Ok(client_state)
    }

    fn query_commitment_prefix(&self) -> Result<CommitmentPrefix, Error> {
        unimplemented!()
    }

    fn proven_client_state(
        &self,
        _client_id: &ClientId,
        _height: Height,
    ) -> Result<(Self::ClientState, MerkleProof), Error> {
        unimplemented!()
    }

    fn proven_client_consensus(
        &self,
        _client_id: &ClientId,
        _consensus_height: Height,
        _height: Height,
    ) -> Result<(Self::ConsensusState, MerkleProof), Error> {
        unimplemented!()
    }
}

// For integration tests with the modules
#[cfg(test)]
pub mod test_utils {
    use std::str::FromStr;
    use std::time::Duration;

    use ibc::ics24_host::identifier::ChainId;

    use crate::config::ChainConfig;

    /// Returns a very minimal chain configuration, to be used in initializing `MockChain`s.
    pub fn get_basic_chain_config(id: &str) -> ChainConfig {
        ChainConfig {
            id: ChainId::from_str(id).unwrap(),
            rpc_addr: "35.192.61.41:26656".parse().unwrap(),
            grpc_addr: "".to_string(),
            account_prefix: "".to_string(),
            key_name: "".to_string(),
            store_prefix: "".to_string(),
            client_ids: vec![],
            gas: 0,
            clock_drift: Duration::from_secs(5),
            trusting_period: Duration::from_secs(14 * 24 * 60 * 60), // 14 days
            trust_threshold: Default::default(),
            peers: None,
        }
    }
}