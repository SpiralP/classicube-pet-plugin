#[cfg(test)]
mod tests;

use std::mem;

use classicube_sys::{
    Entity, Entity_Init, Entity_SetModel, EntityVTABLE, GfxResourceID, LocationUpdate,
    Model_Render, OwnedString, PACKEDCOL_WHITE, PackedCol, Vec3, cc_bool, cc_uint8,
};

pub(super) const PET_MODEL: &str = "chicken";
pub(super) const PET_MODEL_SCALE: f32 = 0.5;

/// Skin state copied from the local player's entity so the pet renders with
/// the player's resolved texture rather than grey geometry.
#[derive(Copy, Clone)]
pub struct SkinFields {
    pub skin_type: cc_uint8,
    pub texture_id: GfxResourceID,
    pub non_human_skin: cc_bool,
    pub u_scale: f32,
    pub v_scale: f32,
}

/// The spec passed to `Entity_SetModel`: model name plus a `|scale` suffix
/// that ClassiCube parses into the entity's `ModelScale`.
fn model_spec() -> String {
    format!("{PET_MODEL}|{PET_MODEL_SCALE}")
}

pub struct PetEntity {
    pub entity: Box<Entity>,
    // VTABLE must be heap-allocated: the entity holds a raw pointer to it that
    // must remain valid for the lifetime of the PetEntity.
    _vtable: Box<EntityVTABLE>,
    // Skin state of a freshly-constructed pet (no custom skin), captured at
    // build time so `reset_to_default_model` can restore it without hardcoding
    // the zero value of GfxResourceID (backend-dependent: pointer or integer).
    default_skin: SkinFields,
}

impl PetEntity {
    pub fn new() -> Self {
        let vtable = Box::new(EntityVTABLE {
            Tick: Some(noop_tick),
            Despawn: Some(noop_despawn),
            SetLocation: Some(noop_set_location),
            GetCol: Some(get_col),
            RenderModel: Some(noop_render_model),
            ShouldRenderName: Some(should_render_name),
        });

        let mut entity: Box<Entity> = Box::new(unsafe { mem::zeroed() });

        // SAFETY: entity is a valid, zeroed Entity struct.
        unsafe { Entity_Init(&mut entity) };

        // Give the pet its own model + scale, independent of the local player.
        // Position and rotation default to world origin / no rotation; future
        // movement code mutates entity.Position / entity.Yaw etc. directly.
        let spec = OwnedString::new(model_spec());
        // SAFETY: entity is a valid initialized Entity; spec outlives the call.
        unsafe { Entity_SetModel(&mut *entity, spec.as_cc_string()) };

        entity.VTABLE = vtable.as_ref();

        // Capture the freshly-built skin state (Entity_Init defaults: no custom
        // skin, uScale/vScale = 1.0) so we can restore it on a model revert.
        let default_skin = SkinFields {
            skin_type: entity.SkinType,
            texture_id: entity.TextureId,
            non_human_skin: entity.NonHumanSkin,
            u_scale: entity.uScale,
            v_scale: entity.vScale,
        };

        Self {
            entity,
            _vtable: vtable,
            default_skin,
        }
    }

    /// Move the pet to a world position + view angles. Pure field writes; the
    /// next `update_and_render` draws it at the new transform.
    pub fn set_transform(&mut self, position: Vec3, pitch: f32, yaw: f32, rot: [f32; 3]) {
        let e = self.entity.as_mut();
        e.Position = position;
        e.Pitch = pitch;
        e.Yaw = yaw;
        let [rot_x, rot_y, rot_z] = rot;
        e.RotX = rot_x;
        e.RotY = rot_y;
        e.RotZ = rot_z;
    }

    /// Swap the pet to a different model and copy skin state from the local
    /// player so it textures correctly. Called from `pet::set_pet_model`.
    pub fn set_model(&mut self, name: &str, skin: SkinFields) {
        let spec = OwnedString::new(format!("{name}|{PET_MODEL_SCALE}"));
        // SAFETY: entity is a valid initialized Entity; spec outlives the call.
        unsafe { Entity_SetModel(&mut *self.entity, spec.as_cc_string()) };
        let e = self.entity.as_mut();
        e.SkinType = skin.skin_type;
        e.TextureId = skin.texture_id;
        e.NonHumanSkin = skin.non_human_skin;
        e.uScale = skin.u_scale;
        e.vScale = skin.v_scale;
    }

    /// Revert the pet to its built-in default model and drop any copied custom
    /// skin. PET_MODEL is built-in and never freed by the engine, so after this
    /// `entity.Model` can no longer dangle even when every custom-model slot is
    /// freed (Game_Reset's CustomModel_FreeAll, or the server reusing the
    /// slot). Unlike `pet::set_pet_model`, this reads no game state, so it is
    /// safe to call mid-Game_Reset / packet-handling when no world exists.
    pub fn reset_to_default_model(&mut self) {
        self.set_model(PET_MODEL, self.default_skin);
    }

    /// Called every frame from the render hook to draw the pet at its own
    /// position with its own model/scale. The pet owns all of its state;
    /// nothing here reads the local player. Must run on the render thread,
    /// which the `RenderModel` hook guarantees.
    pub fn update_and_render(&mut self) {
        let e = self.entity.as_mut();
        // SAFETY: entity.Model is set at construction by Entity_SetModel and is
        // never null; entity is a valid Entity.
        unsafe { Model_Render(e.Model, e) };
    }
}

extern "C" fn noop_tick(_e: *mut Entity, _delta: f32) {}

extern "C" fn noop_despawn(_e: *mut Entity) {}

extern "C" fn noop_set_location(_e: *mut Entity, _update: *mut LocationUpdate) {}

extern "C" fn get_col(_e: *mut Entity) -> PackedCol {
    PACKEDCOL_WHITE
}

extern "C" fn noop_render_model(_e: *mut Entity, _delta: f32, _t: f32) {}

extern "C" fn should_render_name(_e: *mut Entity) -> cc_bool {
    0
}
