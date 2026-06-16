#[cfg(test)]
mod tests;

use std::{cell::RefCell, collections::HashSet, rc::Rc};

use anyhow::{Result, ensure};
use borsh::{BorshDeserialize, BorshSerialize};
use classicube_helpers::{chat, entities::ENTITY_SELF_ID, tab_list::TabListEntry};
use classicube_relay::{RelayListener, Stream, packet::MapScope};
use classicube_sys::Server;
use tracing::{debug, error, warn};

use crate::plugin::{module::Module, pet};

const RELAY_CHANNEL: u8 = 211;

#[derive(Debug, BorshSerialize, BorshDeserialize)]
enum RelayMessage {
    Hello {
        version: String,
    },
    PetState {
        model: String,
        model_scale: (f32, f32, f32),
        offset: (f32, f32, f32),
    },
}

fn entity_nick_name(id: u8) -> String {
    unsafe { TabListEntry::from_id(id) }
        .map(|e| e.get_nick_name())
        .unwrap_or_else(|| format!("Entity#{}", id))
}

fn entity_real_name(id: u8) -> Option<String> {
    unsafe { TabListEntry::from_id(id) }.map(|e| e.get_real_name())
}

fn is_singleplayer() -> bool {
    (unsafe { Server.IsSinglePlayer }) != 0
}

fn parse_version(s: &str) -> Option<(u32, u32, u32)> {
    let mut parts = s.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    let patch = parts.next()?.parse().ok()?;
    Some((major, minor, patch))
}

fn send(msg: &RelayMessage) -> Result<()> {
    if is_singleplayer() {
        return Ok(());
    }
    debug!(?msg, "relay send");
    let data = borsh::to_vec(msg)?;
    let stream = Stream::new(data, MapScope { have_plugin: true })?;
    for packet in stream.packets()? {
        let mut data = packet.encode()?;
        unsafe {
            classicube_sys::CPE_SendPluginMessage(RELAY_CHANNEL, data.as_mut_ptr());
        }
    }
    Ok(())
}

fn handle_receive(
    player_id: u8,
    data: &[u8],
    seen_plugin: &RefCell<HashSet<String>>,
) -> Result<()> {
    ensure!(player_id != ENTITY_SELF_ID, "got ENTITY_SELF_ID");

    let msg = borsh::from_slice::<RelayMessage>(data)?;
    debug!(?player_id, ?msg, "relay recv");

    let sender_real = match entity_real_name(player_id) {
        Some(n) => n,
        None => {
            warn!(
                "relay: could not resolve sender entity {} to name",
                player_id
            );
            return Ok(());
        }
    };

    let is_new = seen_plugin.borrow_mut().insert(sender_real.clone());
    if is_new {
        let sender_nick = entity_nick_name(player_id);
        chat::print(format!("&e[Pet] {sender_nick}&e has the pet plugin"));
    }

    match msg {
        RelayMessage::Hello { version } => {
            if is_new {
                let ours = env!("CARGO_PKG_VERSION");
                if let (Some(theirs), Some(mine)) = (parse_version(&version), parse_version(ours))
                    && theirs > mine
                {
                    chat::print(format!(
                        "&e[Pet] A newer version is available (v{version}, you have v{ours})"
                    ));
                }
            }
            // Re-broadcast our state so the newcomer knows about us.
            broadcast_pet_state();
        }

        RelayMessage::PetState {
            model,
            model_scale,
            offset,
        } => {
            // TODO: render other players' pets (pet-multiplayer todo).
            debug!(
                ?player_id,
                ?model,
                ?model_scale,
                ?offset,
                "remote pet state received"
            );
        }
    }

    Ok(())
}

pub fn send_pet_state(model: &str, model_scale: (f32, f32, f32), offset: (f32, f32, f32)) {
    if let Err(e) = send(&RelayMessage::PetState {
        model: model.to_string(),
        model_scale,
        offset,
    }) {
        error!("relay send_pet_state: {:#}", e);
    }
}

fn broadcast_pet_state() {
    let offset = pet::OFFSET;
    // TODO: wire to live pet model/scale once the pet carries a stable model name.
    // The pet currently mirrors the local player's Model pointer each frame,
    // so there is no stored model string to read here yet.
    send_pet_state("humanoid", (1.0, 1.0, 1.0), (offset.x, offset.y, offset.z));
}

pub struct RelayModule {
    listener: Option<RelayListener>,
    // Sole owner; the listener closure holds only a Weak so it bails if we are freed first.
    seen_plugin: Rc<RefCell<HashSet<String>>>,
}

impl RelayModule {
    pub fn init() -> Self {
        let seen_plugin: Rc<RefCell<HashSet<String>>> = Default::default();

        let listener = if !is_singleplayer() {
            let mut listener = RelayListener::new(RELAY_CHANNEL).unwrap();
            let seen = Rc::downgrade(&seen_plugin);
            listener.on(move |player_id, data| {
                if let Some(seen) = seen.upgrade()
                    && let Err(e) = handle_receive(player_id, data, &seen)
                {
                    error!("relay handle_receive: {:#}", e);
                }
            });
            Some(listener)
        } else {
            None
        };

        Self {
            listener,
            seen_plugin,
        }
    }

    #[expect(
        dead_code,
        reason = "public API for future callers checking peer plugin presence"
    )]
    pub fn has_plugin(&self, real_name: &str) -> bool {
        self.seen_plugin.borrow().contains(real_name)
    }
}

impl Module for RelayModule {
    fn reset(&mut self) {
        self.seen_plugin.borrow_mut().clear();
    }

    fn on_new_map_loaded(&mut self) {
        if let Err(e) = send(&RelayMessage::Hello {
            version: env!("CARGO_PKG_VERSION").to_string(),
        }) {
            error!("relay hello: {:#}", e);
        }
        broadcast_pet_state();
    }

    fn free(&mut self) {
        self.listener = None;
        self.seen_plugin.borrow_mut().clear();
    }
}
