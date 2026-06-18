#[cfg(test)]
mod tests;

use std::collections::VecDeque;

use classicube_sys::{
    Blocks, CollideType, CollideType_COLLIDE_ICE, CollideType_COLLIDE_SLIPPERY_ICE,
    CollideType_COLLIDE_SOLID, Vec3, World, World_GetBlock,
};
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

/// Advance `position` toward the front of `path` over one frame of `delta`
/// seconds, popping reached waypoints. Caps the per-frame move to
/// `MAX_STEP_PER_FRAME` so a stalled frame can't tunnel through walls.
///
/// Aims the **body** (`body_yaw` -> `entity.RotY`) at the next waypoint it is
/// walking toward, and the **head** (`head_pitch`/`head_yaw` ->
/// `entity.Pitch`/`entity.Yaw`) at the goal -- the last waypoint in `path`,
/// recomputed from the post-move position so the head tracks the goal as the
/// pet closes in. Angles that would be undefined (no horizontal component) fall
/// back to the passed-in current values rather than snapping.
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
        let dy = target.y - pos.y;
        let dz = target.z - pos.z;
        let dist = (dx * dx + dy * dy + dz * dz).sqrt();

        if dist < f32::EPSILON {
            // Duplicate or already-at waypoint; pop without consuming budget.
            path.pop_front();
            continue;
        }

        let horiz = (dx * dx + dz * dz).sqrt();

        if dist <= budget {
            // Reach this waypoint; face the direction of travel and snap.
            if horiz > f32::EPSILON {
                body_yaw = facing_yaw(dx, dz);
            }
            pos = target;
            budget -= dist;
            path.pop_front();
        } else {
            // Partial move toward waypoint.
            if horiz > f32::EPSILON {
                body_yaw = facing_yaw(dx, dz);
            }
            let t = budget / dist;
            pos.x += dx * t;
            pos.y += dy * t;
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
}

/// Returns `true` when the world is loaded and it is safe to call
/// `World_GetBlock`.
pub fn world_is_loaded() -> bool {
    // SAFETY: World is a valid global.
    unsafe { World.Loaded != 0 }
}
