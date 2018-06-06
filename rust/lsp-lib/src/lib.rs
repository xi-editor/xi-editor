extern crate jsonrpc_lite;
extern crate languageserver_types as lsp_types;
extern crate serde_json;
extern crate url;
extern crate xi_core_lib as xi_core;
extern crate xi_plugin_lib;
extern crate xi_rope;

use xi_plugin_lib::mainloop;
use xi_plugin_lib::Plugin;

pub mod language_server;
pub mod lsp_plugin;
pub mod parse_helper;
pub mod types;
pub use lsp_plugin::LSPPlugin;

pub fn start_mainloop<P: Plugin>(plugin: &mut P) {
    mainloop(plugin);
}
