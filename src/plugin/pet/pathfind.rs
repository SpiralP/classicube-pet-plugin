#[cfg(test)]
mod tests;

use std::collections::VecDeque;

use classicube_sys::{
    Blocks, CollideType, CollideType_COLLIDE_ICE, CollideType_COLLIDE_SLIPPERY_ICE,
    CollideType_COLLIDE_SOLID, Vec3, World, World_GetBlock,
};

pub const GROUND_STEP_TOLERANCE: f32 = 0.5; // ClassiCube StepSize (EntityComponents.c:729)
use pathfinding::prelude::astar;

pub const WALK_SPEED: f32 = 1.0; // blocks/sec
pub const MAX_STEP_PER_FRAME: f32 = 0.9; // blocks; < 1 cell prevents tunneling on large delta
pub const MAX_FALL: i32 = 3; // cells
pub const MAX_PATH_DISTANCE: i32 = 64; // horizontal Manhattan cap before teleport fallback
pub const MAX_EXPLORED_NODES: usize = 20_000;

/// A grid cell. `y` is the **feet cell**: the pet's feet rest at world-y == `y`,
/// standing on the solid block at `y - 1`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub struct Cell {
    pub x: i32,
    pub y: i32,
    pub z: i32,
}

/// Abstraction over the world's block grid for use by pathfinding logic.
/// `is_solid` must return `false` for out-of-bounds coordinates so callers
/// need not bounds-check before every call.
pub trait Grid {
    fn is_solid(&self, x: i32, y: i32, z: i32) -> bool;

    /// World-Y of the top face of the (assumed solid) block in cell (x, y, z).
    /// Default = full cube (`y + 1.0`). `WorldGrid` overrides with `Blocks.MaxBB[block].y`
    /// so slabs (0.5), snow (0.25), etc. sit at their true surface.
    fn surface_top(&self, _x: i32, y: i32, _z: i32) -> f32 {
        y as f32 + 1.0
    }

    /// World-Y the pet's feet rest on, given its horizontal footprint
    /// `[min_x, max_x] x [min_z, max_z]` (world coords) and current `feet_y`.
    ///
    /// Scans every cell column the footprint overlaps; for each, finds the
    /// highest solid cell from one `GROUND_STEP_TOLERANCE` above the feet down
    /// to `MAX_FALL + 1` below, and returns the **maximum** `surface_top`
    /// across all columns -- mirroring `Respawn_HighestSolidY` (World.c).
    /// `None` if no column has anything standable in range. Taking the max
    /// makes the pet step up as soon as its footprint reaches a higher block
    /// (no model-into-riser clip) and stay supported on a ledge until the
    /// footprint fully clears it.
    fn ground_surface_y(
        &self,
        min_x: f32,
        max_x: f32,
        min_z: f32,
        max_z: f32,
        feet_y: f32,
    ) -> Option<f32> {
        #[expect(
            clippy::cast_possible_truncation,
            reason = "ClassiCube world coordinates fit in i32"
        )]
        let (hi, lo) = (
            (feet_y + GROUND_STEP_TOLERANCE).floor() as i32,
            feet_y.floor() as i32 - MAX_FALL - 1,
        );
        #[expect(
            clippy::cast_possible_truncation,
            reason = "ClassiCube world coordinates fit in i32"
        )]
        let (cx0, cx1, cz0, cz1) = (
            min_x.floor() as i32,
            max_x.floor() as i32,
            min_z.floor() as i32,
            max_z.floor() as i32,
        );
        let mut best: Option<f32> = None;
        for cx in cx0..=cx1 {
            for cz in cz0..=cz1 {
                if let Some(s) = (lo..=hi).rev().find_map(|cy| {
                    self.is_solid(cx, cy, cz)
                        .then(|| self.surface_top(cx, cy, cz))
                }) {
                    best = Some(best.map_or(s, |b| b.max(s)));
                }
            }
        }
        best
    }
}

fn is_passable<G: Grid>(g: &G, x: i32, y: i32, z: i32) -> bool {
    !g.is_solid(x, y, z)
}

/// True when the pet can stand in cell `c`: solid ground at `c.y - 1`, and
/// both the body cell (`c.y`) and headroom cell (`c.y + 1`) are passable.
pub fn can_stand<G: Grid>(g: &G, c: Cell) -> bool {
    g.is_solid(c.x, c.y - 1, c.z)
        && is_passable(g, c.x, c.y, c.z)
        && is_passable(g, c.x, c.y + 1, c.z)
}

/// Snap `c` down to the nearest standable cell within `MAX_FALL`, or return
/// `None` if no such cell exists.
fn snap_to_ground<G: Grid>(g: &G, c: Cell) -> Option<Cell> {
    if can_stand(g, c) {
        return Some(c);
    }
    for dy in 1..=MAX_FALL {
        let candidate = Cell {
            x: c.x,
            y: c.y - dy,
            z: c.z,
        };
        if can_stand(g, candidate) {
            return Some(candidate);
        }
    }
    None
}

/// Return standable neighbors of `c` for A*. For each of the 4 orthogonal
/// horizontal directions, find the first standable `y'` searching from
/// step-up (+1) down to `MAX_FALL` below the current y.
pub fn cell_successors<G: Grid>(g: &G, c: Cell) -> Vec<(Cell, u32)> {
    const DIRS: [(i32, i32); 4] = [(1, 0), (-1, 0), (0, 1), (0, -1)];
    let mut out = Vec::new();

    for (dx, dz) in DIRS {
        let tx = c.x + dx;
        let tz = c.z + dz;

        // Search from step-up (+1) down to max-fall; stop at first standable y.
        for dy in (-MAX_FALL..=1).rev() {
            // Stepping up: the source also needs headroom at y+2 so the pet's
            // head clears the ceiling while lifting.
            if dy == 1 && !is_passable(g, c.x, c.y + 2, c.z) {
                continue;
            }

            let ty = c.y + dy;
            let target = Cell {
                x: tx,
                y: ty,
                z: tz,
            };
            if can_stand(g, target) {
                // Prefer flat routes; add a small vertical penalty so A* avoids
                // unnecessary climbing or falling.
                let cost = 10 + 4 * dy.unsigned_abs();
                out.push((target, cost));
                break;
            }
        }
    }

    out
}

/// Compute an A* path from `start` to `goal` using the given grid. Both
/// endpoints are snapped to standable ground first. Returns `None` when
/// there is no path, the endpoints can't be grounded, or the search exceeds
/// `max_explored` node expansions (teleport-fallback signal).
///
/// Returns the full cell path including both endpoints.
pub fn find_path<G: Grid>(
    g: &G,
    start: Cell,
    goal: Cell,
    max_explored: usize,
) -> Option<Vec<Cell>> {
    let start = snap_to_ground(g, start)?;
    let goal = snap_to_ground(g, goal)?;

    if start == goal {
        return Some(vec![start]);
    }

    let mut explored = 0usize;

    astar(
        &start,
        |&c| {
            if explored >= max_explored {
                return Vec::new();
            }
            explored += 1;
            cell_successors(g, c)
        },
        |&c| {
            let dx = (c.x - goal.x).unsigned_abs();
            let dz = (c.z - goal.z).unsigned_abs();
            10 * (dx + dz)
        },
        |&c| c == goal,
    )
    .map(|(path, _)| path)
}

/// Result of a single `step_walk` call.
///
/// ClassiCube draws an entity with two independent rotations (see
/// `Entity_GetTransform` + `Model.c`): the whole model is rotated by `RotY`
/// (the **body** yaw), and head parts get an extra `Yaw - RotY` so the head's
/// absolute facing is `Yaw`. We exploit that to let the pet's body face where
/// it walks while its head keeps looking at the goal.
pub struct WalkResult {
    /// XZ advanced toward the next waypoint; **Y is left equal to the input
    /// `position.y`** -- the caller is responsible for snapping feet to the
    /// block surface via `Grid::ground_surface_y`.
    pub position: Vec3,
    /// `entity.Pitch` -- head pitch, tilted toward the goal.
    pub head_pitch: f32,
    /// `entity.Yaw` -- head yaw, aimed at the goal (the last waypoint).
    pub head_yaw: f32,
    /// `entity.RotY` -- body yaw, facing the direction of travel (next
    /// waypoint).
    pub body_yaw: f32,
}

/// The yaw that faces horizontal direction `(dx, dz)`.
///
/// Matches ClassiCube's `Vec3_GetDirVector` convention (`Vectors.c`): for a
/// facing of `(x, z)` it computes `x = sin(yaw)`, `z = -cos(yaw)`, so the yaw
/// that points along `(dx, dz)` is `atan2(dx, -dz)`. The `-dz` is what makes +Z
/// face yaw 180 (forward) rather than 0 (backward).
fn facing_yaw(dx: f32, dz: f32) -> f32 {
    dx.atan2(-dz).to_degrees()
}

/// The pitch that tilts toward vertical delta `dy` over horizontal distance
/// `horiz`. ClassiCube's dir vector has `y = -sin(pitch)` with horizontal
/// magnitude `cos(pitch)`, so `pitch = atan2(-dy, horiz)`: a goal below (dy < 0)
/// yields positive pitch (looking down), a goal above yields negative (up).
fn facing_pitch(dy: f32, horiz: f32) -> f32 {
    (-dy).atan2(horiz).to_degrees()
}

/// Advance `position` horizontally toward the front of `path` over one frame
/// of `delta` seconds, popping reached waypoints. Caps the per-frame move to
/// `MAX_STEP_PER_FRAME` so a stalled frame can't tunnel through walls.
///
/// **Y is not written.** The returned `position.y` equals the input. The
/// caller is responsible for snapping the feet to the block surface each frame
/// via `Grid::ground_surface_y`; this keeps the pet on top of steps and partial
/// blocks instead of gliding diagonally through block corners.
///
/// Aims the **body** (`body_yaw` -> `entity.RotY`) at the next waypoint it is
/// walking toward, and the **head** (`head_pitch`/`head_yaw` ->
/// `entity.Pitch`/`entity.Yaw`) at the goal -- the last waypoint in `path`,
/// recomputed from the post-move position so the head tracks the goal as the
/// pet closes in. Head aim still reads the goal's Y for pitch so the pet looks
/// up/down toward higher or lower destinations. Angles that would be undefined
/// (no horizontal component) fall back to the passed-in current values rather
/// than snapping.
pub fn step_walk(
    position: Vec3,
    current_pitch: f32,
    current_yaw: f32,
    current_body_yaw: f32,
    path: &mut VecDeque<Vec3>,
    delta: f32,
) -> WalkResult {
    let mut pos = position;
    let mut body_yaw = current_body_yaw;
    let mut budget = (WALK_SPEED * delta).min(MAX_STEP_PER_FRAME);

    // The goal is the final waypoint; it stays fixed for the whole walk (the
    // path only shrinks from the front), so read it before consuming any.
    let goal = path.back().copied();

    while budget > f32::EPSILON {
        let Some(&target) = path.front() else { break };

        let dx = target.x - pos.x;
        let dz = target.z - pos.z;
        let horiz = (dx * dx + dz * dz).sqrt();

        if horiz < f32::EPSILON {
            // No horizontal component (duplicate or purely-vertical waypoint);
            // pop without consuming budget. Y is owned by the ground snap.
            path.pop_front();
            continue;
        }

        if horiz <= budget {
            // Reach this waypoint; face the direction of travel and snap XZ.
            body_yaw = facing_yaw(dx, dz);
            pos.x = target.x;
            pos.z = target.z;
            budget -= horiz;
            path.pop_front();
        } else {
            // Partial move toward waypoint (XZ only).
            body_yaw = facing_yaw(dx, dz);
            let t = budget / horiz;
            pos.x += dx * t;
            pos.z += dz * t;
            break;
        }
    }

    // Head looks at the goal from wherever the pet ended up this frame.
    let (head_pitch, head_yaw) = match goal {
        Some(goal) => {
            let dx = goal.x - pos.x;
            let dy = goal.y - pos.y;
            let dz = goal.z - pos.z;
            let horiz = (dx * dx + dz * dz).sqrt();
            if horiz > f32::EPSILON {
                (facing_pitch(dy, horiz), facing_yaw(dx, dz))
            } else {
                (current_pitch, current_yaw)
            }
        }
        None => (current_pitch, current_yaw),
    };

    WalkResult {
        position: pos,
        head_pitch,
        head_yaw,
        body_yaw,
    }
}

// ---------------------------------------------------------------------------
// FFI-backed real-world grid -- NOT test-reachable.
// WorldGrid reads ClassiCube's extern statics (World, Blocks) and calls
// World_GetBlock; --gc-sections drops all of this from the test binary because
// nothing reachable from #[test] ever calls these functions.
// ---------------------------------------------------------------------------

/// A `Grid` implementation backed by ClassiCube's live world data.
pub struct WorldGrid;

impl Grid for WorldGrid {
    fn is_solid(&self, x: i32, y: i32, z: i32) -> bool {
        // SAFETY: World and Blocks are valid ClassiCube globals. Bounds check
        // prevents World_GetBlock from panicking on out-of-range coordinates.
        // ICE and SLIPPERY_ICE are stored as COLLIDE_SOLID in Blocks.Collide
        // (ClassiCube's Block.c normalises them), so one comparison suffices.
        unsafe {
            if x < 0 || y < 0 || z < 0 || x >= World.Width || y >= World.Height || z >= World.Length
            {
                return false;
            }
            let id = World_GetBlock(x, y, z);
            if id == 0 {
                return false;
            }

            let c = Blocks.Collide[id as usize] as CollideType;
            c == CollideType_COLLIDE_SOLID
                || c == CollideType_COLLIDE_ICE
                || c == CollideType_COLLIDE_SLIPPERY_ICE
        }
    }

    fn surface_top(&self, x: i32, y: i32, z: i32) -> f32 {
        // SAFETY: ground_surface_y only calls surface_top for cells where
        // is_solid returned true, so (x,y,z) is already bounds-checked and
        // non-air. Mirrors ClassiCube Entity.c spawn logic: feet rest at
        // cell_y + Blocks.MaxBB[block].y (slabs = 0.5, snow = 0.25, etc.).
        unsafe { y as f32 + Blocks.MaxBB[World_GetBlock(x, y, z) as usize].y }
    }
}

/// Returns `true` when the world is loaded and it is safe to call
/// `World_GetBlock`.
pub fn world_is_loaded() -> bool {
    // SAFETY: World is a valid global.
    unsafe { World.Loaded != 0 }
}
