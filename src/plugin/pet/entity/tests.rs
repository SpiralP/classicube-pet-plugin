use super::*;

#[test]
fn model_spec_formats_name_and_scale() {
    assert_eq!(model_spec(PET_MODEL), "chicken|0.5");
}

#[test]
fn model_spec_custom_name() {
    assert_eq!(model_spec("pet_dragon"), "pet_dragon|0.5");
}
