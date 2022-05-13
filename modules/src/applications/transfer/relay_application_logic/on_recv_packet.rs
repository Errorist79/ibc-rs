use crate::applications::transfer::context::Ics20Context;
use crate::applications::transfer::error::Error as Ics20Error;
use crate::applications::transfer::events::DenomTraceEvent;
use crate::applications::transfer::packet::PacketData;
use crate::applications::transfer::{IbcCoin, Source, TracePrefix};
use crate::core::ics04_channel::packet::Packet;
use crate::core::ics26_routing::context::{ModuleOutputBuilder, WriteFn};
use crate::prelude::*;

pub fn process_recv_packet<Ctx: 'static + Ics20Context>(
    ctx: &Ctx,
    output: &mut ModuleOutputBuilder,
    packet: &Packet,
    data: PacketData,
) -> Result<Box<WriteFn>, Ics20Error> {
    if !ctx.is_receive_enabled() {
        return Err(Ics20Error::receive_disabled());
    }

    let receiver = data
        .receiver
        .clone()
        .try_into()
        .map_err(|_| Ics20Error::parse_account_failure())?;

    let prefix = TracePrefix::new(packet.source_port.clone(), packet.source_channel);
    match data.token.denom.source_chain(&prefix) {
        Source::Receiver => {
            // sender chain is not the source, unescrow tokens
            let coin = {
                let mut c = data.token.clone();
                c.denom.remove_prefix(&prefix);
                c
            };

            if ctx.is_blocked_account(&receiver) {
                return Err(Ics20Error::unauthorised_receive(data.receiver));
            }

            let escrow_address = ctx
                .get_channel_escrow_address(&packet.destination_port, packet.destination_channel)?;
            let amount = IbcCoin::from(coin);

            Ok(Box::new(move |ctx| {
                let ctx = ctx.downcast_mut::<Ctx>().unwrap();
                ctx.send_coins(&escrow_address, &receiver, &amount)
                    .map_err(|e| e.to_string())
            }))
        }
        Source::Sender => {
            // sender chain is the source, mint vouchers
            let prefix =
                TracePrefix::new(packet.destination_port.clone(), packet.destination_channel);
            let coin = {
                let mut c = data.token;
                c.denom.add_prefix(prefix);
                c
            };

            let denom_trace_event = DenomTraceEvent {
                trace_hash: coin.denom.hashed(),
                denom: coin.denom.clone(),
            };
            output.emit(denom_trace_event.into());

            Ok(Box::new(move |ctx| {
                let ctx = ctx.downcast_mut::<Ctx>().unwrap();
                let hashed_denom = coin.denom.hashed();
                if ctx.has_denom_trace(&hashed_denom) {
                    ctx.set_denom_trace(&coin.denom)
                        .map_err(|e| e.to_string())?;
                }

                let amount = IbcCoin::from(coin);
                ctx.mint_coins(&ctx.get_transfer_account(), &amount)
                    .map_err(|e| e.to_string())?;
                ctx.send_coins_from_module_to_account(
                    &ctx.get_transfer_account(),
                    &receiver,
                    &amount,
                )
                .map_err(|e| e.to_string())
            }))
        }
    }
}
