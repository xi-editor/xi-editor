use edit_types::EventDomain;

pub(crate) struct Recorder {
    is_recording: bool,
    recording: Recording,
}

impl Recorder {
    pub(crate) fn new() -> Recorder {
        Recorder {
            is_recording: false,
            recording: Recording {
                events: Vec::new()
            },
        }
    }

    pub(crate) fn is_recording(&self) -> bool {
        self.is_recording
    }

    pub(crate) fn toggle_recording(&mut self) {
        if !self.is_recording && !self.recording.events.is_empty() {
            self.recording.events.clear();
        }

        self.is_recording = !self.is_recording;
    }

    pub(crate) fn record(&mut self, cmd: EventDomain) {
        if !self.is_recording {
            // We're not supposed to be recording? Can we log somehow?
            return;
        }

        self.recording.events.push(cmd.clone());
    }

    pub(crate) fn play<F>(&self, action: F)
        where F: FnMut(&EventDomain) -> () {
        self.recording.events.iter().for_each(action)
    }
}

struct Recording {
    events: Vec<EventDomain>
}