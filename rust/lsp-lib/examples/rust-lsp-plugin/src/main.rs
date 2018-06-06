extern crate xi_lsp_lib;
use xi_lsp_lib::{start_mainloop, LSPPlugin};

fn main() {
    // Assuming RLS is in default path i.e. ~/.cargo/bin/rls
    // TODO: Make this configurable
    let home_dir = std::env::home_dir();
    let mut rls_path = String::from(home_dir.unwrap().to_str().unwrap());
    rls_path.push_str(".cargo/bin/rls");

    let mut plugin = LSPPlugin::new(
        &rls_path,
        &[],
        vec!["rs".to_string()],
        Some("Cargo.toml".to_string()),
        "rust",
    );

    start_mainloop(&mut plugin);
}
