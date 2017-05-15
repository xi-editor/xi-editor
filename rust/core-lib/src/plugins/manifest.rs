// Copyright 2017 Google Inc. All rights reserved.
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

//! Structured representation of a plugin's features and capabilities.

use std::io::{BufReader, Write};
use std::env;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::thread;
use std::process::{Command, Stdio};

use serde_json::Value;

use xi_rpc::RpcLoop;

use tabs::ViewIdentifier;
use syntax::SyntaxDefinition;
use super::PluginManagerRef;
use super::{Plugin, PluginRef};

// optional environment variable for debug plugin executables
static PLUGIN_DIR: &'static str = "XI_PLUGIN_DIR";

// example plugins. Eventually these should be loaded from disk.
pub fn debug_plugins() -> Vec<PluginDescription> {
    use self::PluginActivation::*;
    let plugin_dir = match env::var(PLUGIN_DIR).map(PathBuf::from) {
        Ok(p) => p,
        Err(_) => env::current_exe().unwrap().parent().unwrap().to_owned(),
    };
    print_err!("looking for debug plugins in {:?}", plugin_dir);

    let make_path = |p: &str| -> PathBuf {
        let mut pb = plugin_dir.clone();
        pb.push(p);
        pb
    };

    vec![
        PluginDescription::new("syntect", "0.0", make_path("xi-syntect-plugin"),
        vec![Autorun]),
        PluginDescription::new("braces", "0.0", make_path("bracket_example.py"),
        Vec::new()),
        PluginDescription::new("spellcheck", "0.0", make_path("spellcheck.py"),
        Vec::new()),
        PluginDescription::new("shouty", "0.0", make_path("shouty.py"),
        Vec::new()),
    ].iter()
        .filter(|desc|{ 
            if !desc.exec_path.exists() {
                print_err!("missing plugin {} at {:?}", desc.name, desc.exec_path);
                false
            } else {
                true
            }
        })
        .map(|desc| desc.to_owned())
        .collect::<Vec<_>>()
}

/// Describes attributes and capabilities of a plugin.
///
/// Note: - these will eventually be loaded from manifest files.
#[derive(Debug, Clone)]
pub struct PluginDescription {
    pub name: String,
    pub version: String,
    //scope: PluginScope,
    // more metadata ...
    /// path to plugin executable
    pub exec_path: PathBuf,
    /// Events that cause this plugin to run
    pub activations: Vec<PluginActivation>,
}

/// `PluginActivation`s represent events that trigger running a plugin.
#[derive(Debug, Clone)]
pub enum PluginActivation {
    /// Always run this plugin, when available.
    Autorun,
    /// Run this plugin if the provided SyntaxDefinition is active.
    #[allow(dead_code)]
    OnSyntax(SyntaxDefinition),
    /// Run this plugin in response to a given command.
    #[allow(dead_code)]
    OnCommand,
}

impl PluginDescription {
    fn new<S, P>(name: S, version: S, exec_path: P,
                 activations: Vec<PluginActivation>) -> Self
        where S: Into<String>, P: Into<PathBuf>
    {
        PluginDescription {
            name: name.into(),
            version: version.into(),
            exec_path: exec_path.into(),
            activations: activations,
        }
    }

    /// Starts the executable described in this `PluginDescription`.
    //TODO: make this a free function, & move out of manifest
    pub fn launch<W, C>(&self, manager_ref: &PluginManagerRef<W>,
                        view_id: &ViewIdentifier, completion: C)
        where W: Write + Send + 'static,
              C: FnOnce(Result<PluginRef<W>, &'static str>) + Send + 'static
              // TODO: a real result type
    {
        let path = self.exec_path.clone();
        let view_id = view_id.to_owned();
        let manager_ref = manager_ref.to_weak();
        let description = self.clone();

        thread::spawn(move || {
            print_err!("starting plugin at path {:?}", path);
            let mut child = Command::new(&path)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .spawn()
                .expect("plugin failed to start");
            let child_stdin = child.stdin.take().unwrap();
            let child_stdout = child.stdout.take().unwrap();
            let mut looper = RpcLoop::new(child_stdin);
            let peer = looper.get_peer();
            peer.send_rpc_notification("ping", &Value::Array(Vec::new()));
            let plugin = Plugin {
                peer: peer,
                process: child,
                manager: manager_ref,
                description: description,
                view_id: view_id,
            };
            let mut plugin_ref = PluginRef(Arc::new(Mutex::new(plugin)));
            completion(Ok(plugin_ref.clone()));
            looper.mainloop(|| BufReader::new(child_stdout), &mut plugin_ref);
        });
    }
}

