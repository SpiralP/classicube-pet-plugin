pub mod command;
pub mod custom_models;
pub mod logger;
pub mod module;
pub mod pet;
pub mod relay;
pub mod runtime;

use std::cell::RefCell;

use crate::plugin::{
    command::CommandModule, custom_models::CustomModelsModule, logger::LoggerModule,
    module::Module, pet::PetModule, relay::RelayModule, runtime::RuntimeModule,
};

thread_local!(
    static MAIN_MODULE: RefCell<Option<MainModule>> = const { RefCell::new(None) };
);

struct MainModule {
    runtime: RuntimeModule,
    logger: LoggerModule,
    command: CommandModule,
    relay: RelayModule,
    custom_models: CustomModelsModule,
    pet: PetModule,
}

impl MainModule {
    fn init() -> Self {
        // RuntimeModule MUST be constructed first: async_manager::initialize()
        // must run before any module that spawns async work (PetModule's skin
        // download in set_pet_model). It is placed first in children() too, so
        // it is torn down last (children free in reverse order).
        let runtime = RuntimeModule::init();
        let logger = LoggerModule::init();
        let command = CommandModule::init();
        let relay = RelayModule::init();
        let custom_models = CustomModelsModule::init();
        let pet = PetModule::init();
        Self {
            runtime,
            logger,
            command,
            relay,
            custom_models,
            pet,
        }
    }
}

impl Module for MainModule {
    fn children(&mut self) -> Vec<&mut dyn Module> {
        vec![
            &mut self.runtime,
            &mut self.logger,
            &mut self.command,
            &mut self.relay,
            &mut self.custom_models,
            &mut self.pet,
        ]
    }
}

pub fn is_plugin_active() -> bool {
    MAIN_MODULE.with_borrow(|m| m.is_some())
}

pub fn initialize() {
    MAIN_MODULE.with_borrow_mut(|main_module| {
        if main_module.is_none() {
            *main_module = Some(MainModule::init());
        }
    });
}

pub fn free() {
    MAIN_MODULE.with_borrow_mut(|main_module| {
        if let Some(mut main_module) = main_module.take() {
            main_module.handle_free();
        }
    });
}

pub fn reset() {
    MAIN_MODULE.with_borrow_mut(|main_module| {
        if let Some(main_module) = main_module.as_mut() {
            main_module.handle_reset();
        }
    });
}

pub fn on_new_map() {
    MAIN_MODULE.with_borrow_mut(|main_module| {
        if let Some(main_module) = main_module.as_mut() {
            main_module.handle_on_new_map();
        }
    });
}

pub fn on_new_map_loaded() {
    MAIN_MODULE.with_borrow_mut(|main_module| {
        if let Some(main_module) = main_module.as_mut() {
            main_module.handle_on_new_map_loaded();
        }
    });
}
