mod entity;

use std::{
    cell::RefCell,
    rc::{Rc, Weak},
};

use classicube_helpers::{
    entities::{ENTITY_SELF_ID, Entity},
    local_player_vtable_hook::{LocalPlayerVTableHook, LocalPlayerVTableHooks},
};

use self::entity::PetEntity;
use crate::plugin::module::Module;

thread_local!(
    // Weak handle to the live pet so the chat command can reach it. The
    // PetModule field below holds the sole strong Rc; this self-heals to a
    // dead Weak on free, making a command during the reload gap a no-op.
    static PET: RefCell<Weak<RefCell<PetEntity>>> = const { RefCell::new(Weak::new()) };
);

pub struct PetModule {
    /// RAII handle for our chain-safe `RenderModel` hook on the local player.
    /// `None` until the local player entity exists. Dropping it on `free`
    /// restores the prior VTABLE when we are the chain head, or clears our
    /// callback so a buried trampoline forwards transparently. The helper owns
    /// all install/uninstall/reload-safety logic.
    hook: Option<LocalPlayerVTableHook>,
    // Sole strong owner of the pet; render closure + command hold only Weak.
    #[expect(
        dead_code,
        reason = "kept alive to preserve the Weak handles in PET and the render closure"
    )]
    pet: Rc<RefCell<PetEntity>>,
}

impl PetModule {
    pub fn init() -> Self {
        let pet = Rc::new(RefCell::new(PetEntity::new()));
        PET.with_borrow_mut(|slot| *slot = Rc::downgrade(&pet));
        let weak = Rc::downgrade(&pet);
        let hooks = LocalPlayerVTableHooks {
            render_model: Some(Box::new(move |local_player, delta, t, original| {
                // Forward to the original first so the local player (and
                // anything chained below us) renders, then draw the pet.
                // SAFETY: `original` is the next-in-chain RenderModel fn the
                // helper saved at push time; `local_player` is the live,
                // non-null entity the engine passed into this dispatch.
                unsafe { original(local_player, delta, t) };
                if let Some(pet) = weak.upgrade() {
                    pet.borrow_mut().update_and_render();
                }
            })),
            ..Default::default()
        };
        Self {
            hook: Some(LocalPlayerVTableHook::install(hooks)),
            pet,
        }
    }
}

impl Module for PetModule {
    fn free(&mut self) {
        self.hook = None;
    }
}

/// Teleport the pet to the local player's current position and view angles.
/// Returns `false` if the pet is gone (between Free and the next Init) or
/// there is no local player entity yet (e.g. at the main menu).
pub fn bring_pet_to_player() -> bool {
    let Some(pet) = PET.with_borrow(Weak::upgrade) else {
        return false;
    };
    // SAFETY: ENTITY_SELF_ID always exists in-world; the borrow is transient
    // and the wrapped in-game entity outlives this call.
    let Some(local_player) = (unsafe { Entity::from_id(ENTITY_SELF_ID) }) else {
        return false;
    };
    let position = local_player.get_position();
    let [pitch, yaw] = local_player.get_head();
    let rot = local_player.get_rot();
    pet.borrow_mut().set_transform(position, pitch, yaw, rot);
    true
}
