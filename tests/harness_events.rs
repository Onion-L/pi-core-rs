//! Port of `pi-core/agent/test/harness/events.test.ts`.

use std::sync::{Arc, Mutex};

use pi_core::agent::harness::events::{HarnessEvent, HarnessEventBus, RunOutcome};

fn run_start_event() -> HarnessEvent {
    HarnessEvent::RunStart {
        lane: "main".to_string(),
        run_id: "run-1".to_string(),
    }
}

fn run_end_event() -> HarnessEvent {
    HarnessEvent::RunEnd {
        lane: "main".to_string(),
        run_id: "run-1".to_string(),
        outcome: RunOutcome::Completed,
        leaf_id: "entry-1".to_string(),
    }
}

#[test]
fn delivers_matching_events_to_direct_listeners_and_watchers() {
    let events = HarnessEventBus::new();
    let direct: Arc<Mutex<Vec<HarnessEvent>>> = Arc::new(Mutex::new(Vec::new()));
    let watch_events: Arc<Mutex<Vec<HarnessEvent>>> = Arc::new(Mutex::new(Vec::new()));

    let direct_capture = Arc::clone(&direct);
    let listener_id = events.on(
        "run_start",
        Arc::new(move |event: &HarnessEvent| {
            direct_capture.lock().unwrap().push(event.clone());
        }),
    );
    let (_snapshot, watch) = events.watch(|| ());
    let watch_capture = Arc::clone(&watch_events);
    watch.start(Arc::new(move |event: &HarnessEvent| {
        watch_capture.lock().unwrap().push(event.clone());
    }));

    events.emit(run_start_event());
    events.emit(run_end_event());
    events.off("run_start", listener_id);
    events.emit(run_start_event());

    assert_eq!(*direct.lock().unwrap(), vec![run_start_event()]);
    assert_eq!(
        *watch_events.lock().unwrap(),
        vec![run_start_event(), run_end_event(), run_start_event()]
    );
}

#[test]
fn captures_a_snapshot_without_an_event_gap_then_flushes_and_delivers_live_events() {
    let events = Arc::new(HarnessEventBus::new());
    let expected_snapshot = 42usize;
    let received: Arc<Mutex<Vec<HarnessEvent>>> = Arc::new(Mutex::new(Vec::new()));

    let bus = Arc::clone(&events);
    let (snapshot, watch) = events.watch(move || {
        let snapshot = expected_snapshot;
        bus.emit(run_start_event());
        snapshot
    });

    assert_eq!(snapshot, expected_snapshot);
    assert!(received.lock().unwrap().is_empty());

    let received_capture = Arc::clone(&received);
    watch.start(Arc::new(move |event: &HarnessEvent| {
        received_capture.lock().unwrap().push(event.clone());
    }));
    assert_eq!(*received.lock().unwrap(), vec![run_start_event()]);

    events.emit(run_end_event());
    assert_eq!(
        *received.lock().unwrap(),
        vec![run_start_event(), run_end_event()]
    );

    watch.unsubscribe();
    events.emit(run_start_event());
    assert_eq!(
        *received.lock().unwrap(),
        vec![run_start_event(), run_end_event()]
    );
}
