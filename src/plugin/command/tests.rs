use super::*;

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
