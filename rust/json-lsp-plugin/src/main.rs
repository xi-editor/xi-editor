extern crate xi_lsp_lib;
use xi_lsp_lib::{XiLSPPlugin, start_mainloop};

fn main() {

    eprintln!("PT 1");
    let mut plugin = XiLSPPlugin::new("vscode-json-languageserver",&["--stdio"]);
    
    eprintln!("PT 2");
    start_mainloop(&mut plugin);
    
}

