#[cfg(test)]
mod tests;

use std::mem;

use classicube_sys::{
    Entity, Entity_Init, Entity_SetModel, EntityVTABLE, LocationUpdate, Model_Render, OwnedString,
    PACKEDCOL_WHITE, PackedCol, Vec3, cc_bool,
};

pub(super) const PET_MODEL: &str = "chicken";
pub(super) const PET_MODEL_SCALE: f32 = 0.5;

/// The spec passed to `Entity_SetModel`: model name plus a `|scale` suffix
/// that ClassiCube parses into the entity's `ModelScale`.
fn model_spec(name: &str) -> String {
    format!("{name}|{PET_MODEL_SCALE}")
}

pub struct PetEntity {
    pub entity: Box<Entity>,
    // VTABLE must be heap-allocated: the entity holds a raw pointer to it that
    // must remain valid for the lifetime of the PetEntity.
    _vtable: Box<EntityVTABLE>,
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
        let spec = OwnedString::new(model_spec(PET_MODEL));
        // SAFETY: entity is a valid initialized Entity; spec outlives the call.
        unsafe { Entity_SetModel(&mut *entity, spec.as_cc_string()) };

        entity.VTABLE = vtable.as_ref();

        Self {
            entity,
            _vtable: vtable,
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

    /// Swap the pet to a different model. Texture state (`TextureId`,
    /// `SkinType`) is managed by `pet::skin` and the skin fields (`SkinRaw`,
    /// `NonHumanSkin`) by `copy_skin_from`; none are touched here --
    /// `Entity_SetModel` does not modify those fields, so an already-applied
    /// owned skin persists across a model swap.
    pub fn set_model(&mut self, name: &str) {
        let spec = OwnedString::new(model_spec(name));
        // SAFETY: entity is a valid initialized Entity; spec outlives the call.
        unsafe { Entity_SetModel(&mut *self.entity, spec.as_cc_string()) };
    }

    /// Mirror the skin fields ClassiCube's `Entity_SetSkin` writes -- `SkinRaw`
    /// and `NonHumanSkin` -- from the source entity (the player whose model the
    /// pet is copying). `Model_ApplyTexture` honours `e.TextureId` only when
    /// `model.usesHumanSkin || e.NonHumanSkin`, so mirroring `NonHumanSkin`
    /// makes the pet honour its own texture exactly when the source does.
    /// `SkinRaw` is inert on the standalone pet (it's not in `Entities.List`, so
    /// the engine's skin-fetch state machine never runs off it) but is copied to
    /// keep the pet a faithful clone. ClassiCube does the same `NonHumanSkin`
    /// copy when deriving the held-block entity (`HeldBlockRenderer.c`:
    /// `held_entity.NonHumanSkin = p->NonHumanSkin`).
    pub fn copy_skin_from(&mut self, src: &Entity) {
        self.entity.SkinRaw = src.SkinRaw;
        self.entity.NonHumanSkin = src.NonHumanSkin;
    }

    /// Revert the pet to its built-in default model. PET_MODEL is built-in and
    /// never freed by the engine, so after this `entity.Model` can no longer
    /// dangle even when every custom-model slot is freed (Game_Reset's
    /// CustomModel_FreeAll, or the server reusing the slot). Skin is cleared
    /// separately by the caller via `skin::clear()`.
    pub fn reset_to_default_model(&mut self) {
        self.set_model(PET_MODEL);
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
