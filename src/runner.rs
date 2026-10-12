use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread;
use std::time::Duration;

use rusqlite::Connection;

use crate::book::now_ms;
use crate::{Failure, Secret, Spec, due, failed, open, retry_after, started};

const MINUTE: Duration = Duration::from_secs(60);
const HOUR: Duration = Duration::from_secs(3600);

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Changed(pub String);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resync(pub String, pub Option<String>);

#[derive(Debug)]
pub enum Message {
    Resync(Resync),
    Secret(String, Secret),
}

impl From<Resync> for Message {
    fn from(resync: Resync) -> Self {
        Message::Resync(resync)
    }
}

pub trait Source: Send {
    fn name(&self) -> &str;
    fn every(&self) -> Option<Duration>;
    fn pass(&mut self, conn: &mut Connection, id: Option<&str>) -> Result<usize, Failure>;
    fn secret(&mut self, name: &str, value: Secret) {
        let _ = (name, value);
    }
}

pub fn spawn(spec: &Spec, source: Box<dyn Source>, changed: Sender<Changed>) -> Sender<Message> {
    let spec = spec.clone();
    let (asks, inbox) = mpsc::channel::<Message>();
    thread::spawn(move || match open(&spec) {
        Ok(conn) => run(conn, source, &inbox, &changed),
        Err(_) => {
            let _ = changed.send(Changed(source.name().to_owned()));
        }
    });
    asks
}

pub fn spawn_checked(
    spec: &Spec,
    source: Box<dyn Source>,
    changed: Sender<Changed>,
) -> Result<Sender<Message>, String> {
    let conn = open(spec)?;
    let (asks, inbox) = mpsc::channel::<Message>();
    thread::spawn(move || run(conn, source, &inbox, &changed));
    Ok(asks)
}

pub fn pass_once(
    conn: &mut Connection,
    source: &mut dyn Source,
    id: Option<&str>,
) -> Result<usize, Failure> {
    pass_announced(conn, source, id, || true).0
}

fn pass_announced(
    conn: &mut Connection,
    source: &mut dyn Source,
    id: Option<&str>,
    announce: impl FnOnce() -> bool,
) -> (Result<usize, Failure>, bool) {
    let name = source.name().to_owned();
    let _ = started(conn, &name, now_ms());
    if !announce() {
        return (Ok(0), false);
    }
    let outcome = source.pass(conn, id);
    if let Err(failure) = &outcome {
        let _ = failed(conn, &name, failure);
    }
    (outcome, true)
}

fn once(
    conn: &mut Connection,
    source: &mut dyn Source,
    name: &str,
    id: Option<&str>,
    changed: &Sender<Changed>,
) -> Option<()> {
    let (_, ran) = pass_announced(conn, source, id, || {
        changed.send(Changed(name.to_owned())).is_ok()
    });
    if !ran {
        return None;
    }
    changed.send(Changed(name.to_owned())).ok()
}

fn run(
    mut conn: Connection,
    mut source: Box<dyn Source>,
    inbox: &Receiver<Message>,
    changed: &Sender<Changed>,
) {
    let name = source.name().to_owned();
    let wait = source.every().unwrap_or(HOUR).min(MINUTE);
    let held = retry_after(&conn, &name)
        .ok()
        .flatten()
        .is_some_and(|at| now_ms() < at);
    if !held && once(&mut conn, source.as_mut(), &name, None, changed).is_none() {
        return;
    }
    loop {
        let id = match inbox.recv_timeout(wait) {
            Ok(Message::Resync(Resync(_, id))) => id,
            Ok(Message::Secret(key, value)) => {
                source.secret(&key, value);
                None
            }
            Err(RecvTimeoutError::Timeout) => {
                if !source
                    .every()
                    .is_some_and(|every| due(&conn, &name, every, now_ms()))
                {
                    continue;
                }
                None
            }
            Err(RecvTimeoutError::Disconnected) => return,
        };
        if once(&mut conn, source.as_mut(), &name, id.as_deref(), changed).is_none() {
            return;
        }
    }
}
