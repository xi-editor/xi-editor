use edit_types::EventDomain;

pub(crate) struct Recorder {
    is_recording: bool,
    recording: Option<Recording>,
}

impl Recorder {
    pub(crate) fn new() -> Recorder {
        Recorder {
            is_recording: false,
            recording: None,
        }
    }

    pub(crate) fn is_recording(&self) -> bool {
        self.is_recording
    }

    pub(crate) fn toggle_recording(&mut self) {
        self.is_recording = !self.is_recording;
    }

    pub(crate) fn record(&mut self, cmd: EventDomain) {
        if !self.is_recording {
            // We're not supposed to be recording? Can we log somehow?
            return;
        }

        self.recording.iter_mut()
            .for_each(|recording| {
                recording.events.push(cmd.clone());
            });
    }

    pub(crate) fn play<F>(&self, action: F)
        where F: FnMut(&EventDomain) -> () {

        if let Some(ref recording) = self.recording {
            recording.events.iter().for_each(action)
        }
    }
}

struct Recording {
    events: Vec<EventDomain>
}