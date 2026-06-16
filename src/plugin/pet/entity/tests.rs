use super::*;

#[test]
fn model_spec_formats_name_and_scale() {
    assert_eq!(model_spec(), "chicken|0.5");
}
