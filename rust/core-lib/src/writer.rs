// TODO name this file better

use std::fs::OpenOptions;
use std::io::Write;
use std::time::{Duration, Instant};

const THROTTLE_TIME: Duration = Duration::from_millis(1000);

// TODO: better name
pub struct Writer {
    last_write_time: Instant,
    pending_data: Option<String>,
}

impl Writer {
    pub fn new() -> Self {
        Self { last_write_time: Instant::now(), pending_data: None }
    }

    pub fn stage(&mut self, data: String) {
        info!("stage, {:?}", data);
        self.pending_data = Some(data);
    }

    pub fn handle_idle(&mut self) {
        info!("handle idle");
        if Instant::now().duration_since(self.last_write_time) > THROTTLE_TIME {
            self.write();
            self.last_write_time = Instant::now();
        }
    }

    fn write(&mut self) {
        info!("write, {:?}", self.pending_data);
        if let Some(data) = self.pending_data.take() {
            let mut file =
                OpenOptions::new().create(true).write(true).open("/Users/akxcv/xi-test").unwrap();
            file.write_all(&data.as_bytes()).unwrap();
        }
    }
}
