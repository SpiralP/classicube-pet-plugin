#[cfg(test)]
mod tests;

use std::{cell::RefCell, io, mem, rc::Weak, time::Duration};

use classicube_helpers::{
    async_manager,
    events::gfx::{ContextLostEventHandler, ContextRecreatedEventHandler},
};
use classicube_sys::{
    BITMAPCOLOR_A_SHIFT, BITMAPCOLOR_B_SHIFT, BITMAPCOLOR_G_SHIFT, BITMAPCOLOR_R_SHIFT, Bitmap,
    BitmapCol, GfxResourceID, OwnedGfxTexture,
};
use png::{ColorType, Decoder, Transformations};
use tracing::{debug, warn};

use super::PET;
use crate::plugin::is_plugin_active;

// Thread-locals holding the pet's owned GPU texture and the pixel data needed
// to recreate it after a graphics context loss. Both live on the main thread.
thread_local! {
    static PET_TEXTURE: RefCell<Option<OwnedGfxTexture>> = const { RefCell::new(None) };
    static PET_SKIN_PIXELS: RefCell<Option<SkinPixels>> = const { RefCell::new(None) };
}

/// Decoded skin data, kept in memory between context-loss and context-recreated
/// so the texture can be rebuilt without re-downloading.
///
/// Only ever stored in `PET_SKIN_PIXELS`, a main-thread `thread_local!`. It is
/// never sent across threads, so it needs no `Send` bound.
struct SkinPixels {
    /// ARGB `BitmapCol` (`0xAARRGGBB`) pixels, row-major, width * height entries.
    pixels: Vec<BitmapCol>,
    width: i32,
    height: i32,
    skin_type: u8,
}

/// Sent as the User-Agent header on skin downloads, e.g.
/// `classicube-pet-plugin/0.2.0`.
const APP_USER_AGENT: &str = concat!(env!("CARGO_PKG_NAME"), "/", env!("CARGO_PKG_VERSION"));

// -- Pure helpers (no FFI -- safe for the test binary to link) ---------------

pub const DEFAULT_SKIN_SERVER: &str = "http://cdn.classicube.net/skin";

/// Return the full URL to fetch a skin PNG.
///
/// If `skin_name` already starts with `http://` or `https://` it is used
/// verbatim; otherwise we build `DEFAULT_SKIN_SERVER/<skin_name>.png`.
pub fn skin_url(skin_name: &str) -> String {
    if skin_name.starts_with("http://") || skin_name.starts_with("https://") {
        skin_name.to_string()
    } else {
        format!("{DEFAULT_SKIN_SERVER}/{skin_name}.png")
    }
}

/// Repack a flat RGBA8 byte slice (4 bytes per pixel) into ClassiCube
/// `BitmapCol` values using the platform's shift constants.
pub fn repack_rgba_to_bitmapcol(rgba: &[u8]) -> Vec<BitmapCol> {
    rgba.chunks_exact(4)
        .map(|px| {
            let [r, g, b, a] = [px[0] as u32, px[1] as u32, px[2] as u32, px[3] as u32];
            (a << BITMAPCOLOR_A_SHIFT)
                | (r << BITMAPCOLOR_R_SHIFT)
                | (g << BITMAPCOLOR_G_SHIFT)
                | (b << BITMAPCOLOR_B_SHIFT)
        })
        .collect()
}

// SKIN_TYPE constants (from ClassiCube Constants.h):
//   SKIN_64x32 = 0, SKIN_64x64 = 1, SKIN_64x64_SLIM = 2, SKIN_INVALID = 0xF0
pub const SKIN_64X32: u8 = 0;
pub const SKIN_64X64: u8 = 1;
pub const SKIN_64X64_SLIM: u8 = 2;
pub const SKIN_INVALID: u8 = 0xF0;

/// Replicate ClassiCube's `Utils_CalcSkinType` in Rust (the C function is not
/// `CC_API` and is not available cross-platform).
///
/// `pixels` must be row-major ARGB `BitmapCol` data (`width * height` entries).
pub fn calc_skin_type(width: i32, height: i32, pixels: &[BitmapCol]) -> u8 {
    // Legacy half-height layout (e.g. 64x32, 128x64).
    if width == height * 2 {
        return SKIN_64X32;
    }
    // Must be square for a 64x64-family skin.
    if width != height {
        return SKIN_INVALID;
    }
    if width == 0 {
        return SKIN_INVALID;
    }
    let scale = width / 64;
    if scale == 0 {
        return SKIN_INVALID;
    }
    // Alex (slim arms) skins have alpha == 0 at pixel (54*scale, 20*scale).
    let x = 54 * scale;
    let y = 20 * scale;
    if (x < width) && (y < height) {
        let idx = (y * width + x) as usize;
        if idx < pixels.len() {
            let alpha = (pixels[idx] >> BITMAPCOLOR_A_SHIFT) & 0xFF;
            if alpha < 128 {
                return SKIN_64X64_SLIM;
            }
        }
    }
    // Further arm-region heuristics from ClassiCube would go here. For our
    // purposes (correct vScale selection) distinguishing 64x64 vs slim is
    // sufficient; we do the alpha check above and default to non-slim.
    SKIN_64X64
}

// -- FFI / main-thread side --------------------------------------------------

/// Kick off an async skin download for `skin_name`. The download runs on a
/// tokio thread; the PNG decode + repack also happen off-thread. When ready,
/// `apply_pixels` is called on the main thread.
///
/// Must be called from the main thread (it accesses `PET` thread-local).
pub fn request_player_skin(skin_name: String) {
    let url = skin_url(&skin_name);
    debug!("fetching pet skin from {url}");

    async_manager::spawn(async move {
        let result = async_manager::timeout(Duration::from_secs(15), async move {
            let client = reqwest::Client::builder()
                .user_agent(APP_USER_AGENT)
                .build()?;
            let bytes = client
                .get(&url)
                .send()
                .await?
                .error_for_status()?
                .bytes()
                .await?;
            decode_skin(&bytes)
        })
        .await;

        match result {
            None => warn!("pet skin download timed out: {skin_name}"),
            Some(Err(e)) => warn!("pet skin download failed for {skin_name}: {e}"),
            Some(Ok(pixels)) => {
                async_manager::spawn_on_main_thread(async move {
                    if !is_plugin_active() {
                        return;
                    }
                    apply_pixels(pixels);
                });
            }
        }
    });
}

/// Decode a PNG byte slice into `SkinPixels`. Runs off the render thread.
fn decode_skin(png_bytes: &[u8]) -> anyhow::Result<SkinPixels> {
    let mut decoder = Decoder::new(io::Cursor::new(png_bytes));
    // Expand palette/low-bit-depth grayscale/tRNS to full channels and strip
    // 16-bit samples to 8-bit, so the decoded buffer is always 8-bit
    // Grayscale / GrayscaleAlpha / Rgb / Rgba (no Indexed, no 16-bit).
    decoder.set_transformations(Transformations::normalize_to_color8());
    let mut reader = decoder.read_info()?;
    let buf_size = reader
        .output_buffer_size()
        .ok_or_else(|| anyhow::anyhow!("unknown PNG output buffer size"))?;
    let mut buf = vec![0u8; buf_size];
    let info = reader.next_frame(&mut buf)?;
    let width = info.width as i32;
    let height = info.height as i32;
    let data = &buf[..info.buffer_size()];

    // Normalise to RGBA8 regardless of source color type.
    let rgba: Vec<u8> = match info.color_type {
        ColorType::Rgba => data.to_vec(),
        ColorType::Rgb => data
            .chunks_exact(3)
            .flat_map(|px| [px[0], px[1], px[2], 255])
            .collect(),
        ColorType::GrayscaleAlpha => data
            .chunks_exact(2)
            .flat_map(|px| [px[0], px[0], px[0], px[1]])
            .collect(),
        ColorType::Grayscale => data.iter().flat_map(|&g| [g, g, g, 255]).collect(),
        ColorType::Indexed => {
            // normalize_to_color8() expands Indexed to Rgb/Rgba, so this is
            // unreachable -- guard rather than silently misrender.
            anyhow::bail!("indexed PNG was not expanded by the decoder")
        }
    };

    let pixels = repack_rgba_to_bitmapcol(&rgba);
    let skin_type = calc_skin_type(width, height, &pixels);
    Ok(SkinPixels {
        pixels,
        width,
        height,
        skin_type,
    })
}

/// Build an `OwnedGfxTexture` from the current `PET_SKIN_PIXELS`.
///
/// Returns `None` if the context is lost (`Gfx_CreateTexture2` returns 0 while
/// `Gfx.LostContext` is true) or if there are no stored pixels.
///
/// Must be called from the main thread.
fn build_texture_from_pixels() -> Option<OwnedGfxTexture> {
    PET_SKIN_PIXELS.with_borrow_mut(|opt| {
        let sp = opt.as_mut()?;
        let mut bmp = Bitmap {
            scan0: sp.pixels.as_mut_ptr(),
            width: sp.width,
            height: sp.height,
        };
        // `managed: true` is harmless -- on GL backends where ManagedTextures is
        // false the flag is ignored, and we handle recreation ourselves anyway.
        OwnedGfxTexture::new(&mut bmp, true, false)
    })
}

/// Store `pixels`, upload a GPU texture, and apply it to the live pet entity.
///
/// Must be called from the main thread.
fn apply_pixels(pixels: SkinPixels) {
    let skin_type = pixels.skin_type;
    PET_SKIN_PIXELS.with_borrow_mut(|slot| *slot = Some(pixels));
    let Some(tex) = build_texture_from_pixels() else {
        return;
    };
    let resource_id = tex.resource_id;
    PET_TEXTURE.with_borrow_mut(|slot| *slot = Some(tex));
    apply_texture_to_entity(resource_id, skin_type);
}

/// Write a GPU texture resource ID and skin type onto the live pet entity.
fn apply_texture_to_entity(resource_id: GfxResourceID, skin_type: u8) {
    let Some(pet) = PET.with_borrow(Weak::upgrade) else {
        return;
    };
    let mut pet = pet.borrow_mut();
    let e = pet.entity.as_mut();
    e.TextureId = resource_id;
    e.SkinType = skin_type;
    e.uScale = 1.0;
    e.vScale = 1.0;
}

/// Put the live pet entity into the "no owned texture" state so
/// `Model_ApplyTexture` falls back to the model's built-in default texture
/// (`tex = 0` -> uses `model->defaultTex`). `SkinType` is left untouched: the
/// engine only reads it when `TextureId != 0`. Does not touch the kept pixel
/// data -- callers decide whether to keep it for recreation.
fn reset_entity_skin() {
    let Some(pet) = PET.with_borrow(Weak::upgrade) else {
        return;
    };
    let mut pet = pet.borrow_mut();
    let e = pet.entity.as_mut();
    e.TextureId = unsafe { mem::zeroed() };
    e.uScale = 1.0;
    e.vScale = 1.0;
}

/// Drop the pet's GPU texture and the kept pixel data, and reset the entity to
/// the model's built-in default texture. Used on free / reset (a full discard,
/// not a context cycle). Must be called from the main thread.
pub fn clear() {
    PET_TEXTURE.with_borrow_mut(|slot| slot.take());
    PET_SKIN_PIXELS.with_borrow_mut(|slot| slot.take());
    reset_entity_skin();
}

/// Subscribe to graphics-context lost/recreated events.
///
/// On loss: drops the GPU texture (engine deletes it on GL backends anyway).
/// On recreated: rebuilds the texture from the kept pixel data, if any.
///
/// Returns the two RAII event handlers; keep them alive (store in `PetModule`)
/// to stay subscribed. Dropping them unsubscribes.
pub fn install_context_handlers() -> (ContextLostEventHandler, ContextRecreatedEventHandler) {
    let mut lost = ContextLostEventHandler::new();
    lost.on(|_| {
        debug!("pet skin: context lost -- dropping texture");
        // Keep PET_SKIN_PIXELS so the recreated handler can rebuild the texture.
        PET_TEXTURE.with_borrow_mut(|slot| slot.take());
        reset_entity_skin();
    });

    let mut recreated = ContextRecreatedEventHandler::new();
    recreated.on(|_| {
        if !is_plugin_active() {
            return;
        }
        debug!("pet skin: context recreated -- rebuilding texture");
        let Some(tex) = build_texture_from_pixels() else {
            return;
        };
        let resource_id = tex.resource_id;
        let skin_type = PET_SKIN_PIXELS
            .with_borrow(|opt| opt.as_ref().map(|sp| sp.skin_type))
            .unwrap_or(SKIN_64X64);
        PET_TEXTURE.with_borrow_mut(|slot| *slot = Some(tex));
        apply_texture_to_entity(resource_id, skin_type);
    });

    (lost, recreated)
}
