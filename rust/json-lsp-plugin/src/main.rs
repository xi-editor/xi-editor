extern crate xi_lsp_lib;
use xi_lsp_lib::{start_mainloop, LSPPlugin};

fn main() {
    let mut plugin = LSPPlugin::new("vscode-json-languageserver", &["--stdio"]);
    start_mainloop(&mut plugin);
}
