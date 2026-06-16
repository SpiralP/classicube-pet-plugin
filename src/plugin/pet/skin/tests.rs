use classicube_sys::{BITMAPCOLOR_A_SHIFT, BITMAPCOLOR_G_SHIFT, BITMAPCOLOR_R_SHIFT};

use super::*;

#[test]
fn skin_url_bare_name() {
    assert_eq!(
        skin_url("SpiralP"),
        "http://cdn.classicube.net/skin/SpiralP.png"
    );
}

#[test]
fn skin_url_http_passthrough() {
    assert_eq!(
        skin_url("http://example.com/skin.png"),
        "http://example.com/skin.png"
    );
}

#[test]
fn skin_url_https_passthrough() {
    assert_eq!(
        skin_url("https://example.com/skin.png"),
        "https://example.com/skin.png"
    );
}

#[test]
fn repack_rgba_to_bitmapcol_channels() {
    // RGBA (255, 0, 0, 128): r in R slot, a in A slot
    let rgba = [255u8, 0, 0, 128];
    let expected = (128_u32 << BITMAPCOLOR_A_SHIFT) | (255_u32 << BITMAPCOLOR_R_SHIFT);
    let result = repack_rgba_to_bitmapcol(&rgba);
    assert_eq!(result, vec![expected]);
}

#[test]
fn repack_rgba_to_bitmapcol_green_alpha() {
    // RGBA (0, 200, 0, 255): g in G slot, a in A slot
    let rgba = [0u8, 200, 0, 255];
    let expected = (255_u32 << BITMAPCOLOR_A_SHIFT) | (200_u32 << BITMAPCOLOR_G_SHIFT);
    let result = repack_rgba_to_bitmapcol(&rgba);
    assert_eq!(result, vec![expected]);
}

#[test]
fn calc_skin_type_legacy_64x32() {
    // width == height * 2 -> SKIN_64x32
    assert_eq!(calc_skin_type(64, 32, &[0u32; 64 * 32]), SKIN_64X32);
    assert_eq!(calc_skin_type(128, 64, &[0u32; 128 * 64]), SKIN_64X32);
}

#[test]
fn calc_skin_type_non_square_invalid() {
    // width != height and not 2:1 -> SKIN_INVALID
    assert_eq!(calc_skin_type(100, 64, &[0u32; 100 * 64]), SKIN_INVALID);
}

#[test]
fn calc_skin_type_64x64_opaque_alex_pixel() {
    // 64x64 with opaque alpha at (54,20) -> not slim -> SKIN_64x64
    let opaque = 0xFF_u32 << BITMAPCOLOR_A_SHIFT;
    let mut pixels = vec![opaque; 64 * 64];
    let idx = 20 * 64 + 54;
    pixels[idx] = opaque; // alpha = 255 -> not slim
    assert_eq!(calc_skin_type(64, 64, &pixels), SKIN_64X64);
}

#[test]
fn calc_skin_type_64x64_slim_transparent_alex_pixel() {
    // 64x64 with transparent alpha at (54,20) -> slim -> SKIN_64x64_SLIM
    let opaque = 0xFF_u32 << BITMAPCOLOR_A_SHIFT;
    let mut pixels = vec![opaque; 64 * 64];
    let idx = 20 * 64 + 54;
    pixels[idx] = 0x00000000; // alpha = 0 -> slim
    assert_eq!(calc_skin_type(64, 64, &pixels), SKIN_64X64_SLIM);
}
