use xi_rope::rope::RopeInfo;
use xi_rope::delta::Delta;
use tabs::ViewId;
use serde_json::{self, Value};

use std::time::{Duration, Instant};
use std::collections::HashMap;
use std::fs::OpenOptions;
use std::io::Write;

// NOTE: increase delay to notice the effect
pub(crate) const SESSION_SAVE_DELAY: Duration = Duration::from_millis(1000);

// TODO: is this OK?
pub(crate) const SESSION_SAVE_MASK: usize = 1 << 26;

#[derive(Serialize, Deserialize, Default)]
struct SessionData {
    pub views: HashMap<ViewId, BufferData>,
}

#[derive(Serialize, Deserialize)]
struct BufferData {
    pub pristine_delta: Delta<RopeInfo>,
}

pub struct Session {
    data: SessionData,
    pub has_pending_save: bool,
}

impl Session {
    pub fn new() -> Self {
        Self { data: SessionData::default(), has_pending_save: false }
    }

    pub fn set_view_delta(&mut self, view_id: ViewId, delta: Delta<RopeInfo>) {
        self.data.views.insert(view_id, BufferData { pristine_delta: delta });
    }

    pub fn save(&mut self) {
        // TODO: save somewhere appropriate
        let mut file = OpenOptions::new().create(true).write(true).open(
            format!("{}/{}", env!("HOME"), ".xi-session")
        ).unwrap();
        let serialized_data = format!("{}\n", serde_json::to_string(&self.data).unwrap());
        file.write_all(&serialized_data.as_bytes()).unwrap();
        self.set_has_pending_save(false);
    }

    pub fn set_has_pending_save(&mut self, value: bool) {
        self.has_pending_save = value;
    }
}
