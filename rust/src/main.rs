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
use std::io;
use std::fs;
use std::path::PathBuf;

#[macro_use]
extern crate log;
extern crate chrono;
extern crate fern;

extern crate dirs;
use dirs::data_local_dir;

extern crate xi_core_lib;
extern crate xi_rpc;

use xi_core_lib::XiCore;
use xi_rpc::RpcLoop;

fn get_logging_directory() -> Result<PathBuf, fern::InitError> {
    let xi_directory = "xi-core/";
    if let Some(mut log_dir) = data_local_dir() {
        log_dir.push(xi_directory);
        Ok(log_dir)
    } else {
        let dir_error = io::Error::new(io::ErrorKind::Other, "dir was not able to find a directory");
        Err(fern::InitError::Io(dir_error))
    }
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

    let logging_directory = get_logging_directory()?;
    // Create the logging directory
    fs::create_dir_all(&logging_directory)?;

    let xi_log_file = "xi-core.log";
    let mut logging_file_path = logging_directory;
    logging_file_path.push(xi_log_file);
    fern::Dispatch::new()
        .format(|out, message, record| {
            out.finish(format_args!(
                "{}[{}][{}] {}",
                chrono::Local::now().format("[%Y-%m-%d][%H:%M:%S]"),
                record.target(),
                record.level(),
                message
            ))
        }).level(level_filter)
        .chain(io::stderr())
        .chain(fern::log_file(logging_file_path)?)
        .apply()?;
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
