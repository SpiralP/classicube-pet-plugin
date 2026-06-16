use std::{cell::RefCell, os::raw::c_int, slice};

use classicube_helpers::chat;
use classicube_sys::{OwnedChatCommand, cc_string};
use tracing::debug;

use crate::plugin::{is_plugin_active, module::Module};

thread_local!(
    // Pinned for the whole process. `OwnedChatCommand`'s Drop frees memory
    // still referenced by ClassiCube's `cmds_head` list, which has no
    // unregister API -- register once, never clear this slot.
    static COMMAND: RefCell<Option<OwnedChatCommand>> = const { RefCell::new(None) };
);

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

    // TODO: dispatch subcommands here as the plugin grows.
    chat::print(format!("&e[Pet] {}", args.join(" ")));
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
                    vec!["&aUsage: &f/client pet", "&ePlaceholder pet command."],
                );
                command.register();
                *slot = Some(command);
            }
        });
        Self
    }
}

impl Module for CommandModule {}
