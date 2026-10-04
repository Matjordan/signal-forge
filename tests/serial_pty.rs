#![cfg(target_os = "linux")]

use nix::{fcntl::{fcntl, FcntlArg, OFlag}, pty::{openpty, OpenptyResult}, sys::termios::{cfmakeraw, tcgetattr, tcsetattr, SetArg}, unistd::ttyname};
use signal_forge::{config::SerialSettings, endpoint::{ConnectionState, Endpoint}, send::{encode, Encoding, LineEnding}, serial::SerialEndpoint, traffic::{Direction, TrafficBus, TrafficEvent}};
use std::{fs::File, io::{Read, Write}, os::fd::AsRawFd, sync::{mpsc::Receiver, Arc}, thread, time::{Duration, Instant}};

fn pty() -> (File, String, std::os::fd::OwnedFd) {
    let OpenptyResult { master, slave } = openpty(None,None).unwrap();
    let mut settings=tcgetattr(&slave).unwrap();
    cfmakeraw(&mut settings);
    tcsetattr(&slave,SetArg::TCSANOW,&settings).unwrap();
    let path=ttyname(&slave).unwrap().to_string_lossy().into_owned();
    fcntl(master.as_raw_fd(),FcntlArg::F_SETFL(OFlag::O_NONBLOCK)).unwrap();
    (File::from(master),path,slave)
}
fn received(receiver:&Receiver<Arc<TrafficEvent>>,direction:Direction,expected:&[u8]) {
    let deadline=Instant::now()+Duration::from_secs(3);
    let mut bytes=Vec::new();
    while bytes.len()<expected.len() && Instant::now()<deadline {
        if let Ok(event)=receiver.recv_timeout(Duration::from_millis(20)) { if event.direction==direction { bytes.extend_from_slice(&event.bytes); } }
    }
    assert_eq!(bytes,expected);
}
fn read_bytes(master:&mut File,expected:&[u8]) {
    let deadline=Instant::now()+Duration::from_secs(3);
    let mut bytes=Vec::new();
    let mut buffer=[0;256];
    while bytes.len()<expected.len() && Instant::now()<deadline {
        match master.read(&mut buffer) {
            Ok(count) => bytes.extend_from_slice(&buffer[..count]),
            Err(e) if e.kind()==std::io::ErrorKind::WouldBlock => thread::sleep(Duration::from_millis(5)),
            Err(e) => panic!("{e}"),
        }
    }
    assert_eq!(bytes,expected);
}

#[test]
fn real_pty_text_binary_disconnect_and_reopen() {
    let (mut master,path,_slave)=pty();
    let bus=TrafficBus::default();
    let events=bus.subscribe(64);
    let settings=SerialSettings { path,..Default::default() };
    let mut endpoint=SerialEndpoint::open(&settings,bus.clone()).unwrap();
    let text=encode("test\\r\\n",Encoding::Text,true,LineEnding::None).unwrap();
    endpoint.send(text.clone()).unwrap();
    read_bytes(&mut master,&text);
    received(&events,Direction::Tx,&text);
    master.write_all(&[0,255,13,10]).unwrap();
    received(&events,Direction::Rx,&[0,255,13,10]);
    endpoint.disconnect();
    assert_eq!(endpoint.state(),ConnectionState::Disconnected);
    assert!(endpoint.send(vec![1]).is_err());
    let reopened=SerialEndpoint::open(&settings,bus).unwrap();
    reopened.send(vec![0,255]).unwrap();
    read_bytes(&mut master,&[0,255]);
}

#[test]
fn independent_ports_do_not_cross_streams() {
    let (mut first,path_first,_slave_first)=pty();
    let (mut second,path_second,_slave_second)=pty();
    let bus=TrafficBus::default();
    let a=SerialEndpoint::open(&SerialSettings { path:path_first,..Default::default() },bus.clone()).unwrap();
    let b=SerialEndpoint::open(&SerialSettings { path:path_second,..Default::default() },bus).unwrap();
    a.send(vec![1,2,3]).unwrap();
    b.send(vec![4,5,6]).unwrap();
    read_bytes(&mut first,&[1,2,3]);
    read_bytes(&mut second,&[4,5,6]);
    assert_ne!(a.id(),b.id());
}

#[test]
fn peer_close_faults_without_panicking() {
    let (master,path,slave)=pty();
    let endpoint=SerialEndpoint::open(&SerialSettings { path,..Default::default() },TrafficBus::default()).unwrap();
    drop(slave);
    drop(master);
    let deadline=Instant::now()+Duration::from_secs(3);
    while endpoint.state()==ConnectionState::Connected && Instant::now()<deadline { thread::sleep(Duration::from_millis(10)); }
    assert!(matches!(endpoint.state(),ConnectionState::Fault(_)));
    assert!(endpoint.send(vec![1]).is_err());
}
