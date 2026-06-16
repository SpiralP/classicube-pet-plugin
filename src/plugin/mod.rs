pub mod command;
pub mod logger;
pub mod module;
pub mod pet;
pub mod relay;

use std::cell::RefCell;

use crate::plugin::{
    command::CommandModule, logger::LoggerModule, module::Module, pet::PetModule,
    relay::RelayModule,
};

thread_local!(
    static MAIN_MODULE: RefCell<Option<MainModule>> = const { RefCell::new(None) };
);

struct MainModule {
    logger: LoggerModule,
    command: CommandModule,
    relay: RelayModule,
    pet: PetModule,
}

impl MainModule {
    fn init() -> Self {
        let logger = LoggerModule::init();
        let command = CommandModule::init();
        let relay = RelayModule::init();
        let pet = PetModule::init();
        Self {
            logger,
            command,
            relay,
            pet,
        }
    }
}

impl Module for MainModule {
    fn children(&mut self) -> Vec<&mut dyn Module> {
        vec![
            &mut self.logger,
            &mut self.command,
            &mut self.relay,
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
