use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use ibc_proto::google::protobuf::Any as ProtobufAny;
use tendermint::{block, consensus, evidence, public_key::Algorithm};

use crate::applications::ics20_fungible_token_transfer::context::{
    AccountReader, BankKeeper, BankReader, Ics20Context, Ics20Keeper, Ics20Reader,
};
use crate::applications::ics20_fungible_token_transfer::relay_application_logic::send_transfer::send_transfer;
use crate::applications::ics20_fungible_token_transfer::{
    error::Error as Ics20Error, DenomTrace, HashedDenom, IbcCoin,
};
use crate::core::ics02_client::client_consensus::AnyConsensusState;
use crate::core::ics02_client::client_state::AnyClientState;
use crate::core::ics02_client::error::Error as Ics02Error;
use crate::core::ics03_connection::connection::ConnectionEnd;
use crate::core::ics03_connection::error::Error as Ics03Error;
use crate::core::ics04_channel::channel::{ChannelEnd, Counterparty, Order};
use crate::core::ics04_channel::commitment::{AcknowledgementCommitment, PacketCommitment};
use crate::core::ics04_channel::context::{ChannelKeeper, ChannelReader};
use crate::core::ics04_channel::error::Error;
use crate::core::ics04_channel::packet::{Receipt, Sequence};
use crate::core::ics04_channel::Version;
use crate::core::ics05_port::capabilities::{
    Capability, CapabilityName, ChannelCapability, PortCapability,
};
use crate::core::ics05_port::context::{
    CapabilityKeeper, CapabilityReader, PortKeeper, PortReader,
};
use crate::core::ics05_port::error::Error as PortError;
use crate::core::ics24_host::identifier::{ChannelId, ClientId, ConnectionId, PortId};
use crate::core::ics26_routing::context::{Module, ModuleId, ModuleOutputBuilder};
use crate::mock::context::MockIbcStore;
use crate::prelude::*;
use crate::signer::Signer;
use crate::timestamp::Timestamp;
use crate::Height;

// Needed in mocks.
pub fn default_consensus_params() -> consensus::Params {
    consensus::Params {
        block: block::Size {
            max_bytes: 22020096,
            max_gas: -1,
            time_iota_ms: 1000,
        },
        evidence: evidence::Params {
            max_age_num_blocks: 100000,
            max_age_duration: evidence::Duration(core::time::Duration::new(48 * 3600, 0)),
            max_bytes: 0,
        },
        validator: consensus::params::ValidatorParams {
            pub_key_types: vec![Algorithm::Ed25519],
        },
        version: Some(consensus::params::VersionParams::default()),
    }
}

pub fn get_dummy_proof() -> Vec<u8> {
    "Y29uc2Vuc3VzU3RhdGUvaWJjb25lY2xpZW50LzIy"
        .as_bytes()
        .to_vec()
}

pub fn get_dummy_account_id() -> Signer {
    "0CDA3F47EF3C4906693B170EF650EB968C5F4B2C".parse().unwrap()
}

pub fn get_dummy_bech32_account() -> String {
    "cosmos1wxeyh7zgn4tctjzs0vtqpc6p5cxq5t2muzl7ng".to_string()
}

#[derive(Debug)]
pub struct DummyTransferModule {
    ibc_store: Arc<Mutex<MockIbcStore>>,
    denom_traces: BTreeMap<HashedDenom, DenomTrace>,
}

impl DummyTransferModule {
    pub fn new(ibc_store: Arc<Mutex<MockIbcStore>>) -> Self {
        Self {
            ibc_store,
            denom_traces: Default::default(),
        }
    }
}

impl Module for DummyTransferModule {
    fn on_chan_open_try(
        &mut self,
        _output: &mut ModuleOutputBuilder,
        _order: Order,
        _connection_hops: &[ConnectionId],
        _port_id: &PortId,
        _channel_id: &ChannelId,
        _channel_cap: &ChannelCapability,
        _counterparty: &Counterparty,
        _version: &Version,
        counterparty_version: &Version,
    ) -> Result<Version, Error> {
        Ok(counterparty_version.clone())
    }

    fn deliver(&mut self, output: &mut ModuleOutputBuilder, msg: ProtobufAny) -> Result<(), Error> {
        let msg = msg
            .try_into()
            .map_err(|e: Ics20Error| Error::app_module(e.to_string()))?;
        send_transfer(self, output, msg).map_err(|e: Ics20Error| Error::app_module(e.to_string()))
    }
}

impl Ics20Keeper for DummyTransferModule {
    type AccountId = Signer;

    fn set_denom_trace(&mut self, denom_trace: &DenomTrace) -> Result<(), Ics20Error> {
        self.denom_traces
            .insert(denom_trace.hashed(), denom_trace.clone());
        Ok(())
    }
}

impl ChannelKeeper for DummyTransferModule {
    fn store_packet_commitment(
        &mut self,
        key: (PortId, ChannelId, Sequence),
        commitment: PacketCommitment,
    ) -> Result<(), Error> {
        self.ibc_store
            .lock()
            .unwrap()
            .packet_commitment
            .insert(key, commitment);
        Ok(())
    }

    fn delete_packet_commitment(
        &mut self,
        _key: (PortId, ChannelId, Sequence),
    ) -> Result<(), Error> {
        unimplemented!()
    }

    fn store_packet_receipt(
        &mut self,
        _key: (PortId, ChannelId, Sequence),
        _receipt: Receipt,
    ) -> Result<(), Error> {
        unimplemented!()
    }

    fn store_packet_acknowledgement(
        &mut self,
        _key: (PortId, ChannelId, Sequence),
        _ack: AcknowledgementCommitment,
    ) -> Result<(), Error> {
        unimplemented!()
    }

    fn delete_packet_acknowledgement(
        &mut self,
        _key: (PortId, ChannelId, Sequence),
    ) -> Result<(), Error> {
        unimplemented!()
    }

    fn store_connection_channels(
        &mut self,
        _conn_id: ConnectionId,
        _port_channel_id: &(PortId, ChannelId),
    ) -> Result<(), Error> {
        unimplemented!()
    }

    fn store_channel(
        &mut self,
        _port_channel_id: (PortId, ChannelId),
        _channel_end: &ChannelEnd,
    ) -> Result<(), Error> {
        unimplemented!()
    }

    fn store_next_sequence_send(
        &mut self,
        port_channel_id: (PortId, ChannelId),
        seq: Sequence,
    ) -> Result<(), Error> {
        self.ibc_store
            .lock()
            .unwrap()
            .next_sequence_send
            .insert(port_channel_id, seq);
        Ok(())
    }

    fn store_next_sequence_recv(
        &mut self,
        _port_channel_id: (PortId, ChannelId),
        _seq: Sequence,
    ) -> Result<(), Error> {
        unimplemented!()
    }

    fn store_next_sequence_ack(
        &mut self,
        _port_channel_id: (PortId, ChannelId),
        _seq: Sequence,
    ) -> Result<(), Error> {
        unimplemented!()
    }

    fn increase_channel_counter(&mut self) {
        unimplemented!()
    }
}

impl PortKeeper for DummyTransferModule {}

impl CapabilityKeeper for DummyTransferModule {
    fn new_capability(&mut self, _name: CapabilityName) -> Result<Capability, PortError> {
        unimplemented!()
    }

    fn claim_capability(&mut self, _name: CapabilityName, _capability: Capability) {
        unimplemented!()
    }

    fn release_capability(&mut self, _name: CapabilityName, _capability: Capability) {
        unimplemented!()
    }
}

impl PortReader for DummyTransferModule {
    fn lookup_module_by_port(
        &self,
        _port_id: &PortId,
    ) -> Result<(ModuleId, PortCapability), PortError> {
        unimplemented!()
    }
}

impl CapabilityReader for DummyTransferModule {
    fn get_capability(&self, _name: &CapabilityName) -> Result<Capability, PortError> {
        unimplemented!()
    }

    fn authenticate_capability(
        &self,
        _name: &CapabilityName,
        _capability: &Capability,
    ) -> Result<(), PortError> {
        unimplemented!()
    }
}

impl BankKeeper for DummyTransferModule {
    type AccountId = Signer;

    fn send_coins(
        &mut self,
        _from: &Self::AccountId,
        _to: &Self::AccountId,
        _amt: &IbcCoin,
    ) -> Result<(), Ics20Error> {
        Ok(())
    }

    fn mint_coins(&mut self, _module: &Self::AccountId, _amt: &IbcCoin) -> Result<(), Ics20Error> {
        Ok(())
    }

    fn burn_coins(&mut self, _module: &Self::AccountId, _amt: &IbcCoin) -> Result<(), Ics20Error> {
        Ok(())
    }

    fn send_coins_from_module_to_account(
        &mut self,
        _module: &Self::AccountId,
        _to: &Self::AccountId,
        _amt: &IbcCoin,
    ) -> Result<(), Ics20Error> {
        Ok(())
    }

    fn send_coins_from_account_to_module(
        &mut self,
        _from: &Self::AccountId,
        _module: &Self::AccountId,
        _amt: &IbcCoin,
    ) -> Result<(), Ics20Error> {
        Ok(())
    }
}

impl Ics20Reader for DummyTransferModule {
    type AccountId = Signer;

    fn get_port(&self) -> Result<PortId, Ics20Error> {
        Ok(PortId::transfer())
    }

    fn is_send_enabled(&self) -> bool {
        true
    }

    fn is_receive_enabled(&self) -> bool {
        true
    }

    fn get_denom_trace(&self, denom_hash: &HashedDenom) -> Option<DenomTrace> {
        self.denom_traces.get(denom_hash).map(Clone::clone)
    }
}

impl BankReader for DummyTransferModule {
    type AccountId = Signer;

    fn is_blocked_account(&self, _account: &Self::AccountId) -> bool {
        false
    }

    fn get_transfer_account(&self) -> Self::AccountId {
        get_dummy_account_id()
    }
}

impl AccountReader for DummyTransferModule {
    type AccountId = Signer;
    type Address = Signer;

    fn get_account(&self, address: &Self::Address) -> Option<Self::AccountId> {
        Some(address.clone())
    }
}

impl ChannelReader for DummyTransferModule {
    fn channel_end(&self, pcid: &(PortId, ChannelId)) -> Result<ChannelEnd, Error> {
        match self.ibc_store.lock().unwrap().channels.get(pcid) {
            Some(channel_end) => Ok(channel_end.clone()),
            None => Err(Error::channel_not_found(pcid.0.clone(), pcid.1)),
        }
    }

    fn connection_end(&self, cid: &ConnectionId) -> Result<ConnectionEnd, Error> {
        match self.ibc_store.lock().unwrap().connections.get(cid) {
            Some(connection_end) => Ok(connection_end.clone()),
            None => Err(Ics03Error::connection_not_found(cid.clone())),
        }
        .map_err(Error::ics03_connection)
    }

    fn connection_channels(&self, _cid: &ConnectionId) -> Result<Vec<(PortId, ChannelId)>, Error> {
        unimplemented!()
    }

    fn client_state(&self, client_id: &ClientId) -> Result<AnyClientState, Error> {
        match self.ibc_store.lock().unwrap().clients.get(client_id) {
            Some(client_record) => client_record
                .client_state
                .clone()
                .ok_or_else(|| Ics02Error::client_not_found(client_id.clone())),
            None => Err(Ics02Error::client_not_found(client_id.clone())),
        }
        .map_err(|e| Error::ics03_connection(Ics03Error::ics02_client(e)))
    }

    fn client_consensus_state(
        &self,
        client_id: &ClientId,
        height: Height,
    ) -> Result<AnyConsensusState, Error> {
        match self.ibc_store.lock().unwrap().clients.get(client_id) {
            Some(client_record) => match client_record.consensus_states.get(&height) {
                Some(consensus_state) => Ok(consensus_state.clone()),
                None => Err(Ics02Error::consensus_state_not_found(
                    client_id.clone(),
                    height,
                )),
            },
            None => Err(Ics02Error::consensus_state_not_found(
                client_id.clone(),
                height,
            )),
        }
        .map_err(|e| Error::ics03_connection(Ics03Error::ics02_client(e)))
    }

    fn authenticated_capability(&self, _port_id: &PortId) -> Result<ChannelCapability, Error> {
        Ok(Capability::new().into())
    }

    fn get_next_sequence_send(
        &self,
        port_channel_id: &(PortId, ChannelId),
    ) -> Result<Sequence, Error> {
        match self
            .ibc_store
            .lock()
            .unwrap()
            .next_sequence_send
            .get(port_channel_id)
        {
            Some(sequence) => Ok(*sequence),
            None => Err(Error::missing_next_send_seq(port_channel_id.clone())),
        }
    }

    fn get_next_sequence_recv(
        &self,
        _port_channel_id: &(PortId, ChannelId),
    ) -> Result<Sequence, Error> {
        unimplemented!()
    }

    fn get_next_sequence_ack(
        &self,
        _port_channel_id: &(PortId, ChannelId),
    ) -> Result<Sequence, Error> {
        unimplemented!()
    }

    fn get_packet_commitment(
        &self,
        _key: &(PortId, ChannelId, Sequence),
    ) -> Result<PacketCommitment, Error> {
        unimplemented!()
    }

    fn get_packet_receipt(&self, _key: &(PortId, ChannelId, Sequence)) -> Result<Receipt, Error> {
        unimplemented!()
    }

    fn get_packet_acknowledgement(
        &self,
        _key: &(PortId, ChannelId, Sequence),
    ) -> Result<AcknowledgementCommitment, Error> {
        unimplemented!()
    }

    fn hash(&self, value: Vec<u8>) -> Vec<u8> {
        use sha2::Digest;

        sha2::Sha256::digest(value).to_vec()
    }

    fn host_height(&self) -> Height {
        Height::zero()
    }

    fn host_consensus_state(&self, _height: Height) -> Result<AnyConsensusState, Error> {
        unimplemented!()
    }

    fn pending_host_consensus_state(&self) -> Result<AnyConsensusState, Error> {
        unimplemented!()
    }

    fn client_update_time(
        &self,
        _client_id: &ClientId,
        _height: Height,
    ) -> Result<Timestamp, Error> {
        unimplemented!()
    }

    fn client_update_height(
        &self,
        _client_id: &ClientId,
        _height: Height,
    ) -> Result<Height, Error> {
        unimplemented!()
    }

    fn channel_counter(&self) -> Result<u64, Error> {
        unimplemented!()
    }

    fn max_expected_time_per_block(&self) -> Duration {
        unimplemented!()
    }

    fn lookup_module_by_channel(
        &self,
        _channel_id: &ChannelId,
        _port_id: &PortId,
    ) -> Result<(ModuleId, ChannelCapability), Error> {
        unimplemented!()
    }
}

impl Ics20Context for DummyTransferModule {
    type AccountId = Signer;
}
