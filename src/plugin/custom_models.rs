#[cfg(test)]
mod tests;

use std::{
    cell::RefCell,
    collections::HashMap,
    ffi::CStr,
    rc::{Rc, Weak},
};

use classicube_helpers::{entities::Entity, protocol_hook::ProtocolHook};
use classicube_sys::{
    Model_Get, OPCODE__OPCODE_DEFINE_MODEL, OPCODE__OPCODE_DEFINE_MODEL_PART,
    OPCODE__OPCODE_UNDEFINE_MODEL, OwnedString, Protocol,
};
use tracing::{debug, info};
use zerocopy::{FromBytes, Immutable, IntoBytes, KnownLayout};

use crate::plugin::{module::Module, pet};

thread_local!(
    // Weak handle so the command callback can reach module state without
    // borrowing MAIN_MODULE. Self-heals to a dead Weak between Free and Init.
    static CUSTOM_MODELS_STATE: RefCell<Weak<RefCell<State>>> = const { RefCell::new(Weak::new()) };
);

// -- Wire format structs ---------------------------------------------------

/// DefineModel packet payload (115 bytes).
#[repr(C)]
#[derive(Clone, Copy, FromBytes, IntoBytes, Immutable, KnownLayout)]
struct DefineModelPayload {
    id: u8,
    name: [u8; 64],
    /// flags(1) + nameY(4) + eyeY(4) + collision(12) + pickingMin(12) + pickingMax(12) + uScale(2) + vScale(2)
    _rest: [u8; 49],
    num_parts: u8,
}

/// DefineModelPart v2 packet payload (166 bytes).
#[repr(C)]
#[derive(Clone, Copy, FromBytes, IntoBytes, Immutable, KnownLayout)]
struct DefineModelPartPayload {
    id: u8,
    /// min(12) + max(12) + UVs(48) + rotOrigin(12) + rotation(12) + anims(68) + flags(1)
    _rest: [u8; 165],
}

struct CapturedModel {
    define: Vec<u8>,
    parts: Vec<Vec<u8>>,
}

struct InProgress {
    name: String,
    num_parts: u8,
    define: Vec<u8>,
    parts: Vec<Vec<u8>>,
}

struct State {
    captured: HashMap<String, CapturedModel>,
    in_progress: HashMap<u8, InProgress>,
    // Which of the 64 custom-model slots the server has told us are in use.
    occupied: [bool; 64],
    // The slot we last injected a pet model into (if any).
    pet_slot: Option<u8>,
    // Set while we replay our own pet defines through Protocol.Handlers, so the
    // capture hooks skip them. They target pet_slot, so without this they would
    // be mistaken for a foreign collision and self-revert the pet.
    injecting: bool,
}

impl Default for State {
    fn default() -> Self {
        Self {
            captured: HashMap::new(),
            in_progress: HashMap::new(),
            occupied: [false; 64],
            pet_slot: None,
            injecting: false,
        }
    }
}

pub struct CustomModelsModule {
    state: Rc<RefCell<State>>,
    hook_define: Option<ProtocolHook>,
    hook_part: Option<ProtocolHook>,
    hook_undef: Option<ProtocolHook>,
}

// -- Pure helpers (no FFI, unit-tested) -----------------------------------

/// Extract the model name from a DefineModel payload, trimmed at the first
/// NUL or space in the fixed 64-byte name field.
#[cfg(test)]
fn parse_name(data: &[u8]) -> String {
    let Ok(pkt) = DefineModelPayload::ref_from_bytes(data) else {
        return String::new();
    };
    let end = pkt
        .name
        .iter()
        .position(|&b| b == b'\0' || b == b' ')
        .unwrap_or(64);
    String::from_utf8_lossy(&pkt.name[..end]).into_owned()
}

/// Clone a DefineModel payload with the slot id and model name replaced.
/// The name is NUL-padded to 64 bytes; all other bytes are left verbatim.
fn patch_define(data: &[u8], slot: u8, new_name: &str) -> Vec<u8> {
    let mut out = data.to_vec();
    if let Ok(pkt) = DefineModelPayload::mut_from_bytes(&mut out) {
        pkt.id = slot;
        let name_bytes = new_name.as_bytes();
        let len = name_bytes.len().min(64);
        pkt.name.fill(0);
        pkt.name[..len].copy_from_slice(&name_bytes[..len]);
    }
    out
}

/// Clone a DefineModelPart payload with byte 0 (model id) replaced by `slot`.
fn patch_part(data: &[u8], slot: u8) -> Vec<u8> {
    let mut out = data.to_vec();
    if let Ok(pkt) = DefineModelPartPayload::mut_from_bytes(&mut out) {
        pkt.id = slot;
    }
    out
}

/// Return the highest unoccupied slot index, or `None` if all 64 are taken.
/// Picking from the high end avoids colliding with servers that allocate
/// models from slot 0 upward.
fn pick_free_slot(occupied: &[bool; 64]) -> Option<u8> {
    (0..64u8).rev().find(|&i| !occupied[i as usize])
}

// -- Capture handlers (called from ProtocolHook closures) -----------------

/// Capture a DefineModel packet. Returns `true` when a foreign server define
/// lands on the slot the pet is using -- a collision the caller must react to
/// by reverting the pet off the soon-to-be-reused slot. Pure (no FFI) so it can
/// be unit-tested; the revert side effect lives in `on_define`.
fn handle_define(state: &mut State, data: &[u8]) -> bool {
    // Skip the pet defines we replay ourselves (see copy_entity_model_to_pet).
    // They target pet_slot, so without this guard they would self-trigger the
    // collision check below; we also must not recapture them.
    if state.injecting {
        return false;
    }
    let Ok(pkt) = DefineModelPayload::ref_from_bytes(data) else {
        return false;
    };
    // Any foreign define landing on the pet's slot is a collision -- even a
    // server model whose name happens to start with "pet_" -- because once that
    // slot is reused the pet's Model* dangles. Checked before the name filter so
    // a "pet_"-named server model still reverts the pet.
    let revert = state.pet_slot == Some(pkt.id);
    if revert {
        state.pet_slot = None;
    }
    let end = pkt
        .name
        .iter()
        .position(|&b| b == b'\0' || b == b' ')
        .unwrap_or(64);
    let name = String::from_utf8_lossy(&pkt.name[..end]).into_owned();
    // Don't record "pet_"-named models in the replay map; that prefix is our own
    // injection naming scheme. Any collision above is already handled.
    if name.starts_with("pet_") {
        return revert;
    }
    state.occupied[pkt.id as usize] = true;
    state.in_progress.insert(
        pkt.id,
        InProgress {
            name,
            num_parts: pkt.num_parts,
            define: data.to_vec(),
            parts: Vec::new(),
        },
    );
    revert
}

fn handle_part(state: &mut State, data: &[u8]) {
    let Ok(pkt) = DefineModelPartPayload::ref_from_bytes(data) else {
        return;
    };
    let id = pkt.id;
    let Some(ip) = state.in_progress.get_mut(&id) else {
        return;
    };
    ip.parts.push(data.to_vec());
    if ip.parts.len() as u8 == ip.num_parts {
        let ip = state.in_progress.remove(&id).unwrap();
        debug!(
            "captured custom model '{}' ({} parts)",
            ip.name,
            ip.parts.len()
        );
        state.captured.insert(
            ip.name,
            CapturedModel {
                define: ip.define,
                parts: ip.parts,
            },
        );
    }
}

/// Process an UndefineModel packet. Returns `true` when the server undefines
/// the slot the pet is using -- a collision the caller must react to by
/// reverting the pet before its vertex data is freed. Pure (no FFI) so it can
/// be unit-tested; the revert side effect lives in `on_undef`.
fn handle_undef(state: &mut State, data: &[u8]) -> bool {
    if data.is_empty() {
        return false;
    }
    let id = data[0];
    let revert = state.pet_slot == Some(id);
    if revert {
        state.pet_slot = None;
    }
    state.occupied[id as usize] = false;
    state.in_progress.remove(&id);
    revert
}

// The closure entry points below add the FFI side effect -- reverting the pet
// on a slot collision -- on top of the pure, unit-tested handle_* state
// machine. Keeping the FFI out of handle_* is what lets the test binary link
// without ClassiCube symbols: nothing a test calls reaches the pet FFI, so
// dead-code elimination drops it.

fn on_define(state: &mut State, data: &[u8]) {
    if handle_define(state, data) {
        pet::reset_pet_to_default_model();
    }
}

fn on_undef(state: &mut State, data: &[u8]) {
    if handle_undef(state, data) {
        pet::reset_pet_to_default_model();
    }
}

/// Return `true` if ClassiCube already knows a model by this name -- a built-in
/// (e.g. "chicken", "humanoid") or any model already registered in models_head.
/// `Model_Get` returns null for an unknown name. FFI, so not test-reachable.
fn model_exists(name: &str) -> bool {
    let owned = OwnedString::new(name);
    // SAFETY: Model_Get only reads the cc_string we pass; owned outlives the call.
    !unsafe { Model_Get(owned.as_cc_string()) }.is_null()
}

/// Replay an UndefineModel packet for `slot` through ClassiCube's handler so it
/// frees the model's vertex data. FFI, so not test-reachable.
fn undefine_slot(slot: u8) {
    let handler = unsafe { Protocol.Handlers[OPCODE__OPCODE_UNDEFINE_MODEL as usize] };
    if let Some(f) = handler {
        let mut payload = vec![slot];
        // SAFETY: the handler reads a 1-byte UndefineModel payload (the slot id).
        unsafe { f(payload.as_mut_ptr()) };
    }
}

// -- Module ----------------------------------------------------------------

impl CustomModelsModule {
    pub fn init() -> Self {
        let state: Rc<RefCell<State>> = Default::default();
        CUSTOM_MODELS_STATE.with_borrow_mut(|slot| *slot = Rc::downgrade(&state));

        let weak = Rc::downgrade(&state);
        let hook_define = ProtocolHook::install(OPCODE__OPCODE_DEFINE_MODEL as u8, move |data| {
            if let Some(s) = weak.upgrade() {
                on_define(&mut s.borrow_mut(), data);
            }
            false
        });

        let weak = Rc::downgrade(&state);
        let hook_part =
            ProtocolHook::install(OPCODE__OPCODE_DEFINE_MODEL_PART as u8, move |data| {
                if let Some(s) = weak.upgrade() {
                    handle_part(&mut s.borrow_mut(), data);
                }
                false
            });

        let weak = Rc::downgrade(&state);
        let hook_undef = ProtocolHook::install(OPCODE__OPCODE_UNDEFINE_MODEL as u8, move |data| {
            if let Some(s) = weak.upgrade() {
                on_undef(&mut s.borrow_mut(), data);
            }
            false
        });

        Self {
            state,
            hook_define,
            hook_part,
            hook_undef,
        }
    }
}

impl Module for CustomModelsModule {
    fn reset(&mut self) {
        // Re-wrap the handlers: ClassiCube re-registers its own handlers on
        // every (re)connect, which would bury our trampolines.
        if let Some(h) = &self.hook_define {
            h.reinstall();
        }
        if let Some(h) = &self.hook_part {
            h.reinstall();
        }
        if let Some(h) = &self.hook_undef {
            h.reinstall();
        }
        // The new connection will send a fresh set of DefineModel packets, so
        // the old captured set is stale.
        let mut s = self.state.borrow_mut();
        s.captured.clear();
        s.in_progress.clear();
        s.occupied = [false; 64];
        s.pet_slot = None;
        s.injecting = false;
    }

    fn free(&mut self) {
        self.hook_define = None;
        self.hook_part = None;
        self.hook_undef = None;
    }
}

// -- Command entry point ---------------------------------------------------

/// Copy an entity's current model onto the pet.
///
/// `entity_id` is the ClassiCube entity slot (0-255; 255 = local player).
///
/// Reads the entity's active model name. A captured custom model is cloned into
/// a free slot under a `pet_` name and injected; a name ClassiCube already knows
/// (a built-in like "chicken", or any registered model) is applied to the pet
/// directly with no slot allocation. Either way `pet::set_pet_model` then copies
/// the entity's resolved skin.
///
/// Returns `Ok(model_name)` on success, `Err(chat_message)` on any failure.
pub fn copy_entity_model_to_pet(entity_id: u8) -> Result<String, String> {
    let Some(state) = CUSTOM_MODELS_STATE.with_borrow(Weak::upgrade) else {
        return Err("[Pet] Custom models module not active".to_string());
    };

    // Read the entity's active model name (Model.name, no |scale suffix).
    let original_name = unsafe {
        let Some(entity) = Entity::from_id(entity_id) else {
            return Err("[Pet] Entity not available (are you in a world?)".to_string());
        };
        let Some(model) = entity.get_model() else {
            return Err("[Pet] Entity has no model".to_string());
        };
        if model.name.is_null() {
            return Err("[Pet] Entity model has no name".to_string());
        }
        CStr::from_ptr(model.name).to_string_lossy().into_owned()
    };

    // Snapshot the captured payloads (if any) under a short borrow, then drop it.
    let captured = {
        let s = state.borrow();
        s.captured
            .get(&original_name)
            .map(|c| (c.define.clone(), c.parts.clone()))
    };

    let Some((define_data, parts_data)) = captured else {
        // Not a captured custom model. If ClassiCube already knows this name --
        // a built-in (e.g. "chicken", "humanoid") or any already-registered
        // model -- point the pet straight at it. Built-ins live in models_head
        // for the whole process and are never freed, so there's no slot to
        // allocate or guard against server reuse.
        if !model_exists(&original_name) {
            return Err(format!(
                "[Pet] Model '{original_name}' not yet captured -- try /reload"
            ));
        }
        if !pet::set_pet_model(&original_name, entity_id) {
            return Err("[Pet] Pet not available (are you in a world?)".to_string());
        }
        // Release any custom slot the pet held before: drop our claim first so
        // the undefine isn't seen as a collision, then free its vertex data.
        // Otherwise the stale pet_slot leaks and the collision guard could later
        // revert this built-in pet off a slot it no longer uses.
        let old_slot = {
            let mut s = state.borrow_mut();
            let old = s.pet_slot.take();
            if let Some(old) = old {
                s.occupied[old as usize] = false;
            }
            old
        };
        if let Some(old_id) = old_slot {
            undefine_slot(old_id);
        }
        info!("applied built-in model '{}'", original_name);
        return Ok(original_name);
    };

    // -- captured custom-model path --

    // TODO don't like this -- can we detect CPE support more robustly?
    // Custom models v2 sets Sizes[DEFINE_MODEL_PART] to 167 during extension
    // negotiation. Anything else means the server doesn't support them. (A
    // captured entry implies the server sent these packets, but keep the guard
    // so injection never runs against an unprepared handler.)
    let part_size: u16 = unsafe { Protocol.Sizes[OPCODE__OPCODE_DEFINE_MODEL_PART as usize] };
    if part_size != 167 {
        return Err("[Pet] Server does not support custom models v2".to_string());
    }

    // Prepare patched payloads under a single borrow (no FFI calls in here).
    // We release the borrow fully before injecting to avoid re-entrancy on the
    // capture hooks' own RefCell borrows.
    let (old_slot, new_slot, pet_name, patched_define, patched_parts) = {
        let mut s = state.borrow_mut();

        // Pet model name has a "pet_" prefix so it cannot clash in models_head
        // with the server's same-named model (our replay is skipped via the
        // `injecting` flag, not the name).
        let raw_pet_name = format!("pet_{original_name}");
        let pet_name: String = raw_pet_name.chars().take(64).collect();

        let new_slot =
            pick_free_slot(&s.occupied).ok_or("[Pet] All 64 custom model slots are in use")?;

        let old_slot = s.pet_slot;
        if let Some(old) = old_slot {
            s.occupied[old as usize] = false;
        }
        s.occupied[new_slot as usize] = true;
        s.pet_slot = Some(new_slot);

        let patched_define = patch_define(&define_data, new_slot, &pet_name);
        let patched_parts: Vec<Vec<u8>> =
            parts_data.iter().map(|p| patch_part(p, new_slot)).collect();

        (old_slot, new_slot, pet_name, patched_define, patched_parts)
    };
    // State borrow fully released; injection is re-entrant-safe from here on.

    // Mark the replay window so the capture hooks skip the defines/parts we are
    // about to inject (which target pet_slot and would otherwise self-trigger
    // the collision revert). Cleared below before any further work.
    state.borrow_mut().injecting = true;

    // Undefine the previous pet model slot so ClassiCube frees its vertex data.
    if let Some(old_id) = old_slot {
        undefine_slot(old_id);
    }

    // Inject DefineModel (our trampoline forwards to ClassiCube, which allocates
    // the slot; our capture hook skips it because `injecting` is set).
    let handler = unsafe { Protocol.Handlers[OPCODE__OPCODE_DEFINE_MODEL as usize] };
    if let Some(f) = handler {
        let mut payload = patched_define;
        unsafe { f(payload.as_mut_ptr()) };
    }

    // Inject one DefineModelPart per part. The last part triggers
    // ClassiCube's internal CustomModel_Register, adding it to models_head.
    let handler = unsafe { Protocol.Handlers[OPCODE__OPCODE_DEFINE_MODEL_PART as usize] };
    if let Some(f) = handler {
        for mut payload in patched_parts {
            unsafe { f(payload.as_mut_ptr()) };
        }
    }

    // Replay done: subsequent defines on pet_slot are foreign collisions again.
    state.borrow_mut().injecting = false;

    // Apply the model to the pet and copy the source entity's resolved skin.
    if !pet::set_pet_model(&pet_name, entity_id) {
        return Err("[Pet] Pet not available (are you in a world?)".to_string());
    }

    info!(
        "applied custom model '{}' as '{}' in slot {}",
        original_name, pet_name, new_slot
    );
    Ok(original_name)
}
