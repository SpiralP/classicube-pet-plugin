mod entity;

use classicube_helpers::local_player_vtable_hook::{LocalPlayerVTableHook, LocalPlayerVTableHooks};
use classicube_sys::Vec3;

use self::entity::PetEntity;
use crate::plugin::module::Module;

/// Hardcoded world-space offset from the local player (~1 block +X).
pub const OFFSET: Vec3 = Vec3::new(1.0, 0.0, 0.0);

pub struct PetModule {
    /// RAII handle for our chain-safe `RenderModel` hook on the local player.
    /// `None` until the local player entity exists. Dropping it on `free`
    /// restores the prior VTABLE when we are the chain head, or clears our
    /// callback so a buried trampoline forwards transparently. The helper owns
    /// all install/uninstall/reload-safety logic.
    hook: Option<LocalPlayerVTableHook>,
}

impl PetModule {
    pub fn init() -> Self {
        let mut pet = PetEntity::new();
        let hooks = LocalPlayerVTableHooks {
            render_model: Some(Box::new(move |local_player, delta, t, original| {
                // Forward to the original first so the local player (and
                // anything chained below us) renders, then draw the pet.
                // SAFETY: `original` is the next-in-chain RenderModel fn the
                // helper saved at push time; `local_player` is the live,
                // non-null entity the engine passed into this dispatch.
                unsafe { original(local_player, delta, t) };
                // SAFETY: same live local player pointer, valid for this call.
                unsafe { pet.update_and_render(local_player) };
            })),
            ..Default::default()
        };
        Self {
            hook: Some(LocalPlayerVTableHook::install(hooks)),
        }
    }
}

impl Module for PetModule {
    fn free(&mut self) {
        self.hook = None;
    }
}
