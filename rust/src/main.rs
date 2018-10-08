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

fn get_logging_directory_path<P: AsRef<Path>>(directory: P) -> Result<PathBuf, io::Error> {
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

fn setup_logging(logging_path_result: Result<PathBuf, io::Error>) -> Result<(), fern::InitError> {
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

    if let Ok(logging_file_path) = &logging_path_result {
        // Ensure the logging directory is created
        let parent_path = logging_file_path.parent().ok_or(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "Unable to get the parent of the following Path: {}",
                logging_file_path.display(),
            ),
        ))?;
        fs::create_dir_all(parent_path)?;
        // Attach it to fern
        fern_dispatch = fern_dispatch.chain(fern::log_file(logging_file_path)?);
    };

    // Start fern
    fern_dispatch.apply()?;
    info!("Logging with fern is setup");

    // Log details of the logging_file_path result using fern/log
    // Either logging the path fern is outputting to or the error from obtaining the path
    match &logging_path_result {
        Ok(logging_file_path) => {
            info!(
                "Logging to the following file: {}",
                logging_file_path.display()
            );
        }
        Err(e) => {
            let message = "There was an issue getting the path for the log file";
            warn!("{}: {:?}, falling back to stderr.", message, e);
        }
    }
    Ok(())
}

fn prepare_logging_path(logfile_config: LogfileConfig) -> Result<PathBuf, io::Error> {
    // Use the file name set in logfile_config or fallback to the default
    let logfile_file_name = match logfile_config.file {
        Some(file_name) => file_name,
        None => XI_LOG_FILE.to_string(),
    };
    // Use the directory name set in logfile_config or fallback to the default
    let logfile_directory_name = match logfile_config.directory {
        Some(dir) => dir,
        None => XI_LOG_DIR.to_string(),
    };

    let mut logging_directory_path = get_logging_directory_path(logfile_directory_name)?;

    // Add the file name & return the full path
    logging_directory_path.push(logfile_file_name);
    Ok(logging_directory_path)
}

struct LogfileConfig {
    directory: Option<String>,
    file: Option<String>,
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
            let next_arg_not_a_flag: bool = args_itterator
                .peek()
                .map_or(false, |val| !val.starts_with(flag_prefix));
            if next_arg_not_a_flag {
                flags.insert(key, args_itterator.next());
            }
        }
    }
    flags
}

struct EnvFlagConfig {
    env_name: &'static str,
    flag_name: &'static str,
}

fn extract_env_or_flag(flags: &HashMap<String, Option<String>>, conf: EnvFlagConfig) -> Option<String> {
    std::env::var(conf.env_name)
        .ok()
        .or(flags.get(conf.flag_name).cloned().unwrap_or(None))
}

fn generate_logfile_config(flags: &HashMap<String, Option<String>>) -> LogfileConfig {
    // If the key is set, get the Option within
    let log_dir_env_flag = EnvFlagConfig {
        env_name: "XI_LOG_DIR",
        flag_name: "log-dir",
    };
    let log_file_env_flag = EnvFlagConfig {
        env_name: "XI_LOG_FILE",
        flag_name: "log-file",
    };
    let log_dir_flag_option = extract_env_or_flag(&flags, log_dir_env_flag);

    let log_file_flag_option = extract_env_or_flag(&flags, log_file_env_flag);

    LogfileConfig {
        directory: log_dir_flag_option,
        file: log_file_flag_option,
    }
}

fn main() {
    let mut state = XiCore::new();
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut rpc_looper = RpcLoop::new(stdout);

    let flags: HashMap<String, Option<String>> = get_flags();

    let logfile_config = generate_logfile_config(&flags);

    let logging_path: Result<PathBuf, io::Error> = prepare_logging_path(logfile_config);
    if let Err(e) = setup_logging(logging_path) {
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
