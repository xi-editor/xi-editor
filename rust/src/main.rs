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

use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use xi_core_lib::XiCore;
use xi_rpc::RpcLoop;

const XI_LOG_DIR: &str = "xi-core";
const XI_LOG_FILE: &str = "xi-core.log";

fn get_logfile_directory_name() -> String {
    match std::env::var("XI_LOG_DIR") {
        Ok(name) => name,
        Err(_) => String::from(XI_LOG_DIR),
    }
}

fn get_logfile_file_name() -> String {
    match std::env::var("XI_LOG_FILE") {
        Ok(name) => name,
        Err(_) => String::from(XI_LOG_FILE),
    }
}

fn get_logging_directory<P: AsRef<Path>>(directory: P) -> Result<PathBuf, io::Error> {
    match dirs::data_local_dir() {
        Some(mut log_dir) => {
            log_dir.push(directory);
            Ok(log_dir)
        }
        None => Err(io::Error::new(
            io::ErrorKind::NotFound,
            "No standard logging directory known for this platform",
        )),
    }
}

fn path_for_log_file<P: AsRef<Path>>(filename: P) -> Result<PathBuf, io::Error> {
    let mut logging_directory = get_logging_directory(get_logfile_directory_name())?;
    // Create the logging directory
    fs::create_dir_all(&logging_directory)?;
    // Pushing the filename to the end
    logging_directory.push(filename.as_ref());
    Ok(logging_directory)
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

    let mut fern_dispatch = fern::Dispatch::new()
        .format(|out, message, record| {
            out.finish(format_args!(
                "{}[{}][{}] {}",
                chrono::Local::now().format("[%Y-%m-%d][%H:%M:%S]"),
                record.target(),
                record.level(),
                message,
            ))
        }).level(level_filter)
        .chain(io::stderr());

    let path_result = path_for_log_file(get_logfile_file_name());
    // If the logging_file_path returned successfully, add the logfile capability to fern
    if let Ok(logging_file_path) = &path_result {
        fern_dispatch = fern_dispatch.chain(fern::log_file(logging_file_path)?);
    }

    // Start fern
    fern_dispatch.apply()?;

    // Log details of the fern setup using fern/log
    match &path_result {
        Err(e) => {
            let message = "There was an issue getting the path for the log file";
            warn!("{}: {:?}, falling back to stderr.", message, e);
        }
        Ok(logging_file_path) => info!("Logging to the following file: {:?}", logging_file_path),
    }

    info!("Logging with fern is setup");
    Ok(())
}

fn get_flags() -> HashMap<String, Option<String>> {
    let mut flags: HashMap<String, Option<String>> = HashMap::new();

    let flag_prefix = "-";
    let mut args_itterator = std::env::args().peekable();
    while let Some(arg) = args_itterator.next() {
        if arg.starts_with(flag_prefix) {
            let key = arg.trim_left_matches(flag_prefix).to_string();

            // Check the next argument doesn't start with the flag prefix
            // map_or accounts for peek returning an Option
            let next_arg_not_a_flag: bool = args_itterator.peek().map_or(false, |val| !val.starts_with(flag_prefix));
            if next_arg_not_a_flag {
                flags.insert(key, args_itterator.next());
            }
        }
    }
    flags
}

fn main() {
    let mut state = XiCore::new();
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut rpc_looper = RpcLoop::new(stdout);

    if let Err(e) = setup_logging() {
        eprintln!(
            "[ERROR] setup_logging returned error, logging disabled: {:?}",
            e
        );
    }

    match rpc_looper.mainloop(|| stdin.lock(), &mut state) {
        Ok(_) => (),
        Err(err) => error!("xi-core exited with error:\n{:?}", err),
    }
}
