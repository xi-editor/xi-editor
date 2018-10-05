use std::mem;

use edit_types::{BufferEvent, EventDomain};

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
            self.recording.clear();
        } else if self.is_recording {
            self.recording.filter_undos();
        }

        self.is_recording = !self.is_recording;
    }

    pub(crate) fn record(&mut self, cmd: EventDomain) {
        if !self.is_recording {
            // We're not supposed to be recording? Can we log somehow?
            return;
        }

        self.recording.events.push(cmd);
    }

    pub(crate) fn play<F>(&self, action: F)
        where F: FnMut(&EventDomain) -> () {
        self.recording.play(action);
    }

    pub(crate) fn clear(&mut self) {
        self.recording.clear();
    }
}

struct Recording {
    events: Vec<EventDomain>
}

impl Recording {
    pub(crate) fn clear(&mut self) {
        self.events.clear();
    }

    pub(crate) fn play<F>(&self, action: F)
        where F: FnMut(&EventDomain) -> () {
        self.events.iter().for_each(action)
    }

    pub(crate) fn filter_undos(&mut self) {
        let mut saw_undo = false;
        let mut saw_redo = false;

        // Walk the recording backwards and remove any undo / redo events
        let filtered: Vec<EventDomain> = self.events.clone()
            .into_iter()
            .rev()
            .filter(|event| {
                if let EventDomain::Buffer(event) = event {
                    return match event {
                        BufferEvent::Undo => {
                            saw_undo = !saw_redo;
                            saw_redo = false;
                            false
                        }
                        BufferEvent::Redo => {
                            saw_redo = !saw_undo;
                            saw_undo = false;
                            false
                        }
                        _ => {
                            let ret = !saw_undo;
                            saw_undo = false;
                            saw_redo = false;
                            ret
                        }
                    };
                }

                true
            })
            .collect::<Vec<EventDomain>>()
            .into_iter() // Why does rev().filter().rev().collect() cancel out the first rev()????
            .rev()
            .collect();

        mem::replace(&mut self.events, filtered);
    }
}

#[cfg(test)]
mod tests {
    use recorder::Recorder;
    use edit_types::BufferEvent;

    #[test]
    fn undo_filtering_tests() {
        let mut recorder = Recorder::new();

        // Tests for filtering undo / redo from the recording buffer
        // A = Event
        // B = Event
        // U = Undo
        // R = Redo

        // Undo removes last item, redo only affects undo
        // A U B R => B
        recorder.toggle_recording();
        recorder.record(BufferEvent::Transpose.into());
        recorder.record(BufferEvent::Undo.into());
        recorder.record(BufferEvent::DuplicateLine.into());
        recorder.record(BufferEvent::Redo.into());
        recorder.toggle_recording();
        assert_eq!(recorder.recording.events, vec![BufferEvent::DuplicateLine.into()]);

        recorder.clear();

        // Swapping order of undo and redo should give us a different leftover item
        // A R B U => A
        recorder.toggle_recording();
        recorder.record(BufferEvent::Transpose.into());
        recorder.record(BufferEvent::Redo.into());
        recorder.record(BufferEvent::DuplicateLine.into());
        recorder.record(BufferEvent::Undo.into());
        recorder.toggle_recording();
        assert_eq!(recorder.recording.events, vec![BufferEvent::Transpose.into()]);

        recorder.clear();

        // Redo cancels out an undo
        // A U R B => A B
        recorder.toggle_recording();
        recorder.record(BufferEvent::Transpose.into());
        recorder.record(BufferEvent::Undo.into());
        recorder.record(BufferEvent::Redo.into());
        recorder.record(BufferEvent::DuplicateLine.into());
        recorder.toggle_recording();
        assert_eq!(recorder.recording.events, vec![BufferEvent::Transpose.into(), BufferEvent::DuplicateLine.into()]);

        recorder.clear();

        // Undo should cancel a redo, preventing it from canceling another undo
        // A U R U => _
        recorder.toggle_recording();
        recorder.record(BufferEvent::Transpose.into());
        recorder.record(BufferEvent::Undo.into());
        recorder.record(BufferEvent::Redo.into());
        recorder.record(BufferEvent::Undo.into());
        recorder.toggle_recording();
        assert_eq!(recorder.recording.events, vec![]);
    }
}