use classicube_sys::Vec3;

use super::*;

#[test]
fn offset_zero() {
    let base = Vec3::new(1.0, 2.0, 3.0);
    let result = offset_position(base, Vec3::new(0.0, 0.0, 0.0));
    assert_eq!(result.x, 1.0);
    assert_eq!(result.y, 2.0);
    assert_eq!(result.z, 3.0);
}

#[test]
fn offset_positive() {
    let base = Vec3::new(10.0, 64.0, -5.0);
    let result = offset_position(base, Vec3::new(1.0, 0.0, 0.0));
    assert_eq!(result.x, 11.0);
    assert_eq!(result.y, 64.0);
    assert_eq!(result.z, -5.0);
}

#[test]
fn offset_negative() {
    let base = Vec3::new(0.0, 0.0, 0.0);
    let result = offset_position(base, Vec3::new(-3.5, 0.5, 2.0));
    assert_eq!(result.x, -3.5);
    assert_eq!(result.y, 0.5);
    assert_eq!(result.z, 2.0);
}
