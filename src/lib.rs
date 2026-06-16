mod plugin;

use std::{os::raw::c_int, ptr};

use classicube_sys::IGameComponent;

extern "C" fn init() {
    plugin::initialize();
}

extern "C" fn free() {
    plugin::free();
}

extern "C" fn reset() {
    plugin::reset();
}

extern "C" fn on_new_map() {
    plugin::on_new_map();
}

extern "C" fn on_new_map_loaded() {
    plugin::on_new_map_loaded();
}

#[allow(non_upper_case_globals)]
#[unsafe(no_mangle)]
pub static Plugin_ApiVersion: c_int = 1;

#[allow(non_upper_case_globals)]
#[unsafe(no_mangle)]
pub static mut Plugin_Component: IGameComponent = IGameComponent {
    Init: Some(init),
    Free: Some(free),
    Reset: Some(reset),
    OnNewMap: Some(on_new_map),
    OnNewMapLoaded: Some(on_new_map_loaded),
    next: ptr::null_mut(),
};
