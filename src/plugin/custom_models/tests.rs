use zerocopy::{FromZeros, IntoBytes};

use super::*;

// -- Helpers for building synthetic payloads ------------------------------

fn make_define_payload(id: u8, name: &str, num_parts: u8) -> Vec<u8> {
    let mut pkt = DefineModelPayload::new_zeroed();
    pkt.id = id;
    let name_bytes = name.as_bytes();
    let len = name_bytes.len().min(64);
    pkt.name[..len].copy_from_slice(&name_bytes[..len]);
    pkt.num_parts = num_parts;
    pkt.as_bytes().to_vec()
}

fn make_part_payload(id: u8) -> Vec<u8> {
    let mut pkt = DefineModelPartPayload::new_zeroed();
    pkt.id = id;
    pkt.as_bytes().to_vec()
}

// -- parse_name -----------------------------------------------------------

#[test]
fn parse_name_nul_terminated() {
    let data = make_define_payload(3, "myskin", 1);
    assert_eq!(parse_name(&data), "myskin");
}

#[test]
fn parse_name_space_terminated() {
    let mut pkt = DefineModelPayload::new_zeroed();
    pkt.name[..5].copy_from_slice(b"hello");
    pkt.name[5..].fill(b' ');
    assert_eq!(parse_name(pkt.as_bytes()), "hello");
}

#[test]
fn parse_name_full_64() {
    let name = "x".repeat(64);
    let mut pkt = DefineModelPayload::new_zeroed();
    pkt.name.copy_from_slice(name.as_bytes());
    assert_eq!(parse_name(pkt.as_bytes()), name);
}

#[test]
fn parse_name_too_short_returns_empty() {
    assert_eq!(parse_name(&[0u8; 10]), "");
}

// -- patch_define ---------------------------------------------------------

#[test]
fn patch_define_sets_slot_and_name() {
    let original = make_define_payload(5, "oldname", 3);
    let patched = patch_define(&original, 42, "newname");
    assert_eq!(patched[0], 42, "slot byte");
    assert_eq!(&patched[1..8], b"newname", "name start");
    assert!(
        patched[8..65].iter().all(|&b| b == 0),
        "rest of name field NUL-padded"
    );
    assert_eq!(&patched[65..], &original[65..], "geometry bytes unchanged");
}

#[test]
fn patch_define_name_exactly_64_bytes() {
    let original = make_define_payload(0, "x", 1);
    let name64 = "a".repeat(64);
    let patched = patch_define(&original, 1, &name64);
    assert_eq!(patched[0], 1);
    assert_eq!(&patched[1..65], name64.as_bytes());
}

#[test]
fn patch_define_name_truncated_to_64_bytes() {
    let original = make_define_payload(0, "x", 1);
    let long_name = "b".repeat(80);
    let patched = patch_define(&original, 1, &long_name);
    // Only the first 64 bytes of the name should appear
    assert_eq!(&patched[1..65], &long_name.as_bytes()[..64]);
}

// -- patch_part -----------------------------------------------------------

#[test]
fn patch_part_sets_slot() {
    let original = make_part_payload(5);
    let patched = patch_part(&original, 42);
    assert_eq!(patched[0], 42);
    assert_eq!(&patched[1..], &original[1..]);
}

// -- pick_free_slot -------------------------------------------------------

#[test]
fn pick_free_slot_all_free_returns_highest() {
    let occupied = [false; 64];
    assert_eq!(pick_free_slot(&occupied), Some(63));
}

#[test]
fn pick_free_slot_all_occupied_returns_none() {
    let occupied = [true; 64];
    assert_eq!(pick_free_slot(&occupied), None);
}

#[test]
fn pick_free_slot_returns_highest_free() {
    let mut occupied = [true; 64];
    occupied[10] = false;
    occupied[20] = false;
    assert_eq!(pick_free_slot(&occupied), Some(20));
}

// -- capture state machine ------------------------------------------------

#[test]
fn capture_completes_after_all_parts() {
    let mut s = State::default();
    let define = make_define_payload(5, "mymodel", 2);
    handle_define(&mut s, &define);
    assert!(s.occupied[5], "slot marked occupied after define");
    assert!(s.in_progress.contains_key(&5), "in-progress entry created");

    handle_part(&mut s, &make_part_payload(5));
    assert!(
        s.in_progress.contains_key(&5),
        "still in progress after part 1/2"
    );

    handle_part(&mut s, &make_part_payload(5));
    assert!(
        !s.in_progress.contains_key(&5),
        "in-progress cleared after part 2/2"
    );
    let captured = s.captured.get("mymodel").expect("model in captured map");
    assert_eq!(captured.define, define);
    assert_eq!(captured.parts.len(), 2);
}

#[test]
fn capture_ignores_pet_prefix() {
    let mut s = State::default();
    handle_define(&mut s, &make_define_payload(5, "pet_foo", 1));
    assert!(!s.occupied[5], "pet_ model does not mark slot occupied");
    assert!(!s.in_progress.contains_key(&5));
    assert!(!s.captured.contains_key("pet_foo"));
}

#[test]
fn capture_undefine_clears_slot() {
    let mut s = State::default();
    handle_define(&mut s, &make_define_payload(5, "mymodel", 2));
    assert!(s.occupied[5]);

    handle_undef(&mut s, &[5]);
    assert!(!s.occupied[5]);
    assert!(!s.in_progress.contains_key(&5));
}

#[test]
fn capture_part_for_unknown_slot_is_noop() {
    let mut s = State::default();
    // No DefineModel was seen for slot 9; a stray part should not panic.
    handle_part(&mut s, &make_part_payload(9));
    assert!(s.in_progress.is_empty());
    assert!(s.captured.is_empty());
}

#[test]
fn capture_undefine_empty_payload_is_noop() {
    let mut s = State::default();
    handle_undef(&mut s, &[]);
    // Nothing should have changed.
    assert_eq!(s.occupied, [false; 64]);
}

// -- pet-slot collision guard ---------------------------------------------

#[test]
fn define_on_pet_slot_triggers_revert() {
    let mut s = State::default();
    s.pet_slot = Some(5);
    s.occupied[5] = true;

    let revert = handle_define(&mut s, &make_define_payload(5, "servermodel", 1));
    assert!(revert, "foreign define on pet slot signals a revert");
    assert_eq!(s.pet_slot, None, "pet claim dropped");
    assert!(s.occupied[5], "slot now belongs to the server model");
    assert!(
        s.in_progress.contains_key(&5),
        "server model captured normally"
    );
}

#[test]
fn foreign_pet_named_define_on_pet_slot_triggers_revert() {
    let mut s = State::default();
    s.pet_slot = Some(5);
    s.occupied[5] = true;

    // A server model whose name starts with "pet_" landing on the pet slot is
    // still a foreign collision: the name filter must not suppress the revert.
    let revert = handle_define(&mut s, &make_define_payload(5, "pet_foo", 1));
    assert!(revert, "foreign pet_-named define on pet slot reverts");
    assert_eq!(s.pet_slot, None, "pet claim dropped");
    assert!(
        !s.in_progress.contains_key(&5),
        "pet_-named models are still not captured"
    );
}

#[test]
fn injected_replay_does_not_revert() {
    let mut s = State::default();
    s.pet_slot = Some(5);
    s.occupied[5] = true;
    // The `injecting` flag marks the defines we replay ourselves; they target
    // pet_slot but must not self-trigger the collision revert.
    s.injecting = true;

    let revert = handle_define(&mut s, &make_define_payload(5, "pet_foo", 1));
    assert!(!revert, "our own replayed define does not revert");
    assert_eq!(s.pet_slot, Some(5), "pet claim untouched during injection");
    assert!(
        !s.in_progress.contains_key(&5),
        "injected replay is not captured"
    );
}

#[test]
fn define_on_other_slot_no_revert() {
    let mut s = State::default();
    s.pet_slot = Some(5);

    let revert = handle_define(&mut s, &make_define_payload(7, "servermodel", 1));
    assert!(!revert, "define on a different slot does not revert");
    assert_eq!(s.pet_slot, Some(5), "pet claim untouched");
}

#[test]
fn undef_on_pet_slot_triggers_revert() {
    let mut s = State::default();
    s.pet_slot = Some(5);
    s.occupied[5] = true;

    let revert = handle_undef(&mut s, &[5]);
    assert!(revert, "undefine of pet slot signals a revert");
    assert_eq!(s.pet_slot, None, "pet claim dropped");
    assert!(!s.occupied[5], "slot freed");
}

#[test]
fn undef_on_other_slot_no_revert() {
    let mut s = State::default();
    s.pet_slot = Some(5);

    let revert = handle_undef(&mut s, &[7]);
    assert!(!revert, "undefine of a different slot does not revert");
    assert_eq!(s.pet_slot, Some(5), "pet claim untouched");
}
