use classicube_helpers::async_manager;

use crate::plugin::module::Module;

pub struct RuntimeModule;

impl RuntimeModule {
    pub fn init() -> Self {
        async_manager::initialize();
        Self
    }
}

impl Module for RuntimeModule {
    fn free(&mut self) {
        async_manager::shutdown();
    }
}
