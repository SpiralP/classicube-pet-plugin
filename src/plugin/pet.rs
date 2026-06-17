mod entity;
mod skin;

use std::{
    cell::RefCell,
    ffi::{CStr, c_char},
    rc::{Rc, Weak},
};

use classicube_helpers::{
    entities::{ENTITY_SELF_ID, Entity},
    events::gfx::{ContextLostEventHandler, ContextRecreatedEventHandler},
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
    // RAII graphics context event handlers for pet skin texture lifecycle.
    _ctx_lost: ContextLostEventHandler,
    _ctx_recreated: ContextRecreatedEventHandler,
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
        let (ctx_lost, ctx_recreated) = skin::install_context_handlers();
        Self {
            hook: Some(LocalPlayerVTableHook::install(hooks)),
            pet,
            _ctx_lost: ctx_lost,
            _ctx_recreated: ctx_recreated,
        }
    }
}

impl Module for PetModule {
    fn reset(&mut self) {
        // Game_Reset (disconnect / local map load) zeroes every custom-model
        // slot via CustomModel_FreeAll. Revert to the built-in default first so
        // the pet's entity.Model never dangles; built-ins are never freed, so
        // this is safe regardless of whether our reset runs before or after the
        // engine's. Clear the owned skin texture too.
        reset_pet_to_default_model();
    }

    fn free(&mut self) {
        self.hook = None;
        // Release the owned GPU texture and unsubscribe context handlers (done
        // automatically when _ctx_lost/_ctx_recreated drop after this fn).
        skin::clear();
    }
}

/// Revert the pet to its built-in default model and drop the owned skin
/// texture. Safe to call without a live world. Returns `false` (no-op) if the
/// pet is gone (between Free and the next Init).
pub fn reset_pet_to_default_model() -> bool {
    let Some(pet) = PET.with_borrow(Weak::upgrade) else {
        return false;
    };
    pet.borrow_mut().reset_to_default_model();
    skin::clear();
    true
}

/// Apply `model_name` to the pet and kick off an async download of the local
/// player's skin to build an owned GPU texture. Returns `false` if the pet or
/// local player is not available.
pub fn set_pet_model(model_name: &str) -> bool {
    let Some(pet) = PET.with_borrow(Weak::upgrade) else {
        return false;
    };
    pet.borrow_mut().set_model(model_name);

    // DeleteSkin-equivalent: drop the old owned texture + reset the pet's skin
    // fields so the new model shows its default texture until the new skin
    // downloads (mirrors Entity_SetSkin's leading DeleteSkin call). Must run
    // with no outstanding pet borrow -- skin::clear borrows the same PET cell.
    skin::clear();

    // Mirror the player's skin fields onto the pet (the writes Entity_SetSkin
    // performs: SkinRaw + NonHumanSkin) and capture the skin name for the async
    // re-fetch. Mirroring NonHumanSkin makes the pet honour its own TextureId
    // exactly when the player's model does (Model_ApplyTexture uses
    // `model.usesHumanSkin || e.NonHumanSkin`); otherwise a non-human/custom
    // model would ignore the pet's downloaded skin.
    // SAFETY: ENTITY_SELF_ID always exists in-world; the borrow is transient.
    let skin_name = unsafe {
        let Some(local_player) = Entity::from_id(ENTITY_SELF_ID) else {
            return false;
        };
        let inner = local_player.get_inner();
        pet.borrow_mut().copy_skin_from(inner);
        // SkinRaw is a NUL-terminated char[64]; treat bytes as a C string.
        let ptr = inner.SkinRaw.as_ptr() as *const c_char;
        CStr::from_ptr(ptr).to_string_lossy().into_owned()
        // local_player (and the inner borrow) dropped here
    };

    skin::request_player_skin(skin_name);
    true
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
