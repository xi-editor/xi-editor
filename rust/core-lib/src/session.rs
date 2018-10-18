use xi_rope::rope::RopeInfo;
use xi_rope::delta::Delta;
use tabs::ViewId;
use serde_json::{self, Value};

use std::time::{Duration, Instant};
use std::collections::HashMap;
use std::fs::OpenOptions;
use std::io::Write;

const THROTTLE_TIME: Duration = Duration::from_millis(1000);

#[derive(Serialize, Deserialize, Default)]
struct SessionData {
    pub views: HashMap<ViewId, BufferData>,
}

#[derive(Serialize, Deserialize)]
struct BufferData {
    pub pristine_delta: Delta<RopeInfo>,
}

pub struct Session {
    last_write_time: Instant,
    data: SessionData,
}

impl Session {
    pub fn new() -> Self {
        Self { last_write_time: Instant::now(), data: SessionData::default() }
    }

    pub fn handle_idle(&mut self) {
        if Instant::now().duration_since(self.last_write_time) > THROTTLE_TIME {
            self.write();
            self.last_write_time = Instant::now();
        }
    }

    pub fn set_view_delta(&mut self, view_id: ViewId, delta: Delta<RopeInfo>) {
        self.data.views.insert(view_id, BufferData { pristine_delta: delta });
    }

    fn write(&mut self) {
        let mut file = OpenOptions::new().create(true).write(true).open("/Users/akxcv/xi-test").unwrap();
        let serialized_data = serde_json::to_string(&self.data).unwrap();
        file.write_all(&serialized_data.as_bytes()).unwrap();
    }
}
