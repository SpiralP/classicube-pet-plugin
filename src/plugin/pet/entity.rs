#[cfg(test)]
mod tests;

use std::mem;

use classicube_sys::{
    Entity, Entity_Init, EntityVTABLE, LocationUpdate, Model_Render, PACKEDCOL_WHITE, PackedCol,
    Vec3, cc_bool,
};

use super::OFFSET;

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

        entity.VTABLE = vtable.as_ref();

        Self {
            entity,
            _vtable: vtable,
        }
    }

    /// Called every frame from the render hook: sync position/model from the
    /// local player then draw.
    ///
    /// # Safety
    /// `local_player` must be a valid non-null pointer to the local player
    /// Entity, which remains valid for the duration of this call.
    pub unsafe fn update_and_render(&mut self, local_player: *mut Entity) {
        let lp = unsafe { &*local_player };

        // Mirror the player's model pointer directly so the pet tracks model
        // changes (e.g. CPE ChangeModel) automatically.
        self.entity.Model = lp.Model;
        self.entity.ModelScale = lp.ModelScale;

        self.entity.Position = offset_position(lp.Position, OFFSET);
        self.entity.Yaw = lp.Yaw;
        self.entity.RotY = lp.RotY;
        self.entity.Pitch = lp.Pitch;

        let e = self.entity.as_mut();
        // SAFETY: entity.Model was just set from the local player's non-null
        // model pointer, and entity is a valid Entity.
        unsafe { Model_Render(e.Model, e) };
    }
}

/// Offset `base` by `off` component-wise (pure, testable without FFI).
pub fn offset_position(base: Vec3, off: Vec3) -> Vec3 {
    Vec3::new(base.x + off.x, base.y + off.y, base.z + off.z)
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
