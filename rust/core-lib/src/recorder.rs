use std::mem;
use std::collections::HashMap;

use edit_types::{BufferEvent, EventDomain};

pub(crate) struct Recorder {
    active_recording: Option<String>,
    recordings: HashMap<String, Recording>,
}

impl Recorder {
    pub(crate) fn new() -> Recorder {
        Recorder {
            active_recording: None,
            recordings: HashMap::new(),
        }
    }

    pub(crate) fn is_recording(&self) -> bool {
        self.active_recording.is_some()
    }

    pub(crate) fn toggle_recording(&mut self, recording_name: Option<String>) {
        if self.is_recording() {
            let last_recording = self.active_recording.take().unwrap();

            // If a recording name was provided, we're going to switch
            if let Some(ref recording_name) = recording_name {
                if &last_recording == recording_name {
                    self.recordings.get_mut(&last_recording)
                        .and_then(|recording| {
                            recording.filter_undos();
                            Some(())
                        });
                    return;
                } else {
                    self.clear(&last_recording);
                }
            } else {
                self.recordings.get_mut(&last_recording)
                    .and_then(|recording| {
                        recording.filter_undos();
                        Some(())
                    });
            }
        }

        mem::replace(&mut self.active_recording, recording_name);
    }

    pub(crate) fn record(&mut self, cmd: EventDomain) {
        if !self.is_recording() {
            // We're not supposed to be recording? Can we log somehow?
            return;
        }

        let current_recording = self.active_recording.as_ref().unwrap();
        let recording = self.recordings.entry(current_recording.clone())
            .or_insert(Recording::new());
        recording.events.push(cmd);
    }

    pub(crate) fn play<F>(&self, recording_name: &str, action: F)
        where F: FnMut(&EventDomain) -> () {
        self.recordings.get(recording_name)
            .and_then(|recording| {
                recording.play(action);
                Some(())
            });
    }

    pub(crate) fn clear(&mut self, recording_name: &str) {
        self.recordings.remove(recording_name);
    }
}

struct Recording {
    events: Vec<EventDomain>
}

impl Recording {
    pub(crate) fn new() -> Recording {
        Recording {
            events: Vec::new()
        }
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

// Tests for filtering undo / redo from the recording buffer
// A = Event
// B = Event
// U = Undo
// R = Redo
#[cfg(test)]
mod tests {
    use recorder::Recorder;
    use edit_types::{BufferEvent, EventDomain};

    #[test]
    fn play_recording() {
        let mut recorder = Recorder::new();

        let recording_name = String::new();
        let mut expected_events: Vec<EventDomain> = vec![
            BufferEvent::Indent.into(),
            BufferEvent::Outdent.into(),
            BufferEvent::DuplicateLine.into(),
            BufferEvent::Transpose.into(),
        ];

        recorder.toggle_recording(Some(recording_name.clone()));
        for event in expected_events.iter().rev() {
            recorder.record(event.clone());
        }
        recorder.toggle_recording(Some(recording_name.clone()));

        recorder.play(&recording_name, |event| {
            // We shouldn't iterate more times than we added items!
            let expected_event = expected_events.pop();
            assert!(expected_event.is_some());

            // Should be the event we expect
            assert_eq!(*event, expected_event.unwrap());
        });

        // We should have iterated over everything we inserted
        assert_eq!(expected_events.len(), 0);
    }

    #[test]
    fn clear_recording() {
        let mut recorder = Recorder::new();

        let recording_name = String::new();

        recorder.toggle_recording(Some(recording_name.clone()));
        recorder.record(BufferEvent::Transpose.into());
        recorder.record(BufferEvent::DuplicateLine.into());
        recorder.record(BufferEvent::Outdent.into());
        recorder.record(BufferEvent::Indent.into());
        recorder.toggle_recording(Some(recording_name.clone()));

        assert_eq!(recorder.recordings.get(&recording_name).unwrap().events.len(), 4);

        recorder.clear(&recording_name);

        assert!(recorder.recordings.get(&recording_name).is_none());
    }

    #[test]
    fn basic_test() {
        let mut recorder = Recorder::new();

        let recording_name = String::new();

        // Undo removes last item, redo only affects undo
        // A U B R => B
        recorder.toggle_recording(Some(recording_name.clone()));
        recorder.record(BufferEvent::Transpose.into());
        recorder.record(BufferEvent::Undo.into());
        recorder.record(BufferEvent::DuplicateLine.into());
        recorder.record(BufferEvent::Redo.into());
        recorder.toggle_recording(Some(recording_name.clone()));
        assert_eq!(recorder.recordings.get(&recording_name).unwrap().events, vec![BufferEvent::DuplicateLine.into()]);
    }

    #[test]
    fn basic_test_swapped() {
        let mut recorder = Recorder::new();

        let recording_name = String::new();

        // Swapping order of undo and redo from the basic test should give us a different leftover item
        // A R B U => A
        recorder.toggle_recording(Some(recording_name.clone()));
        recorder.record(BufferEvent::Transpose.into());
        recorder.record(BufferEvent::Redo.into());
        recorder.record(BufferEvent::DuplicateLine.into());
        recorder.record(BufferEvent::Undo.into());
        recorder.toggle_recording(Some(recording_name.clone()));
        assert_eq!(recorder.recordings.get(&recording_name).unwrap().events, vec![BufferEvent::Transpose.into()]);
    }

    #[test]
    fn redo_cancels_undo() {
        let mut recorder = Recorder::new();

        let recording_name = String::new();

        // Redo cancels out an undo
        // A U R B => A B
        recorder.toggle_recording(Some(recording_name.clone()));
        recorder.record(BufferEvent::Transpose.into());
        recorder.record(BufferEvent::Undo.into());
        recorder.record(BufferEvent::Redo.into());
        recorder.record(BufferEvent::DuplicateLine.into());
        recorder.toggle_recording(Some(recording_name.clone()));
        assert_eq!(recorder.recordings.get(&recording_name).unwrap().events, vec![BufferEvent::Transpose.into(), BufferEvent::DuplicateLine.into()]);
    }

    #[test]
    fn undo_cancels_redo() {
        let mut recorder = Recorder::new();

        let recording_name = String::new();

        // Undo should cancel a redo, preventing it from canceling another undo
        // A U R U => _
        recorder.toggle_recording(Some(recording_name.clone()));
        recorder.record(BufferEvent::Transpose.into());
        recorder.record(BufferEvent::Undo.into());
        recorder.record(BufferEvent::Redo.into());
        recorder.record(BufferEvent::Undo.into());
        recorder.toggle_recording(Some(recording_name.clone()));
        assert_eq!(recorder.recordings.get(&recording_name).unwrap().events, vec![]);
    }

    #[test]
    fn undo_as_first_item() {
        let mut recorder = Recorder::new();

        let recording_name = String::new();

        // Undo shouldn't do anything as the first item
        // U A B R => A B
        recorder.toggle_recording(Some(recording_name.clone()));
        recorder.record(BufferEvent::Undo.into());
        recorder.record(BufferEvent::Transpose.into());
        recorder.record(BufferEvent::DuplicateLine.into());
        recorder.record(BufferEvent::Redo.into());
        recorder.toggle_recording(Some(recording_name.clone()));
        assert_eq!(recorder.recordings.get(&recording_name).unwrap().events, vec![BufferEvent::Transpose.into(), BufferEvent::DuplicateLine.into()]);
    }

    #[test]
    fn redo_as_first_item() {
        let mut recorder = Recorder::new();

        let recording_name = String::new();

        // Redo shouldn't do anything as the first item
        // R A B U => A
        recorder.toggle_recording(Some(recording_name.clone()));
        recorder.record(BufferEvent::Redo.into());
        recorder.record(BufferEvent::Transpose.into());
        recorder.record(BufferEvent::DuplicateLine.into());
        recorder.record(BufferEvent::Undo.into());
        recorder.toggle_recording(Some(recording_name.clone()));
        assert_eq!(recorder.recordings.get(&recording_name).unwrap().events, vec![BufferEvent::Transpose.into()]);
    }
}