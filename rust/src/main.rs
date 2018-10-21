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

/// This function tries to create the parent directories for a file
///
/// It wraps around the `parent()` function of `Path` which returns an `Option<&Path>` and
/// `fs::create_dir_all` which returns an `io::Result<()>`.
///
/// This allows you to use `?`/`try!()` to create the dir and you recive the additional custom error for when `parent()`
/// returns nothing.
///
/// # Errors
/// This can return an `io::Error` if `fs::create_dir_all` fails or if `parent()` returns `None`.
/// See `Path`'s `parent()` function for more details.
/// # Examples
/// ```
/// use std::path::Path;
/// use std::ffi::OsStr;
///
/// let path_with_file = Path::new("/some/directory/then/file");
/// assert_eq!(Some(OsStr::new("file")), path_with_file.file_name());
/// assert_eq!(create_log_directory(path_with_file).is_ok(), true);
///
/// let path_with_other_file = Path::new("/other_file");
/// assert_eq!(Some(OsStr::new("other_file")), path_with_other_file.file_name());
/// assert_eq!(create_log_directory(path_with_file).is_ok(), true);
///
/// // Path that is just the root or prefix:
/// let path_without_file = Path::new("/");
/// assert_eq!(None, path_without_file.file_name());
/// assert_eq!(create_log_directory(path_without_file).is_ok(), false);
/// ```
fn create_log_directory(path_with_file: &Path) -> io::Result<()> {
    let log_dir = path_with_file.parent().ok_or(io::Error::new(
        io::ErrorKind::InvalidInput,
        format!(
            "Unable to get the parent of the following Path: {}, Your path should contain a file name",
            path_with_file.display(),
        ),
    ))?;
    fs::create_dir_all(log_dir)?;
    Ok(())
}

fn setup_logging(logging_path: Option<&Path>) -> Result<(), fern::InitError> {
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

    if let Some(logging_file_path) = logging_path {
        create_log_directory(logging_file_path)?;

        fern_dispatch = fern_dispatch.chain(fern::log_file(logging_file_path)?);
    };

    // Start fern
    fern_dispatch.apply()?;
    info!("Logging with fern is set up");

    // Log details of the logging_file_path result using fern/log
    // Either logging the path fern is outputting to or the error from obtaining the path
    match logging_path {
        Some(logging_file_path) => info!("Writing logs to: {}", logging_file_path.display()),
        None => warn!("No path was supplied for the log file. Not saving logs to disk, falling back to just stderr"),
    }
    Ok(())
}

fn generate_logging_path(logfile_config: LogfileConfig) -> Result<PathBuf, io::Error> {
    // Use the file name set in logfile_config or fallback to the default
    let logfile_file_name = match logfile_config.file {
        Some(file_name) => file_name,
        None => PathBuf::from(XI_LOG_FILE),
    };
    if logfile_file_name.eq(Path::new("")) {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "A blank file name was supplied"));
    };
    // Use the directory name set in logfile_config or fallback to the default
    let logfile_directory_name = match logfile_config.directory {
        Some(dir) => dir,
        None => PathBuf::from(XI_LOG_DIR),
    };

    let mut logging_directory_path = get_logging_directory_path(logfile_directory_name)?;

    // Add the file name & return the full path
    logging_directory_path.push(logfile_file_name);
    Ok(logging_directory_path)
}

fn get_flags() -> HashMap<String, Option<String>> {
    let mut flags: HashMap<String, Option<String>> = HashMap::new();

    let flag_prefix = "-";
    let mut args_iterator = std::env::args().peekable();
    while let Some(arg) = args_iterator.next() {
        if arg.starts_with(flag_prefix) {
            let key = arg.trim_left_matches(flag_prefix).to_string();

            // Check the next argument doesn't start with the flag prefix
            // map_or accounts for peek returning an Option
            let next_arg_not_a_flag: bool =
                args_iterator.peek().map_or(false, |val| !val.starts_with(flag_prefix));
            if next_arg_not_a_flag {
                flags.insert(key, args_iterator.next());
            }
        }
    }
    flags
}

struct EnvFlagConfig {
    env_name: &'static str,
    flag_name: &'static str,
}

/// Extracts a value from the flags and the env.
///
/// In this order: `String` from the flags, then `String` from the env, then `None`
fn extract_env_or_flag(
    flags: &HashMap<String, Option<String>>,
    conf: EnvFlagConfig,
) -> Option<String> {
    flags.get(conf.flag_name).cloned().unwrap_or(std::env::var(conf.env_name).ok())
}

struct LogfileConfig {
    directory: Option<PathBuf>,
    file: Option<PathBuf>,
}

fn generate_logfile_config(flags: &HashMap<String, Option<String>>) -> LogfileConfig {
    // If the key is set, get the Option within
    let log_dir_env_flag = EnvFlagConfig { env_name: "XI_LOG_DIR", flag_name: "log-dir" };
    let log_file_env_flag = EnvFlagConfig { env_name: "XI_LOG_FILE", flag_name: "log-file" };
    let log_dir_flag_option = extract_env_or_flag(&flags, log_dir_env_flag).map(PathBuf::from);

    let log_file_flag_option = extract_env_or_flag(&flags, log_file_env_flag).map(PathBuf::from);

    LogfileConfig { directory: log_dir_flag_option, file: log_file_flag_option }
}

fn main() {
    let mut state = XiCore::new();
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut rpc_looper = RpcLoop::new(stdout);

    let flags = get_flags();

    let logfile_config = generate_logfile_config(&flags);

    let logging_path_result = generate_logging_path(logfile_config);

    let logging_path =
        logging_path_result.as_ref().map(|p: &PathBuf| -> &Path { p.as_path() }).ok();

    if let Err(e) = setup_logging(logging_path) {
        eprintln!("[ERROR] setup_logging returned error, logging not enabled: {:?}", e);
    }
    if let Err(e) = logging_path_result.as_ref() {
        warn!("Unable to generate the logging path to pass to set up: {}", e)
    }

    match rpc_looper.mainloop(|| stdin.lock(), &mut state) {
        Ok(_) => (),
        Err(err) => error!("xi-core exited with error:\n{:?}", err),
    }
}
