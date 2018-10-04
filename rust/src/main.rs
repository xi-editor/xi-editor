// Copyright 2016 The xi-editor Authors.
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

#[macro_use]
extern crate log;
extern crate chrono;
extern crate fern;

extern crate dirs;

extern crate xi_core_lib;
extern crate xi_rpc;

use std::io;
use std::fs;
use std::path::PathBuf;

use xi_core_lib::XiCore;
use xi_rpc::RpcLoop;

const XI_DIRECTORY: &'static str = "xi-core";
const XI_LOG_FILE: &'static str = "xi-core.log";

fn get_logging_directory() -> Option<PathBuf> {
    if let Some(mut log_dir) = dirs::data_local_dir() {
        log_dir.push(XI_DIRECTORY);
        Some(log_dir)
    } else {
        eprintln!("[WARNING] The dir library was not able to find a directory for this platform");
        None
    }
}

fn path_for_log_file<P: AsRef<str>>(filename: P) -> Option<PathBuf> {
    if let Some(mut logging_directory) = get_logging_directory() {
        // Create the logging directory
        match fs::create_dir_all(&logging_directory) {
            Ok(_) => (), Err(why) => eprintln!("[WARNING] {:?}", why)
        }
        logging_directory.push(filename.as_ref());
        Some(logging_directory)
    } else { None }
}

fn setup_logging() -> Result<(), fern::InitError> {
    let level_filter = match std::env::var("XI_LOG") {
        Ok(level) => match level.to_lowercase().as_ref() {
            "trace" => log::LevelFilter::Trace,
            "debug" => log::LevelFilter::Debug,
            _ => log::LevelFilter::Info,
        },
        // Default to info
        Err(_) => log::LevelFilter::Info,
    };

    let fern_dispatch = fern::Dispatch::new()
        .format(|out, message, record| {
            out.finish(format_args!(
                "{}[{}][{}] {}",
                chrono::Local::now().format("[%Y-%m-%d][%H:%M:%S]"),
                record.target(),
                record.level(),
                message,
            ))
        })
        .level(level_filter)
        .chain(io::stderr());

    if let Some(logging_file_path) = path_for_log_file(XI_LOG_FILE) {
        fern_dispatch
            .chain(fern::log_file(logging_file_path)?)
            .apply()?;
    } else {
        eprintln!("[WARNING] There was an issue getting the path for the log file, using stdout");
        fern_dispatch
            .chain(io::stdout())
            .apply()?;
    }
    info!("Logging with fern setup");
    Ok(())
}

fn main() {
    let mut state = XiCore::new();
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut rpc_looper = RpcLoop::new(stdout);

    if let Err(e) = setup_logging() {
        eprintln!("[ERROR] setup_logging returned error, logging disabled: {:?}", e);
    }

    match rpc_looper.mainloop(|| stdin.lock(), &mut state) {
        Ok(_) => (),
        Err(err) => error!("xi-core exited with error:\n{:?}", err),
    }
}
