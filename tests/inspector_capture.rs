use signal_forge::{capture::{Capture,CaptureState},endpoint::EndpointId,inspector::{Inspector,DirectionFilter,BridgeDirection,HISTORY_LIMIT,timestamp_utc},traffic::{Direction,TrafficBus,TrafficEvent}};
use std::{fs,path::PathBuf,sync::{Arc,atomic::{AtomicU64,Ordering}},time::{Duration,UNIX_EPOCH}};
fn path()->PathBuf {
    static ID:AtomicU64=AtomicU64::new(0);
    std::env::temp_dir().join(format!("signal-forge-capture-{}-{}.jsonl",std::process::id(),ID.fetch_add(1,Ordering::Relaxed)))
}
fn records(path:&std::path::Path)->Vec<serde_json::Value> { fs::read_to_string(path).unwrap().lines().map(|line|serde_json::from_str(line).unwrap()).collect() }
fn event(sequence:u64,a:bool,millis:u64)->Arc<TrafficEvent> {
    Arc::new(TrafficEvent { sequence, timestamp:UNIX_EPOCH+Duration::from_millis(millis), endpoint:EndpointId(if a { "A" } else { "B" }.into()), direction:Direction::Rx,bytes:Arc::from([0,255]) })
}
#[test]
fn inspector_keeps_bounded_unfiltered_history_and_deltas_across_pause_and_gaps() {
    let mut inspector=Inspector::new(); let a=EndpointId("A".into());
    inspector.receive(event(1,true,1000),&a);
    inspector.paused=true; inspector.receive(event(2,false,1010),&a);
    inspector.paused=false; inspector.receive(event(4,false,1025),&a);
    assert_eq!(inspector.rows.len(),2);
    let row=inspector.rows.back().unwrap(); assert_eq!(row.delta_ns,Some(15_000_000)); assert_eq!(row.missed_before,1);
    inspector.filter=DirectionFilter::AToB;
    assert_eq!(inspector.rows.iter().filter(|r| inspector.filter.accepts(r.direction)).count(),1);
    assert!(DirectionFilter::BToA.accepts(BridgeDirection::BToA)); assert!(!DirectionFilter::BToA.accepts(BridgeDirection::AToB));
    for seq in 5..(HISTORY_LIMIT as u64 +20) { inspector.receive(event(seq,true,seq),&a); }
    assert_eq!(inspector.rows.len(),HISTORY_LIMIT);
    inspector.clear(); assert!(inspector.rows.is_empty());
    inspector.receive(event(HISTORY_LIMIT as u64+20,true,1),&a); assert!(inspector.rows[0].delta_ns.unwrap()<0);
}
#[test]
fn full_utc_timestamps_handle_epoch_leap_days_and_clock_before_epoch() {
    assert_eq!(timestamp_utc(UNIX_EPOCH),"1970-01-01 00:00:00.000000000 UTC");
    assert_eq!(timestamp_utc(UNIX_EPOCH+Duration::from_secs(951782400)),"2000-02-29 00:00:00.000000000 UTC");
    assert_eq!(timestamp_utc(UNIX_EPOCH-Duration::from_millis(1)),"1969-12-31 23:59:59.999000000 UTC");
}
#[test]
fn capture_preserves_raw_binary_directions_timestamps_and_summary() {
    let bus=TrafficBus::default(); let path=path();
    let mut capture=Capture::start_subscription(&path,EndpointId("A".into()),EndpointId("B".into()),bus.subscribe_tracked(64)).unwrap();
    let bytes:Vec<_>=(0..=255).collect();
    bus.publish(EndpointId("A".into()),Direction::Rx,&bytes);
    bus.publish(EndpointId("B".into()),Direction::Rx,b"reverse\r\n\0");
    capture.finish(); assert_eq!(capture.status().state,CaptureState::Completed);
    let lines=records(&path); assert_eq!(lines.len(),4);
    assert_eq!(lines[0]["format"],"signal-forge-capture"); assert_eq!(lines[0]["version"],1);
    assert_eq!(lines[1]["direction"],"a_to_b"); assert_eq!(lines[2]["direction"],"b_to_a");
    assert_eq!(lines[1]["raw_bytes"],serde_json::json!(bytes));
    let first=lines[1]["timestamp_unix_ns"].as_str().unwrap().parse::<i128>().unwrap();
    let second=lines[2]["timestamp_unix_ns"].as_str().unwrap().parse::<i128>().unwrap();
    assert_eq!(lines[2]["delta_ns"].as_str().unwrap().parse::<i128>().unwrap(),second-first);
    assert!(lines[1]["delta_ns"].is_null()); assert_eq!(lines[3]["events"],2); assert_eq!(lines[3]["bytes"],266);
    assert_eq!(lines[3]["complete"],true); fs::remove_file(path).unwrap();
}
#[test]
fn capture_reports_exact_drops_including_at_start_and_end_without_blocking_publish() {
    let bus=TrafficBus::default(); let subscription=bus.subscribe_tracked(1);
    for _ in 0..100 { bus.publish(EndpointId("A".into()),Direction::Rx,b"lost?"); }
    assert_eq!(subscription.dropped_events(),99);
    let path=path(); let mut capture=Capture::start_subscription(&path,EndpointId("A".into()),EndpointId("B".into()),subscription).unwrap();
    capture.finish(); let lines=records(&path);
    assert_eq!(lines[0]["queue_capacity"],1); assert_eq!(lines.last().unwrap()["dropped_events"],99);
    assert_eq!(lines.last().unwrap()["complete"],false); assert_eq!(capture.status().dropped,99);
    fs::remove_file(path).unwrap();
}
#[test]
fn capture_refuses_overwrite_and_bad_paths_and_drop_finalizes_queued_data() {
    let bus=TrafficBus::default(); let path=path(); fs::write(&path,b"existing").unwrap();
    assert!(Capture::start_subscription(&path,EndpointId("A".into()),EndpointId("B".into()),bus.subscribe_tracked(4)).is_err());
    assert_eq!(fs::read(&path).unwrap(),b"existing"); fs::remove_file(&path).unwrap();
    assert!(Capture::start_subscription(&path.join("missing"),EndpointId("A".into()),EndpointId("B".into()),bus.subscribe_tracked(4)).is_err());
    let capture=Capture::start_subscription(&path,EndpointId("A".into()),EndpointId("B".into()),bus.subscribe_tracked(4)).unwrap();
    bus.publish(EndpointId("B".into()),Direction::Rx,b"persist on drop"); drop(capture);
    let lines=records(&path); assert_eq!(lines.last().unwrap()["events"],1); fs::remove_file(path).unwrap();
}
#[test]
fn subscription_cutoff_is_stable_while_other_subscribers_continue() {
    let bus=TrafficBus::default(); let closed=bus.subscribe_tracked(2); let live=bus.subscribe(4);
    bus.publish(EndpointId("A".into()),Direction::Rx,b"before"); closed.close();
    for _ in 0..3 { bus.publish(EndpointId("B".into()),Direction::Rx,b"after"); }
    assert_eq!(closed.receiver.try_iter().count(),1); assert_eq!(closed.dropped_events(),0);
    assert_eq!(live.try_iter().count(),4);
}
#[test]
fn large_capture_streams_more_data_than_display_retains() {
    let bus=TrafficBus::default(); let path=path();
    let mut capture=Capture::start_subscription(&path,EndpointId("A".into()),EndpointId("B".into()),bus.subscribe_tracked(4096)).unwrap();
    let mut inspector=Inspector::new(); let receiver=bus.subscribe(4); let a=EndpointId("A".into());
    let deadline=std::time::Instant::now()+Duration::from_secs(5);
    for batch in 0..20 {
        for _ in 0..100 { bus.publish(a.clone(),Direction::Rx,&[batch;256]); }
        for event in receiver.try_iter() { inspector.receive(event,&a); }
        while capture.status().events < (batch as u64+1)*100 {
            assert!(std::time::Instant::now()<deadline); std::thread::sleep(Duration::from_millis(1));
        }
    }
    capture.finish(); assert_eq!(capture.status().events,2000); assert_eq!(capture.status().bytes,512000);
    assert!(inspector.rows.len()<=HISTORY_LIMIT); assert!(inspector.rows.len()<2000);
    use std::io::{BufRead,BufReader};
    assert_eq!(BufReader::new(fs::File::open(&path).unwrap()).lines().count(),2002);
    fs::remove_file(path).unwrap();
}
