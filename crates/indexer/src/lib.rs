#![allow(dead_code, unused_imports)]

pub mod db;
pub mod geyser_plugin;

pub use geyser_plugin::SonarGeyserPlugin;

use clone_agave_geyser_plugin_interface::geyser_plugin_interface::GeyserPlugin;

#[no_mangle]
#[allow(improper_ctypes_definitions)]
/// # Safety
/// The caller becomes responsible for the returned raw pointer and must treat it
/// as an owning `Box<dyn GeyserPlugin>` allocated by this dynamic library.
pub unsafe extern "C" fn _create_plugin() -> *mut dyn GeyserPlugin {
    let plugin: Box<dyn GeyserPlugin> = Box::new(SonarGeyserPlugin::default());
    Box::into_raw(plugin)
}
