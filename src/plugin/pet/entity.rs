#[cfg(test)]
mod tests;

use std::{collections::VecDeque, mem, os::raw::c_int};

use classicube_sys::{
    Entity, Entity_Init, Entity_SetModel, EntityVTABLE, Lighting, LocationUpdate, Model_Render,
    OwnedString, PACKEDCOL_WHITE, PackedCol, Vec3, cc_bool,
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
    /// Remaining world-space waypoint centers. Empty == idle. Owned plain data;
    /// dropped automatically with the pet on Free, so reload safety is free.
    walk_path: VecDeque<Vec3>,
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
            walk_path: VecDeque::new(),
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

    /// Mirror the source entity's `ModelBlock` -- the block id the shared
    /// `"block"` model renders. For a block model (`/model stone`) the id lives
    /// only in `ModelBlock`, never in the model name; without this mirror the
    /// pet's block model renders `BLOCK_AIR` (nothing). For non-block models the
    /// source's `ModelBlock` is `BLOCK_AIR` and this is a no-op.
    ///
    /// Must be called *after* `set_model`, because `Entity_SetModel` resets
    /// `ModelBlock` to `BLOCK_AIR` unconditionally. Mirrors `HeldBlockRenderer.c`:
    /// `held_entity.ModelBlock = held_block`.
    pub fn copy_model_block_from(&mut self, src: &Entity) {
        self.entity.ModelBlock = src.ModelBlock;
    }

    /// Revert the pet to its built-in default model. PET_MODEL is built-in and
    /// never freed by the engine, so after this `entity.Model` can no longer
    /// dangle even when every custom-model slot is freed (Game_Reset's
    /// CustomModel_FreeAll, or the server reusing the slot). Skin is cleared
    /// separately by the caller via `skin::clear()`.
    pub fn reset_to_default_model(&mut self) {
        self.set_model(PET_MODEL);
    }

    pub fn position(&self) -> Vec3 {
        self.entity.Position
    }

    /// Replace the current walk path with `waypoints`. Any in-progress walk is
    /// discarded; the pet immediately starts heading for the first new waypoint.
    pub fn set_walk_path(&mut self, waypoints: VecDeque<Vec3>) {
        self.walk_path = waypoints;
    }

    #[expect(
        dead_code,
        reason = "available for future use; not yet called from pet.rs"
    )]
    pub fn is_walking(&self) -> bool {
        !self.walk_path.is_empty()
    }

    /// Discard any in-progress walk. Called on map change to avoid the pet
    /// walking toward stale coordinates from the previous map.
    pub fn stop_walk(&mut self) {
        self.walk_path.clear();
    }

    /// Called every frame from the render hook to advance the walk (if any)
    /// and draw the pet. `delta` is the frame delta in seconds from ClassiCube's
    /// `RenderModel` callback. Must run on the render thread.
    pub fn update_and_render(&mut self, delta: f32) {
        if !self.walk_path.is_empty() {
            let pos = self.entity.Position;
            let pitch = self.entity.Pitch;
            let yaw = self.entity.Yaw;
            let body_yaw = self.entity.RotY;
            let result =
                super::pathfind::step_walk(pos, pitch, yaw, body_yaw, &mut self.walk_path, delta);
            self.entity.Position = result.position;
            // Head looks at the goal; body faces the direction of travel.
            self.entity.Pitch = result.head_pitch;
            self.entity.Yaw = result.head_yaw;
            self.entity.RotY = result.body_yaw;
        }
        let e = self.entity.as_mut();
        // SAFETY: entity.Model is set at construction by Entity_SetModel and is
        // never null; entity is a valid Entity.
        unsafe { Model_Render(e.Model, e) };
    }
}

extern "C" fn noop_tick(_e: *mut Entity, _delta: f32) {}

extern "C" fn noop_despawn(_e: *mut Entity) {}

extern "C" fn noop_set_location(_e: *mut Entity, _update: *mut LocationUpdate) {}

/// Mirror ClassiCube's `Entity_GetColor` (`Entity.c`): sample the world
/// lighting engine at the pet's eye position so it darkens in shadow and
/// brightens in light, exactly like a real player/bot. Returning
/// `PACKEDCOL_WHITE` unconditionally made the pet fullbright.
///
/// `Entity_GetEyePosition`/`IVec3_Floor` aren't `CC_API`, so the eye position
/// is computed inline: `Position.y + model.GetEyeY(e) * ModelScale.y`, floored
/// (matches `Entity_GetEyeHeight`). `Lighting.Color` is a `CC_VAR` global
/// installed by the Lighting game component before any world renders; fall back
/// to white if it is somehow unset (e.g. outside a world).
///
/// SAFETY: `e` is the live pet entity ClassiCube passes into `GetCol` on the
/// render thread; `Model` is non-null (set at construction by
/// `Entity_SetModel`).
extern "C" fn get_col(e: *mut Entity) -> PackedCol {
    unsafe {
        let ent = &*e;
        let eye_y = match (*ent.Model).GetEyeY {
            Some(get_eye_y) => get_eye_y(e) * ent.ModelScale.y,
            None => 0.0,
        };
        let x = ent.Position.x.floor() as c_int;
        let y = (ent.Position.y + eye_y).floor() as c_int;
        let z = ent.Position.z.floor() as c_int;
        match Lighting.Color {
            Some(color) => color(x, y, z),
            None => PACKEDCOL_WHITE,
        }
    }
}

extern "C" fn noop_render_model(_e: *mut Entity, _delta: f32, _t: f32) {}

extern "C" fn should_render_name(_e: *mut Entity) -> cc_bool {
    0
}
