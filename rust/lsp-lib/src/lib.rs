extern crate jsonrpc_lite;
extern crate languageserver_types as lsp_types;
#[macro_use]
extern crate url;
extern crate serde_json;
extern crate xi_core_lib as xi_core;
extern crate xi_plugin_lib;
extern crate xi_rope;

use serde_json::Value;
use jsonrpc_lite::Error;
use xi_plugin_lib::{mainloop}; 
use xi_plugin_lib::Plugin;

pub mod parse_helper;
pub mod types;
pub mod language_server;
pub mod lsp_plugin;

pub use lsp_plugin::LSPPlugin;
use language_server::LanguageServerClient;

pub trait Callable: Send {
    fn call(self: Box<Self>, client: &mut LanguageServerClient, result: Result<Value, Error>);
}

impl<F: Send + FnOnce(&mut LanguageServerClient, Result<Value, Error>)> Callable for F {
    fn call(self: Box<F>, client: &mut LanguageServerClient, result: Result<Value, Error>) {
        (*self)(client, result)
    }
}

pub type Callback = Box<Callable>;

pub fn start_mainloop<P: Plugin>(plugin: &mut P)  {
    mainloop(plugin);
}
