use super::*;

// -- pick_match ---------------------------------------------------------------

#[test]
fn pick_match_empty() {
    assert_eq!(pick_match(&[]), None);
}

#[test]
fn pick_match_single_renderable() {
    assert_eq!(pick_match(&[(7, false)]), Some(7));
}

#[test]
fn pick_match_single_invisible() {
    assert_eq!(pick_match(&[(3, true)]), Some(3));
}

#[test]
fn pick_match_prefers_renderable_when_first() {
    assert_eq!(pick_match(&[(1, false), (2, true)]), Some(1));
}

#[test]
fn pick_match_prefers_renderable_when_second() {
    assert_eq!(pick_match(&[(1, true), (2, false)]), Some(2));
}

#[test]
fn pick_match_all_invisible_returns_first() {
    assert_eq!(pick_match(&[(5, true), (9, true)]), Some(5));
}

#[test]
fn name_matches_color_code_candidate() {
    assert!(name_matches("&cSpiral&fP", "spiralp"));
}

#[test]
fn name_matches_color_code_query() {
    // color codes in the query are also stripped before matching
    assert!(name_matches("SpiralP", "&cspiral&fp"));
}

#[test]
fn name_matches_case_insensitive_substring() {
    assert!(name_matches("BotAlice", "alice"));
    assert!(name_matches("BotAlice", "ALICE"));
    assert!(name_matches("BotAlice", "Bot"));
}

#[test]
fn name_matches_non_match() {
    assert!(!name_matches("Bob", "alice"));
}

#[test]
fn name_matches_empty_query_matches_all() {
    // Empty query always matches (execute handles the empty-arg case before
    // calling find_entity_by_name, but the matcher itself should be consistent).
    assert!(name_matches("Anyone", ""));
}
