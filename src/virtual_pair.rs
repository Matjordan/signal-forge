//! Owned Linux PTYs joined by a bounded, full-duplex internal relay.
use nix::{fcntl::{fcntl,FcntlArg,OFlag},pty::openpty,sys::termios::{cfmakeraw,tcgetattr,tcsetattr,SetArg},unistd::ttyname};
use std::{collections::VecDeque,fs::File,io::{Read,Write},os::fd::AsRawFd,path::{Path,PathBuf},sync::{atomic::{AtomicBool,Ordering},Arc,Mutex},thread::{self,JoinHandle},time::Duration};

#[derive(Debug,Clone,PartialEq,Eq)]
pub enum PairState { Running, Stopped, Fault(String) }

struct OwnedLink { path:PathBuf, target:PathBuf }
impl Drop for OwnedLink {
    fn drop(&mut self) {
        // Never unlink a path another process replaced after creation.
        if std::fs::read_link(&self.path).ok().as_ref()==Some(&self.target) {
            if let Err(error)=std::fs::remove_file(&self.path) { log::warn!("{}: {error}",self.path.display()); }
        }
    }
}

pub struct VirtualPair {
    pub name:String,
    pub paths:[String;2],
    pub raw_paths:[String;2],
    state:Arc<Mutex<PairState>>,
    stop:Arc<AtomicBool>,
    worker:Option<JoinHandle<()>>,
    links:Vec<OwnedLink>,
}
impl VirtualPair {
    pub fn create(name:&str, directory:Option<&Path>) -> Result<Self,String> {
        if name.is_empty() || name.len()>64 || !name.bytes().all(|b| b.is_ascii_alphanumeric() || b==b'_' || b==b'-') {
            return Err("Pair name must contain 1–64 letters, digits, hyphens, or underscores".into());
        }
        let first=openpty(None,None).map_err(|e|e.to_string())?;
        let second=openpty(None,None).map_err(|e|e.to_string())?;
        for slave in [&first.slave,&second.slave] {
            let mut settings=tcgetattr(slave).map_err(|e|e.to_string())?;
            cfmakeraw(&mut settings);
            tcsetattr(slave,SetArg::TCSANOW,&settings).map_err(|e|e.to_string())?;
        }
        let raw_paths=[ttyname(&first.slave).map_err(|e|e.to_string())?.to_string_lossy().into_owned(),ttyname(&second.slave).map_err(|e|e.to_string())?.to_string_lossy().into_owned()];
        let mut links=Vec::new();
        let mut paths=raw_paths.clone();
        if let Some(directory)=directory {
            std::fs::create_dir_all(directory).map_err(|e|format!("{}: {e}",directory.display()))?;
            for index in 0..2 {
                let path=directory.join(format!("{name}-{}",if index==0 { "a" } else { "b" }));
                let target=PathBuf::from(&raw_paths[index]);
                std::os::unix::fs::symlink(&target,&path).map_err(|e|format!("{}: {e}",path.display()))?;
                paths[index]=path.to_string_lossy().into_owned();
                links.push(OwnedLink { path,target });
            }
        }
        for master in [&first.master,&second.master] {
            fcntl(master.as_raw_fd(),FcntlArg::F_SETFL(OFlag::O_NONBLOCK)).map_err(|e|e.to_string())?;
        }
        let mut a=File::from(first.master);
        let mut b=File::from(second.master);
        let slaves=(first.slave,second.slave);
        let stop=Arc::new(AtomicBool::new(false));
        let state=Arc::new(Mutex::new(PairState::Running));
        let worker_stop=stop.clone();
        let worker_state=state.clone();
        let worker=thread::Builder::new().name(format!("pty-pair:{name}")).spawn(move || {
            // Keep both slaves alive while clients open/close them, avoiding EIO.
            let _keepalive=slaves;
            let mut a_to_b=VecDeque::new();
            let mut b_to_a=VecDeque::new();
            let result=(||->std::io::Result<()> {
                while !worker_stop.load(Ordering::Acquire) {
                    let moved=relay(&mut a,&mut b,&mut a_to_b)? | relay(&mut b,&mut a,&mut b_to_a)?;
                    if !moved { thread::sleep(Duration::from_millis(2)); }
                }
                Ok(())
            })();
            *worker_state.lock().unwrap_or_else(|e|e.into_inner())=match result { Ok(())=>PairState::Stopped, Err(error)=>PairState::Fault(error.to_string()) };
        }).map_err(|e|e.to_string())?;
        Ok(Self { name:name.into(),paths,raw_paths,state,stop,worker:Some(worker),links })
    }
    pub fn state(&self)->PairState { self.state.lock().unwrap_or_else(|e|e.into_inner()).clone() }
    pub fn stop(&mut self) {
        self.stop.store(true,Ordering::Release);
        if let Some(worker)=self.worker.take() { if worker.join().is_err() { log::error!("PTY pair {} worker panicked",self.name); } }
        self.links.clear();
        *self.state.lock().unwrap_or_else(|e|e.into_inner())=PairState::Stopped;
    }
}
impl Drop for VirtualPair { fn drop(&mut self) { self.stop(); } }

fn relay(source:&mut File,destination:&mut File,pending:&mut VecDeque<u8>)->std::io::Result<bool> {
    const LIMIT:usize=65536;
    let mut moved=false;
    if pending.len()<LIMIT {
        let mut buffer=[0;4096];
        let capacity=(LIMIT-pending.len()).min(buffer.len());
        match source.read(&mut buffer[..capacity]) {
            Ok(0)=>return Err(std::io::Error::new(std::io::ErrorKind::UnexpectedEof,"PTY closed")),
            Ok(count)=>{ pending.extend(&buffer[..count]); moved=true; }
            Err(e) if matches!(e.kind(),std::io::ErrorKind::WouldBlock|std::io::ErrorKind::Interrupted)=>{},
            Err(e)=>return Err(e),
        }
    }
    if !pending.is_empty() {
        let (bytes,_)=pending.as_slices();
        match destination.write(bytes) {
            Ok(0)=>return Err(std::io::Error::new(std::io::ErrorKind::WriteZero,"PTY write returned zero")),
            Ok(count)=>{ pending.drain(..count); moved=true; }
            Err(e) if matches!(e.kind(),std::io::ErrorKind::WouldBlock|std::io::ErrorKind::Interrupted)=>{},
            Err(e)=>return Err(e),
        }
    }
    Ok(moved)
}
