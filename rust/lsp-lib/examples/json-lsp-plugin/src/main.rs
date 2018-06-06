extern crate xi_lsp_lib;
use xi_lsp_lib::{start_mainloop, LSPPlugin};

fn main() {
    
    // TODO: Make path to plugin configurable
    let mut plugin = LSPPlugin::new(
        "/usr/local/bin/vscode-json-languageserver",
        &["--stdio"],
        vec!["json".to_string(), "jsonc".to_string()],
        None,
        "json",
    ); 
    
    start_mainloop(&mut plugin);
}
