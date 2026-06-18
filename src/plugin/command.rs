#[cfg(test)]
mod tests;

use std::{cell::RefCell, os::raw::c_int, slice};

use classicube_helpers::{
    chat,
    entities::{ENTITY_SELF_ID, Entity},
    tab_list::remove_color,
};
use classicube_sys::{ENTITIES_MAX_COUNT, OwnedChatCommand, cc_string};
use tracing::debug;

use crate::plugin::{custom_models, is_plugin_active, module::Module, pet};

thread_local!(
    // Pinned for the whole process. `OwnedChatCommand`'s Drop frees memory
    // still referenced by ClassiCube's `cmds_head` list, which has no
    // unregister API -- register once, never clear this slot.
    static COMMAND: RefCell<Option<OwnedChatCommand>> = const { RefCell::new(None) };
);

/// True if `candidate` contains `query`, with color codes stripped and
/// case-folded on both sides.
fn name_matches(candidate: &str, query: &str) -> bool {
    remove_color(candidate)
        .to_lowercase()
        .contains(&remove_color(query).to_lowercase())
}

/// Return the id of the first live entity on the current map whose display name
/// matches `query` (case-insensitive substring, color codes stripped).
///
/// This function touches ClassiCube FFI (`Entities.List`, `Entity::from_id`).
/// Keep it out of test-reachable code so `--gc-sections` drops the FFI
/// references and the test binary links cleanly.
fn find_entity_by_name(query: &str) -> Option<u8> {
    for id in 0..ENTITIES_MAX_COUNT {
        // ENTITIES_MAX_COUNT == 256; ids fit in u8 (0..=255).
        #[expect(
            clippy::cast_possible_truncation,
            reason = "ENTITIES_MAX_COUNT == 256; range is 0..255 which fits u8"
        )]
        let id = id as u8;
        // SAFETY: Entity::from_id null-checks Entities.List[id]; the returned
        // reference is valid for this loop iteration only.
        let Some(entity) = (unsafe { Entity::from_id(id) }) else {
            continue;
        };
        if name_matches(&entity.get_display_name(), query) {
            return Some(id);
        }
    }
    None
}

/// `/client pet ...` (or `/pet ...`). Bails when the plugin is inactive so a
/// command invoked during the Free -> next-Init gap touches no torn-down state.
unsafe extern "C" fn execute(args: *const cc_string, args_count: c_int) {
    if !is_plugin_active() {
        chat::print("&ePet: plugin not active (between hot-reload Free/Init); ignoring command");
        return;
    }
    // SAFETY: ClassiCube passes a valid pointer to its fixed-size args array
    // (length == args_count); each entry is a live `cc_string` for this call.
    let args: Vec<String> = unsafe { slice::from_raw_parts(args, args_count as usize) }
        .iter()
        .map(|s| s.to_string())
        .collect();
    debug!(?args, "pet command");

    match args.first().map(String::as_str) {
        Some("here") => {
            if pet::bring_pet_to_player() {
                chat::print("&e[Pet] Coming!");
            } else {
                chat::print("&c[Pet] No pet to bring (are you in a world?)");
            }
        }
        Some("copy") => {
            // Join remaining args as the entity name query.
            let query = args[1..].join(" ");
            let query = query.trim();

            let entity_id = if query.is_empty() {
                // No name: copy the local player (self).
                ENTITY_SELF_ID
            } else {
                match find_entity_by_name(query) {
                    Some(id) => id,
                    None => {
                        chat::print(format!(
                            "&c[Pet] No entity named '{query}' found on this map"
                        ));
                        return;
                    }
                }
            };

            match custom_models::copy_entity_model_to_pet(entity_id) {
                Ok(name) => chat::print(format!("&e[Pet] Copied model '{name}' to your pet")),
                Err(msg) => chat::print(msg),
            }
        }
        Some("go") => {
            if pet::walk_pet_to_player() {
                chat::print("&e[Pet] On my way!");
            } else {
                chat::print("&c[Pet] No pet to move (are you in a world?)");
            }
        }
        _ => {
            chat::print("&aUsage: &f/client pet here &e-- bring your pet to you");
            chat::print(
                "&aUsage: &f/client pet copy [name] &e-- copy an entity's model to your pet (no \
                 name = you)",
            );
            chat::print(
                "&aUsage: &f/client pet go &e-- walk your pet to you (teleports if no path or too \
                 far)",
            );
        }
    }
}

pub struct CommandModule;

impl CommandModule {
    pub fn init() -> Self {
        COMMAND.with_borrow_mut(|slot| {
            if slot.is_none() {
                let mut command = OwnedChatCommand::new(
                    "Pet",
                    execute,
                    false, // not singleplayer-only
                    vec![
                        "&aUsage: &f/client pet here",
                        "&eBring your pet to your position.",
                        "&aUsage: &f/client pet copy [name]",
                        "&eCopy an entity's model to your pet. No name copies your own model.",
                        "&aUsage: &f/client pet go",
                        "&eWalk your pet to your position. Teleports if no path or too far away.",
                    ],
                );
                command.register();
                *slot = Some(command);
            }
        });
        Self
    }
}

impl Module for CommandModule {}
