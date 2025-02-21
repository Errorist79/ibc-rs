use core::fmt;
use core::str::FromStr;

use derive_more::{Display, From, Into};
use ibc_proto::cosmos::base::v1beta1::Coin as RawCoin;
use ibc_proto::ibc::applications::transfer::v1::DenomTrace as RawDenomTrace;
use serde::{Deserialize, Serialize};

use super::error::Error;
use crate::bigint::U256;
use crate::core::ics24_host::identifier::{ChannelId, PortId};
use crate::prelude::*;
use crate::serializers::serde_string;

/// A `Coin` type with fully qualified `PrefixedDenom`.
pub type PrefixedCoin = Coin<PrefixedDenom>;

/// A `Coin` type with an unprefixed denomination.
pub type BaseCoin = Coin<BaseDenom>;

/// Base denomination type
#[derive(Clone, Debug, Eq, PartialEq, PartialOrd, Ord, Serialize, Deserialize, Display)]
#[serde(transparent)]
pub struct BaseDenom(String);

impl FromStr for BaseDenom {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.trim().is_empty() {
            Err(Error::empty_base_denom())
        } else {
            Ok(BaseDenom(s.to_owned()))
        }
    }
}

#[derive(Clone, Debug, Ord, PartialOrd, Eq, PartialEq)]
pub struct TracePrefix {
    port_id: PortId,
    channel_id: ChannelId,
}

impl TracePrefix {
    pub fn new(port_id: PortId, channel_id: ChannelId) -> Self {
        Self {
            port_id,
            channel_id,
        }
    }
}

impl fmt::Display for TracePrefix {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.port_id, self.channel_id)
    }
}

/// A full trace path modelled as a collection of `TracePrefix`s.
// Internally, the `TracePath` is modelled as a `Vec<TracePrefix>` but with the order reversed, i.e.
// "transfer/channel-0/transfer/channel-1/uatom" => `["transfer/channel-1", "transfer/channel-0"]`
// This is done for ease of addition/removal of prefixes.
#[derive(Clone, Debug, Default, Eq, PartialEq, PartialOrd, Ord, From)]
pub struct TracePath(Vec<TracePrefix>);

impl TracePath {
    /// Returns true iff this path starts with the specified prefix
    pub fn starts_with(&self, prefix: &TracePrefix) -> bool {
        self.0.last().map(|p| p == prefix).unwrap_or(false)
    }

    /// Removes the specified prefix from the path if there is a match, otherwise does nothing.
    pub fn remove_prefix(&mut self, prefix: &TracePrefix) {
        if self.starts_with(prefix) {
            self.0.pop();
        }
    }

    /// Adds the specified prefix to the path.
    pub fn add_prefix(&mut self, prefix: TracePrefix) {
        self.0.push(prefix)
    }

    /// Returns true if the path is empty and false otherwise.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl<'a> TryFrom<Vec<&'a str>> for TracePath {
    type Error = Error;

    fn try_from(v: Vec<&'a str>) -> Result<Self, Self::Error> {
        if v.len() % 2 != 0 {
            return Err(Error::invalid_trace_length(v.len()));
        }

        let mut trace = vec![];
        let id_pairs = v.chunks_exact(2).map(|paths| (paths[0], paths[1]));
        for (pos, (port_id, channel_id)) in id_pairs.rev().enumerate() {
            let port_id =
                PortId::from_str(port_id).map_err(|e| Error::invalid_trace_port_id(pos, e))?;
            let channel_id = ChannelId::from_str(channel_id)
                .map_err(|e| Error::invalid_trace_channel_id(pos, e))?;
            trace.push(TracePrefix {
                port_id,
                channel_id,
            });
        }

        Ok(trace.into())
    }
}

impl FromStr for TracePath {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let parts = {
            let parts: Vec<&str> = s.split('/').collect();
            if parts.len() == 1 && parts[0].trim().is_empty() {
                vec![]
            } else {
                parts
            }
        };
        parts.try_into()
    }
}

impl fmt::Display for TracePath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let path = self
            .0
            .iter()
            .rev()
            .map(|prefix| prefix.to_string())
            .collect::<Vec<String>>()
            .join("/");
        write!(f, "{}", path)
    }
}

/// A type that contains the base denomination for ICS20 and the source tracing information path.
#[derive(Clone, Debug, Eq, PartialEq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct PrefixedDenom {
    /// A series of `{port-id}/{channel-id}`s for tracing the source of the token.
    #[serde(with = "serde_string")]
    trace_path: TracePath,
    /// Base denomination of the relayed fungible token.
    base_denom: BaseDenom,
}

impl PrefixedDenom {
    /// Removes the specified prefix from the trace path if there is a match, otherwise does nothing.
    pub fn remove_trace_prefix(&mut self, prefix: &TracePrefix) {
        self.trace_path.remove_prefix(prefix)
    }

    /// Adds the specified prefix to the trace path.
    pub fn add_trace_prefix(&mut self, prefix: TracePrefix) {
        self.trace_path.add_prefix(prefix)
    }
}

/// Returns true if the denomination originally came from the sender chain and
/// false otherwise.
///
/// Note: It is better to think of the "source" chain as the chain that
/// escrows/unescrows the token, while the other chain mints/burns the tokens,
/// respectively. A chain being the "source" of a token does NOT mean it is the
/// original creator of the token (e.g. "uatom"), as "source" might suggest.
///
/// This means that in any given transfer, a chain can very well be the source
/// of a token of which it is not the creator. For example, let
///
/// A: sender chain in this transfer, port "transfer" and channel "c2b" (to B)
/// B: receiver chain in this transfer, port "transfer" and channel "c2a" (to A)
/// token denom: "transfer/someOtherChannel/someDenom"
///
/// A, initiator of the transfer, needs to figure out if it should escrow the
/// tokens, or burn them. If B had originally sent the token to A in a previous
/// transfer, then A would have stored the token as "transfer/c2b/someDenom".
/// Now, A is sending to B, so to check if B is the source of the token, we need
/// to check if the token starts with "transfer/c2b". In this example, it
/// doesn't, so the token doesn't originate from B. A is considered the source,
/// even though it is not the creator of the token. Specifically, the token was
/// created by the chain at the other end of A's port "transfer" and channel
/// "someOtherChannel".
pub fn is_sender_chain_source(
    source_port: PortId,
    source_channel: ChannelId,
    denom: &PrefixedDenom,
) -> bool {
    !is_receiver_chain_source(source_port, source_channel, denom)
}

/// Returns true if the denomination originally came from the receiving chain and false otherwise.
pub fn is_receiver_chain_source(
    source_port: PortId,
    source_channel: ChannelId,
    denom: &PrefixedDenom,
) -> bool {
    // For example, let
    // A: sender chain in this transfer, port "transfer" and channel "c2b" (to B)
    // B: receiver chain in this transfer, port "transfer" and channel "c2a" (to A)
    //
    // If B had originally sent the token in a previous tranfer, then A would have stored the token as
    // "transfer/c2b/{token_denom}". Now, A is sending to B, so to check if B is the source of the token,
    // we need to check if the token starts with "transfer/c2b".
    let prefix = TracePrefix::new(source_port, source_channel);
    denom.trace_path.starts_with(&prefix)
}

impl FromStr for PrefixedDenom {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut parts: Vec<&str> = s.split('/').collect();
        let last_part = parts.pop().expect("split() returned an empty iterator");

        let (base_denom, trace_path) = {
            if last_part == s {
                (BaseDenom::from_str(s)?, TracePath::default())
            } else {
                let base_denom = BaseDenom::from_str(last_part)?;
                let trace_path = TracePath::try_from(parts)?;
                (base_denom, trace_path)
            }
        };

        Ok(Self {
            trace_path,
            base_denom,
        })
    }
}

impl TryFrom<RawDenomTrace> for PrefixedDenom {
    type Error = Error;

    fn try_from(value: RawDenomTrace) -> Result<Self, Self::Error> {
        let base_denom = BaseDenom::from_str(&value.base_denom)?;
        let trace_path = TracePath::from_str(&value.path)?;
        Ok(Self {
            trace_path,
            base_denom,
        })
    }
}

impl From<PrefixedDenom> for RawDenomTrace {
    fn from(value: PrefixedDenom) -> Self {
        Self {
            path: value.trace_path.to_string(),
            base_denom: value.base_denom.to_string(),
        }
    }
}

impl From<BaseDenom> for PrefixedDenom {
    fn from(denom: BaseDenom) -> Self {
        Self {
            trace_path: Default::default(),
            base_denom: denom,
        }
    }
}

impl fmt::Display for PrefixedDenom {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.trace_path.0.is_empty() {
            write!(f, "{}", self.base_denom)
        } else {
            write!(f, "{}/{}", self.trace_path, self.base_denom)
        }
    }
}

/// A type for representing token transfer amounts.
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord, Display, From, Into)]
pub struct Amount(U256);

impl Amount {
    pub fn checked_add(self, rhs: Self) -> Option<Self> {
        self.0.checked_add(rhs.0).map(Self)
    }

    pub fn checked_sub(self, rhs: Self) -> Option<Self> {
        self.0.checked_sub(rhs.0).map(Self)
    }
}

impl FromStr for Amount {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let amount = U256::from_str_radix(s, 10).map_err(Error::invalid_amount)?;
        Ok(Self(amount))
    }
}

impl From<u64> for Amount {
    fn from(v: u64) -> Self {
        Self(v.into())
    }
}

/// Coin defines a token with a denomination and an amount.
#[derive(Clone, Debug, Eq, PartialEq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Coin<D> {
    /// Denomination
    pub denom: D,
    /// Amount
    #[serde(with = "serde_string")]
    pub amount: Amount,
}

impl<D: FromStr> TryFrom<RawCoin> for Coin<D>
where
    Error: From<<D as FromStr>::Err>,
{
    type Error = Error;

    fn try_from(proto: RawCoin) -> Result<Coin<D>, Self::Error> {
        let denom = D::from_str(&proto.denom)?;
        let amount = Amount::from_str(&proto.amount)?;
        Ok(Self { denom, amount })
    }
}

impl<D: ToString> From<Coin<D>> for RawCoin {
    fn from(coin: Coin<D>) -> RawCoin {
        RawCoin {
            denom: coin.denom.to_string(),
            amount: coin.amount.to_string(),
        }
    }
}

impl From<BaseCoin> for PrefixedCoin {
    fn from(coin: BaseCoin) -> PrefixedCoin {
        PrefixedCoin {
            denom: coin.denom.into(),
            amount: coin.amount,
        }
    }
}

impl fmt::Display for PrefixedCoin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}-{}", self.amount, self.denom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_denom_validation() -> Result<(), Error> {
        assert!(BaseDenom::from_str("").is_err(), "empty base denom");
        assert!(BaseDenom::from_str("uatom").is_ok(), "valid base denom");
        assert!(PrefixedDenom::from_str("").is_err(), "empty denom trace");
        assert!(
            PrefixedDenom::from_str("transfer/channel-0/").is_err(),
            "empty base denom with trace"
        );
        assert!(PrefixedDenom::from_str("/uatom").is_err(), "empty prefix");
        assert!(PrefixedDenom::from_str("//uatom").is_err(), "empty ids");
        assert!(
            PrefixedDenom::from_str("transfer/").is_err(),
            "single trace"
        );
        assert!(
            PrefixedDenom::from_str("transfer/atom").is_err(),
            "single trace with base denom"
        );
        assert!(
            PrefixedDenom::from_str("transfer/channel-0/uatom").is_ok(),
            "valid single trace info"
        );
        assert!(
            PrefixedDenom::from_str("transfer/channel-0/transfer/channel-1/uatom").is_ok(),
            "valid multiple trace info"
        );
        assert!(
            PrefixedDenom::from_str("(transfer)/channel-0/uatom").is_err(),
            "invalid port"
        );
        assert!(
            PrefixedDenom::from_str("transfer/(channel-0)/uatom").is_err(),
            "invalid channel"
        );

        Ok(())
    }

    #[test]
    fn test_denom_trace() -> Result<(), Error> {
        assert_eq!(
            PrefixedDenom::from_str("transfer/channel-0/uatom")?,
            PrefixedDenom {
                trace_path: "transfer/channel-0".parse()?,
                base_denom: "uatom".parse()?
            },
            "valid single trace info"
        );
        assert_eq!(
            PrefixedDenom::from_str("transfer/channel-0/transfer/channel-1/uatom")?,
            PrefixedDenom {
                trace_path: "transfer/channel-0/transfer/channel-1".parse()?,
                base_denom: "uatom".parse()?
            },
            "valid multiple trace info"
        );

        Ok(())
    }

    #[test]
    fn test_denom_serde() -> Result<(), Error> {
        let dt_str = "transfer/channel-0/uatom";
        let dt = PrefixedDenom::from_str(dt_str)?;
        assert_eq!(dt.to_string(), dt_str, "valid single trace info");

        let dt_str = "transfer/channel-0/transfer/channel-1/uatom";
        let dt = PrefixedDenom::from_str(dt_str)?;
        assert_eq!(dt.to_string(), dt_str, "valid multiple trace info");

        Ok(())
    }

    #[test]
    fn test_trace_path() -> Result<(), Error> {
        assert!(TracePath::from_str("").is_ok(), "empty trace path");
        assert!(
            TracePath::from_str("transfer/uatom").is_err(),
            "invalid trace path: bad ChannelId"
        );
        assert!(
            TracePath::from_str("transfer//uatom").is_err(),
            "malformed trace path: missing ChannelId"
        );
        assert!(
            TracePath::from_str("transfer/channel-0/").is_err(),
            "malformed trace path: trailing delimiter"
        );

        let prefix_1 = TracePrefix::new("transfer".parse().unwrap(), "channel-1".parse().unwrap());
        let prefix_2 = TracePrefix::new("transfer".parse().unwrap(), "channel-0".parse().unwrap());
        let mut trace_path = TracePath(vec![prefix_1.clone()]);

        trace_path.add_prefix(prefix_2.clone());
        assert_eq!(
            TracePath::from_str("transfer/channel-0/transfer/channel-1")?,
            trace_path
        );
        assert_eq!(
            TracePath(vec![prefix_1.clone(), prefix_2.clone()]),
            trace_path
        );

        trace_path.remove_prefix(&prefix_2);
        assert_eq!(TracePath::from_str("transfer/channel-1")?, trace_path);
        assert_eq!(TracePath(vec![prefix_1.clone()]), trace_path);

        trace_path.remove_prefix(&prefix_1);
        assert!(trace_path.is_empty());

        Ok(())
    }
}
