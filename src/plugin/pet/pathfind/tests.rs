use std::collections::{HashMap, HashSet, VecDeque};

use classicube_sys::Vec3;

use super::*;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

struct FakeGrid {
    width: i32,
    height: i32,
    length: i32,
    solid: HashSet<(i32, i32, i32)>,
    /// Optional per-cell top-surface override (cell-local 0..1 fraction added
    /// to cell Y). Simulates slabs (0.5), snow (0.25), etc. without FFI.
    heights: HashMap<(i32, i32, i32), f32>,
}

impl FakeGrid {
    fn new(width: i32, height: i32, length: i32) -> Self {
        Self {
            width,
            height,
            length,
            solid: HashSet::new(),
            heights: HashMap::new(),
        }
    }

    fn set_solid(&mut self, x: i32, y: i32, z: i32) {
        self.solid.insert((x, y, z));
    }

    /// Override the top-surface height for a cell. `top` is the cell-local
    /// fraction (0..1); the world-Y surface = `y + top`.
    fn set_surface(&mut self, x: i32, y: i32, z: i32, top: f32) {
        self.heights.insert((x, y, z), top);
    }

    /// Fill an entire horizontal layer at `y` with solid blocks.
    fn fill_floor(&mut self, y: i32) {
        for x in 0..self.width {
            for z in 0..self.length {
                self.set_solid(x, y, z);
            }
        }
    }
}

impl Grid for FakeGrid {
    fn is_solid(&self, x: i32, y: i32, z: i32) -> bool {
        if x < 0 || y < 0 || z < 0 || x >= self.width || y >= self.height || z >= self.length {
            return false;
        }
        self.solid.contains(&(x, y, z))
    }

    fn surface_top(&self, x: i32, y: i32, z: i32) -> f32 {
        let frac = self.heights.get(&(x, y, z)).copied().unwrap_or(1.0);
        y as f32 + frac
    }
}

fn v(x: f32, y: f32, z: f32) -> Vec3 {
    Vec3 { x, y, z }
}

// ---------------------------------------------------------------------------
// can_stand
// ---------------------------------------------------------------------------

#[test]
fn can_stand_solid_ground_open_above() {
    let mut g = FakeGrid::new(10, 10, 10);
    g.set_solid(5, 2, 5); // ground
    // Cell (5,3,5): ground at y=2 (solid), body y=3 (air), headroom y=4 (air).
    assert!(can_stand(&g, Cell { x: 5, y: 3, z: 5 }));
}

#[test]
fn cant_stand_no_ground() {
    let g = FakeGrid::new(10, 10, 10);
    assert!(!can_stand(&g, Cell { x: 5, y: 3, z: 5 }));
}

#[test]
fn cant_stand_ceiling_blocks_headroom() {
    let mut g = FakeGrid::new(10, 10, 10);
    g.set_solid(5, 2, 5); // ground
    g.set_solid(5, 4, 5); // ceiling -- blocks headroom cell y+1=4 for feet y=3
    assert!(!can_stand(&g, Cell { x: 5, y: 3, z: 5 }));
}

#[test]
fn cant_stand_body_blocked() {
    let mut g = FakeGrid::new(10, 10, 10);
    g.set_solid(5, 2, 5); // ground
    g.set_solid(5, 3, 5); // solid in the body cell -- not passable
    assert!(!can_stand(&g, Cell { x: 5, y: 3, z: 5 }));
}

// ---------------------------------------------------------------------------
// find_path
// ---------------------------------------------------------------------------

#[test]
fn find_path_straight_line() {
    let mut g = FakeGrid::new(10, 5, 10);
    g.fill_floor(0);
    // Feet at y=1 along z=5, walking in +x direction.
    let path = find_path(
        &g,
        Cell { x: 0, y: 1, z: 5 },
        Cell { x: 5, y: 1, z: 5 },
        MAX_EXPLORED_NODES,
    );
    let path = path.expect("expected a path");
    assert_eq!(path.first(), Some(&Cell { x: 0, y: 1, z: 5 }));
    assert_eq!(path.last(), Some(&Cell { x: 5, y: 1, z: 5 }));
    assert_eq!(path.len(), 6); // Manhattan dist 5, 6 cells total
    // All cells should be standable.
    for c in &path {
        assert!(can_stand(&g, *c), "{c:?} should be standable");
    }
}

#[test]
fn find_path_detours_around_wall() {
    let mut g = FakeGrid::new(10, 6, 10);
    g.fill_floor(0);
    // Wall from z=0 to z=8 at x=3 (all heights, effectively blocking direct route).
    for y in 0..6 {
        for z in 0..9 {
            g.set_solid(3, y, z);
        }
    }
    // Gap at z=9 lets the path go around.
    let path = find_path(
        &g,
        Cell { x: 0, y: 1, z: 5 },
        Cell { x: 5, y: 1, z: 5 },
        MAX_EXPLORED_NODES,
    );
    let path = path.expect("expected a path around the wall");
    assert_eq!(path.first(), Some(&Cell { x: 0, y: 1, z: 5 }));
    assert_eq!(path.last(), Some(&Cell { x: 5, y: 1, z: 5 }));
    // Path must be longer than the direct Manhattan distance (5).
    assert!(path.len() > 6, "path should detour around the wall");
    // All cells standable.
    for c in &path {
        assert!(can_stand(&g, *c), "{c:?} should be standable");
    }
}

#[test]
fn find_path_stairs_up() {
    let mut g = FakeGrid::new(10, 10, 5);
    // Staircase: block at (x, x, 0) forms rising ground.
    // Pet feet at y=x+1 for column x.
    for x in 0..6 {
        g.set_solid(x, x, 2);
    }
    let path = find_path(
        &g,
        Cell { x: 0, y: 1, z: 2 },
        Cell { x: 5, y: 6, z: 2 },
        MAX_EXPLORED_NODES,
    );
    let path = path.expect("expected a climbing path");
    assert_eq!(path.first(), Some(&Cell { x: 0, y: 1, z: 2 }));
    assert_eq!(path.last(), Some(&Cell { x: 5, y: 6, z: 2 }));
    // Each step moves x by +1 and y by +1 (step-up).
    for w in path.windows(2) {
        let dy = w[1].y - w[0].y;
        assert!(dy <= 1, "step should be at most +1 cell up, got dy={dy}");
    }
    for c in &path {
        assert!(can_stand(&g, *c), "{c:?} should be standable");
    }
}

#[test]
fn find_path_fall_within_max_fall() {
    let mut g = FakeGrid::new(10, 10, 5);
    // Ledge at x=0 (ground block at y=3), then ground at y=0 for x=1..5.
    g.set_solid(0, 3, 2); // ledge
    for x in 1..6 {
        g.set_solid(x, 0, 2);
    }
    // Start on ledge, goal 3 cells lower on the ground.
    let path = find_path(
        &g,
        Cell { x: 0, y: 4, z: 2 },
        Cell { x: 5, y: 1, z: 2 },
        MAX_EXPLORED_NODES,
    );
    let path = path.expect("expected a path that includes a fall");
    assert_eq!(path.first(), Some(&Cell { x: 0, y: 4, z: 2 }));
    assert_eq!(path.last(), Some(&Cell { x: 5, y: 1, z: 2 }));
    // The path must include at least one downward step (fall off ledge).
    assert!(
        path.windows(2).any(|w| w[1].y < w[0].y),
        "path should include at least one downward step"
    );
}

#[test]
fn find_path_drop_beyond_max_fall_returns_none() {
    let mut g = FakeGrid::new(10, 10, 5);
    // Ledge at x=0, ground is 4 cells lower (> MAX_FALL=3).
    g.set_solid(0, 4, 2); // ledge: feet at y=5
    for x in 1..5 {
        g.set_solid(x, 0, 2); // ground: feet at y=1 (4 cells below y=5)
    }
    // No adjacent ground within MAX_FALL from x=0, no alternate route.
    let path = find_path(
        &g,
        Cell { x: 0, y: 5, z: 2 },
        Cell { x: 4, y: 1, z: 2 },
        MAX_EXPLORED_NODES,
    );
    assert!(
        path.is_none(),
        "drop of 4 cells exceeds MAX_FALL=3, should be None"
    );
}

#[test]
fn find_path_unreachable_walled_off() {
    let mut g = FakeGrid::new(10, 5, 10);
    g.fill_floor(0);
    // Surround the goal with a complete solid wall.
    for y in 0..5 {
        g.set_solid(7, y, 5);
        g.set_solid(9, y, 5);
        g.set_solid(8, y, 4);
        g.set_solid(8, y, 6);
    }
    // Make the ceiling solid too.
    g.set_solid(8, 1, 5);
    g.set_solid(8, 2, 5);
    let path = find_path(
        &g,
        Cell { x: 0, y: 1, z: 5 },
        Cell { x: 8, y: 1, z: 5 },
        MAX_EXPLORED_NODES,
    );
    assert!(path.is_none());
}

#[test]
fn find_path_node_cap_returns_none() {
    // Large open grid: the path is a long straight line that A* can find, but
    // we cap exploration at 1 node so the search terminates with None.
    let mut g = FakeGrid::new(100, 5, 100);
    g.fill_floor(0);
    let path = find_path(
        &g,
        Cell { x: 0, y: 1, z: 50 },
        Cell { x: 90, y: 1, z: 50 },
        1, // only 1 node allowed
    );
    assert!(path.is_none(), "search should terminate via node cap");
}

#[test]
fn find_path_already_at_goal() {
    let mut g = FakeGrid::new(10, 5, 10);
    g.fill_floor(0);
    let path = find_path(
        &g,
        Cell { x: 5, y: 1, z: 5 },
        Cell { x: 5, y: 1, z: 5 },
        MAX_EXPLORED_NODES,
    );
    let path = path.expect("trivial same-cell path");
    assert_eq!(path.len(), 1);
    assert_eq!(path[0], Cell { x: 5, y: 1, z: 5 });
}

#[test]
fn find_path_snaps_start_to_ground() {
    let mut g = FakeGrid::new(10, 5, 10);
    g.fill_floor(0);
    // Start cell is in mid-air (y=3, but ground is at y=0 so feet-y=1).
    let path = find_path(
        &g,
        Cell { x: 0, y: 3, z: 5 }, // mid-air -- should snap to y=1
        Cell { x: 5, y: 1, z: 5 },
        MAX_EXPLORED_NODES,
    );
    let path = path.expect("expected path after snapping start to ground");
    assert_eq!(path.first(), Some(&Cell { x: 0, y: 1, z: 5 }));
}

// ---------------------------------------------------------------------------
// cell_successors
// ---------------------------------------------------------------------------

#[test]
fn cell_successors_flat_floor_four_neighbors() {
    let mut g = FakeGrid::new(10, 5, 10);
    g.fill_floor(0);
    let neighbors = cell_successors(&g, Cell { x: 5, y: 1, z: 5 });
    // All four orthogonal flat neighbors should be returned.
    assert_eq!(neighbors.len(), 4);
    for (c, _) in &neighbors {
        assert!(can_stand(&g, *c), "{c:?} should be standable");
    }
}

#[test]
fn cell_successors_step_up_blocked_by_source_ceiling() {
    let mut g = FakeGrid::new(10, 5, 10);
    g.fill_floor(0);
    // Floor at y=1 in the +x direction (one cell higher than the source).
    g.set_solid(6, 1, 5); // raised ground block -- target feet at y=2
    // Low ceiling at source: blocks headroom at y+2=3 for the pet at y=1.
    g.set_solid(5, 3, 5);
    let neighbors = cell_successors(&g, Cell { x: 5, y: 1, z: 5 });
    // The step-up to (6,2,5) should be absent due to ceiling at source.
    assert!(
        !neighbors
            .iter()
            .any(|(c, _)| *c == Cell { x: 6, y: 2, z: 5 }),
        "step-up should be blocked by ceiling at source"
    );
}

// ---------------------------------------------------------------------------
// step_walk
// ---------------------------------------------------------------------------

#[test]
fn step_walk_advances_toward_waypoint_without_overshoot() {
    let target = v(5.0, 0.0, 0.0);
    let mut path = VecDeque::from([target]);
    // delta=0.1: budget = min(WALK_SPEED*0.1, MAX_STEP) = min(0.1, 0.9) = 0.1
    // dist = 5.0 > 0.1 -> partial move
    let result = step_walk(v(0.0, 0.0, 0.0), 0.0, 0.0, 0.0, &mut path, 0.1);
    assert!(
        (result.position.x - 0.1).abs() < 1e-4,
        "expected ~0.1 advance, got {}",
        result.position.x
    );
    assert_eq!(result.position.y, 0.0);
    assert!(!path.is_empty(), "waypoint should not be popped yet");
}

#[test]
fn step_walk_snaps_and_pops_on_arrival() {
    let target = v(0.2, 0.0, 0.0);
    let mut path = VecDeque::from([target]);
    // budget = min(WALK_SPEED*1.0, MAX_STEP) = min(3.0, 0.9) = 0.9
    // dist = 0.2 <= 0.9 -> snap to target
    let result = step_walk(v(0.0, 0.0, 0.0), 0.0, 0.0, 0.0, &mut path, 1.0);
    assert!(
        (result.position.x - 0.2).abs() < 1e-6,
        "should snap to waypoint"
    );
    assert!(path.is_empty(), "waypoint should be popped after arrival");
}

#[test]
fn step_walk_consumes_multiple_short_waypoints_in_one_frame() {
    // Two waypoints each 0.1 blocks away; budget = 0.9 covers both.
    let mut path = VecDeque::from([v(0.1, 0.0, 0.0), v(0.2, 0.0, 0.0)]);
    let result = step_walk(v(0.0, 0.0, 0.0), 0.0, 0.0, 0.0, &mut path, 1.0);
    // Both waypoints should be consumed; final position at x=0.2.
    assert!(path.is_empty(), "both waypoints should be consumed");
    assert!((result.position.x - 0.2).abs() < 1e-5);
}

#[test]
fn step_walk_caps_huge_delta() {
    let target = v(100.0, 0.0, 0.0);
    let mut path = VecDeque::from([target]);
    // delta=100.0: budget = min(300.0, MAX_STEP_PER_FRAME=0.9) = 0.9
    let result = step_walk(v(0.0, 0.0, 0.0), 0.0, 0.0, 0.0, &mut path, 100.0);
    assert!(
        result.position.x <= MAX_STEP_PER_FRAME + 1e-5,
        "huge delta should be capped; got {}",
        result.position.x
    );
    assert!(
        !path.is_empty(),
        "waypoint should not be popped (too far away)"
    );
}

#[test]
fn step_walk_empty_path_returns_position_and_angles_unchanged() {
    let pos = v(1.0, 2.0, 3.0);
    let mut path = VecDeque::new();
    let result = step_walk(pos, 10.0, 42.0, 7.0, &mut path, 1.0);
    assert_eq!(result.position.x, 1.0);
    assert_eq!(result.position.y, 2.0);
    assert_eq!(result.position.z, 3.0);
    assert_eq!(result.head_pitch, 10.0);
    assert_eq!(result.head_yaw, 42.0);
    assert_eq!(result.body_yaw, 7.0);
}

#[test]
fn step_walk_body_faces_travel_when_moving_in_positive_x() {
    // Moving +x: dx=1, dz=0. atan2(1, -0) = 90 degrees. Per Vec3_GetDirVector,
    // yaw 90 faces +x, so +x travel is correctly aimed right.
    let target = v(5.0, 0.0, 0.0);
    let mut path = VecDeque::from([target]);
    let result = step_walk(v(0.0, 0.0, 0.0), 0.0, 0.0, 0.0, &mut path, 0.1);
    assert!(
        (result.body_yaw - 90.0_f32).abs() < 1e-3,
        "moving +x should give body_yaw ~90, got {}",
        result.body_yaw
    );
}

#[test]
fn step_walk_body_faces_travel_when_moving_in_positive_z() {
    // Moving +z: dx=0, dz=1. atan2(0, -1) = 180 degrees. Per Vec3_GetDirVector,
    // yaw 180 faces +z (forward), not yaw 0 which faces -z (backward).
    let target = v(0.0, 0.0, 5.0);
    let mut path = VecDeque::from([target]);
    let result = step_walk(v(0.0, 0.0, 0.0), 0.0, 45.0, 45.0, &mut path, 0.1);
    assert!(
        (result.body_yaw.abs() - 180.0_f32).abs() < 1e-3,
        "moving +z should give body_yaw ~180, got {}",
        result.body_yaw
    );
}

#[test]
fn step_walk_keeps_body_yaw_unchanged_for_purely_vertical_move() {
    // Waypoint is directly above; no horizontal movement.
    // Since step_walk is now horizontal-only, horiz==0 triggers the zero-
    // horizontal pop path (popped immediately, budget unchanged) rather than a
    // Y move -- but body_yaw is still not touched, so the assertion holds.
    let target = v(0.0, 5.0, 0.0);
    let mut path = VecDeque::from([target]);
    let result = step_walk(v(0.0, 0.0, 0.0), 0.0, 0.0, 123.0, &mut path, 0.01);
    // No horizontal delta => body_yaw unchanged. The goal is directly overhead,
    // so the head angles fall back to the passed-in current values too.
    assert!(
        (result.body_yaw - 123.0_f32).abs() < 1e-3,
        "body_yaw should stay 123 when moving purely vertically, got {}",
        result.body_yaw
    );
}

#[test]
fn step_walk_head_aims_at_goal_while_body_faces_next_waypoint() {
    // Travel toward +z (first waypoint) while the goal sits off to the +x side.
    // Body should face the travel direction (+z, yaw 180); head should aim at
    // the far goal (+x, yaw ~90), proving the two rotations are decoupled.
    let next = v(0.0, 0.0, 1.0);
    let goal = v(100.0, 0.0, 0.0);
    let mut path = VecDeque::from([next, goal]);
    // Tiny delta: the pet barely advances along +z, so the head's view of the
    // far goal stays essentially due +x.
    let result = step_walk(v(0.0, 0.0, 0.0), 0.0, 0.0, 0.0, &mut path, 0.01);
    assert!(
        (result.body_yaw.abs() - 180.0_f32).abs() < 1e-3,
        "body should face travel (+z, ~180), got {}",
        result.body_yaw
    );
    assert!(
        (result.head_yaw - 90.0_f32).abs() < 0.5,
        "head should aim at the +x goal (~90), got {}",
        result.head_yaw
    );
}

#[test]
fn step_walk_head_pitches_toward_a_lower_goal() {
    // Goal is below and to the +x side; horizontal dist 3, drop of 3 => the head
    // should pitch ~45 degrees down (positive pitch looks down).
    let goal = v(3.0, -3.0, 0.0);
    let mut path = VecDeque::from([goal]);
    // Tiny delta so the pet stays ~at the origin and the geometry is clean.
    let result = step_walk(v(0.0, 0.0, 0.0), 0.0, 0.0, 0.0, &mut path, 0.0001);
    assert!(
        (result.head_pitch - 45.0_f32).abs() < 0.5,
        "head should pitch ~45 down toward the lower goal, got {}",
        result.head_pitch
    );
}

// ---------------------------------------------------------------------------
// ground_surface_y
// ---------------------------------------------------------------------------

/// Sample the column containing cell (x, z): point footprint at cell center.
fn ground_at<G: Grid>(g: &G, x: i32, z: i32, feet_y: f32) -> Option<f32> {
    let (cx, cz) = (x as f32 + 0.5, z as f32 + 0.5);
    g.ground_surface_y(cx, cx, cz, cz, feet_y)
}

#[test]
fn ground_surface_y_flat_full_block() {
    let mut g = FakeGrid::new(10, 5, 10);
    g.set_solid(5, 0, 5);
    // feet at 1.0 (standing on block top=1.0); should return 1.0.
    assert_eq!(ground_at(&g, 5, 5, 1.0), Some(1.0));
}

#[test]
fn ground_surface_y_none_over_void() {
    let g = FakeGrid::new(10, 5, 10);
    assert_eq!(ground_at(&g, 5, 5, 1.0), None);
}

#[test]
fn ground_surface_y_step_up_one() {
    // Support block at cell y=1 (top surface = 2.0); pet feet currently at 1.0.
    // GROUND_STEP_TOLERANCE=0.5: hi = floor(1.5) = 1, which reaches cell 1.
    let mut g = FakeGrid::new(10, 5, 10);
    g.set_solid(6, 1, 6);
    assert_eq!(ground_at(&g, 6, 6, 1.0), Some(2.0));
}

#[test]
fn ground_surface_y_step_down_one() {
    // Support block at cell y=0 (top=1.0); pet feet currently at 2.0.
    let mut g = FakeGrid::new(10, 5, 10);
    g.set_solid(6, 0, 6);
    assert_eq!(ground_at(&g, 6, 6, 2.0), Some(1.0));
}

#[test]
fn ground_surface_y_step_down_max_fall_boundary_inclusive() {
    // Support block at exactly floor(feet) - MAX_FALL - 1 (deepest cell in window).
    // feet=4.0 -> lo = 4 - 3 - 1 = 0; solid at y=0 must be found.
    let mut g = FakeGrid::new(10, 10, 10);
    g.set_solid(5, 0, 5);
    assert_eq!(
        ground_at(&g, 5, 5, 4.0),
        Some(1.0),
        "solid at lo boundary should be found"
    );
}

#[test]
fn ground_surface_y_step_down_beyond_max_fall_returns_none() {
    // Support is one cell deeper than the window; should return None.
    // feet=4.0 -> lo=0; solid only at y=-1 (out of FakeGrid bounds -> not solid).
    let g = FakeGrid::new(10, 10, 10);
    assert_eq!(
        ground_at(&g, 5, 5, 4.0),
        None,
        "nothing within window should yield None"
    );
}

#[test]
fn ground_surface_y_slab_half_height() {
    // Slab: solid block with a custom surface_top of 0.5 (half-height).
    // Pet was floating at feet=1.0; should now land at 0.5.
    let mut g = FakeGrid::new(10, 5, 10);
    g.set_solid(5, 0, 5);
    g.set_surface(5, 0, 5, 0.5);
    assert_eq!(ground_at(&g, 5, 5, 1.0), Some(0.5));
}

#[test]
fn ground_surface_y_returns_highest_when_stacked() {
    // Two solid cells stacked; from feet=2.0 the search returns the highest one.
    let mut g = FakeGrid::new(10, 5, 10);
    g.set_solid(5, 0, 5);
    g.set_solid(5, 1, 5); // top surface = 2.0
    // hi = floor(2.5) = 2; search 2..=0 rev => cell 2 (air), cell 1 (solid) first.
    assert_eq!(ground_at(&g, 5, 5, 2.0), Some(2.0));
}

#[test]
fn ground_surface_y_does_not_grab_ceiling() {
    // Ceiling block at y=2 must not be returned as the ground.
    let mut g = FakeGrid::new(10, 5, 10);
    g.set_solid(5, 0, 5); // ground, top=1.0
    g.set_solid(5, 2, 5); // ceiling above the headroom cell
    // feet=1.0 -> hi=floor(1.5)=1; cell 1 is air, cell 0 is the ground.
    assert_eq!(ground_at(&g, 5, 5, 1.0), Some(1.0));
}

#[test]
fn ground_surface_y_fractional_feet() {
    // feet=0.999 (slightly below 1.0) with full-block ground at y=0 -> Some(1.0).
    // Confirms no off-by-one around sub-integer feet values.
    let mut g = FakeGrid::new(10, 5, 10);
    g.set_solid(5, 0, 5);
    assert_eq!(ground_at(&g, 5, 5, 0.999), Some(1.0));
}

#[test]
fn ground_surface_y_footprint_steps_up_when_edge_reaches_higher_block() {
    // Low block at (5,0,5) surface=1.0; step block at (6,1,5) surface=2.0.
    // A footprint spanning both cells returns the higher surface; one confined
    // to the low cell returns the low surface.
    let mut g = FakeGrid::new(10, 5, 10);
    g.set_solid(5, 0, 5);
    g.set_solid(6, 1, 5);
    assert_eq!(
        g.ground_surface_y(5.9, 6.1, 5.4, 5.6, 1.0),
        Some(2.0),
        "footprint over the step edge should snap up"
    );
    assert_eq!(
        g.ground_surface_y(5.4, 5.6, 5.4, 5.6, 1.0),
        Some(1.0),
        "footprint confined to the low cell should stay low"
    );
}

#[test]
fn ground_surface_y_footprint_stays_on_ledge_until_clear() {
    // High block at (5,1,5) surface=2.0; low block at (6,0,5) surface=1.0.
    // While the footprint still overlaps the high cell, the max holds at 2.0.
    // Once the footprint fully clears it, the max drops to 1.0.
    let mut g = FakeGrid::new(10, 5, 10);
    g.set_solid(5, 1, 5);
    g.set_solid(6, 0, 5);
    assert_eq!(
        g.ground_surface_y(5.4, 6.1, 5.4, 5.6, 2.0),
        Some(2.0),
        "still overlapping the high cell -> stay on the ledge"
    );
    assert_eq!(
        g.ground_surface_y(6.0, 6.4, 5.4, 5.6, 2.0),
        Some(1.0),
        "footprint fully cleared the high cell -> drop to low"
    );
}
