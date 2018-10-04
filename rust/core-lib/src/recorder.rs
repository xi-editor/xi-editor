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
            self.clear_recording();
        } else if self.is_recording {
            let mut saw_undo = false;
            let mut saw_redo = false;

            // Walk the recording backwards and remove any undo / redo events
            let filtered: Vec<EventDomain> = self.recording.events.clone().into_iter()
                .rev()
                .filter(|event| {
                    if let EventDomain::Buffer(event) = event {
                        match event {
                            BufferEvent::Undo => {
                                saw_undo = !saw_redo;
                                saw_redo = false;
                                return false;
                            }
                            BufferEvent::Redo => {
                                saw_redo = !saw_undo;
                                saw_undo = false;
                                return false;
                            }
                            _ => {
                                let ret = !saw_undo;
                                saw_undo = false;
                                saw_redo = false;
                                return ret;
                            }
                        }
                    }

                    true
                })
                .collect();

            mem::replace(&mut self.recording.events, filtered);
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

    fn clear_recording(&mut self) {
        self.recording.events.clear();
    }
}

struct Recording {
    events: Vec<EventDomain>
}

#[cfg(test)]
mod tests {
    use recorder::Recorder;
    use edit_types::BufferEvent;

    #[test]
    fn undo_filtering_tests() {
        let mut recorder = Recorder::new();

        // Tests for filtering undo / redo from the recording buffer
        // A = Action
        // U = Undo
        // R = Redo

        // Undo removes last item, redo only affects undo
        // A U A R => A
        recorder.toggle_recording();
        recorder.record(BufferEvent::Transpose.into());
        recorder.record(BufferEvent::Undo.into());
        recorder.record(BufferEvent::Transpose.into());
        recorder.record(BufferEvent::Redo.into());
        recorder.toggle_recording();
        assert_eq!(recorder.recording.events, vec![BufferEvent::Transpose.into()]);

        recorder.clear_recording();

        // Swapping order shouldn't change the outcome
        // A R A U => A
        recorder.toggle_recording();
        recorder.record(BufferEvent::Transpose.into());
        recorder.record(BufferEvent::Redo.into());
        recorder.record(BufferEvent::Transpose.into());
        recorder.record(BufferEvent::Undo.into());
        recorder.toggle_recording();
        assert_eq!(recorder.recording.events, vec![BufferEvent::Transpose.into()]);

        recorder.clear_recording();

        // Redo cancels out an undo
        // A U R A => A A
        recorder.toggle_recording();
        recorder.record(BufferEvent::Transpose.into());
        recorder.record(BufferEvent::Undo.into());
        recorder.record(BufferEvent::Redo.into());
        recorder.record(BufferEvent::Transpose.into());
        recorder.toggle_recording();
        assert_eq!(recorder.recording.events, vec![BufferEvent::Transpose.into(), BufferEvent::Transpose.into()]);

        recorder.clear_recording();

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