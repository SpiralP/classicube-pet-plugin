use super::*;

#[test]
fn logger_name_is_stable() {
    let logger = Logger;
    assert_eq!(logger.name(), "Logger");
}
