//! one sync session over one connection entry — a state machine without I/O:
//! the frontend feeds it the bytes it received ([`SyncSession::feed`]), sends
//! what it hands out ([`SyncSession::take_output`]), writes the files of an
//! import and reports where ([`SyncSession::import_done`]).
//!
//! the conversation is lock-step: every `Index` / `Want` / `Data` / `Done` is
//! answered with `Ack` before the sender goes on. The listening side is the
//! host:
//! ```text
//! client -> Hello            host -> Hello                    (pairing)
//! round 1, host gives:       host -> Index      client -> Ack
//!                            client -> Want     host -> Ack
//!                            host -> Data ×n    client -> Ack (import result) ×n
//!                            host -> Done       client -> Ack
//! round 2, client gives:     the same with the roles swapped
//! ```
//! a one-way send (`Send` on one side, `Receive` on the other, either of them
//! may be the host) is a single round without comparing libraries: after the
//! hellos the sender goes straight to `Data ×n` + `Done`, sending exactly the
//! chosen selection. The mode is announced in `Hello`; modes that don't fit
//! end the session with `ModeMismatch`.
//! an `Ack` with an error to `Data` is a warning (the next portion follows);
//! to anything else — the end of the session. `Abort` ends it at any point;
//! an unknown tag / status is answered with `Ack(InvalidPacket)`.

use std::{
    collections::{HashMap, VecDeque},
    path::PathBuf,
};

use storage::Db;
use vpair::{DeviceManager, KEY_LEN, VPairError, VPairState, crypt::OVERHEAD};

use super::frame::{self, Frame, Mode, Reader, Tag, WireStatus};
use super::library::{self, Plan, PortionStats, Want};
use crate::cdc::{self, FFICdcStatus, FFIImportFileKind, ImportSession, Store};

/// access to the core for a session (replaced in tests).
pub(crate) trait Env {
    type Store: Store;
    /// runs `f` on the pairing state machine and the db under the core lock.
    fn run<R>(
        &self,
        f: impl FnOnce(&mut DeviceManager, &Db) -> Result<R, WireStatus>,
    ) -> Result<R, WireStatus>;
    /// db access for the import (takes the lock only for the db part).
    fn store(&self) -> &Self::Store;
}

/// what happened, for the frontend.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    /// keys are exchanged; `paired_id` — the device in the db.
    Paired { paired_id: i64 },
    /// a data portion has files to write: [`SyncSession::import_files`],
    /// then [`SyncSession::import_done`].
    Import,
    /// the peer accepted a data portion we sent.
    Sent(PortionStats),
    /// a received data portion is saved.
    Received(PortionStats),
    /// the peer didn't accept a data portion we sent; the session goes on.
    PeerRejected(WireStatus),
    /// both rounds are over.
    Done,
    /// the session is over with an error; `by_peer` — the peer reported it.
    Failed { status: WireStatus, by_peer: bool },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    AwaitHello,
    // giving
    SentIndex,
    AwaitWant,
    SentData,
    SentDone,
    // receiving
    AwaitIndex,
    SentWant,
    AwaitData,
    Importing,
    // the end
    Done,
    Failed,
}

/// a received data portion waiting for the frontend to write its files.
pub struct PendingImport {
    pub(crate) session: ImportSession,
    /// for every file: the local file to write it over — a song with the same
    /// file name is replaced by the received one.
    pub(crate) replace: Vec<Option<PathBuf>>,
}

/// what this side wants from the session.
pub enum Kind {
    /// two-way library sync.
    Sync,
    /// send this plan one way.
    Send(Plan),
    /// accept a one-way send; `force` — import even outdated playlists.
    Receive { force: bool },
}

/// misuse of the session by the frontend.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionError {
    /// no import is waiting.
    NoImport,
    /// the number of paths differs from the number of files.
    PathCount,
}

pub struct SyncSession {
    con: u64,
    host: bool,
    mode: Mode,
    /// import outdated playlists too (`Receive` only).
    force: bool,
    /// the selection of a one-way send, until it starts.
    push: Option<Plan>,
    round: u8,
    state: State,
    reader: Reader,
    out: Vec<u8>,
    events: VecDeque<Event>,
    plan: Option<Plan>,
    /// what the data portion waiting for its `Ack` carries.
    sending: PortionStats,
    import: Option<PendingImport>,
}

impl SyncSession {
    /// a session over the connection entry `con` (it must be `Connected`).
    /// The client starts right away by sending its key.
    pub(crate) fn new(env: &impl Env, con: u64, host: bool, kind: Kind) -> Self {
        let (mode, force, push) = match kind {
            Kind::Sync => (Mode::Sync, false, None),
            Kind::Send(plan) => (Mode::Send, false, Some(plan)),
            Kind::Receive { force } => (Mode::Receive, force, None),
        };
        let mut s = Self {
            con,
            host,
            mode,
            force,
            push,
            round: 1,
            state: State::AwaitHello,
            reader: Reader::default(),
            out: Vec::new(),
            events: VecDeque::new(),
            plan: None,
            sending: PortionStats::default(),
            import: None,
        };
        if !host {
            s.send_hello(env);
        }
        s
    }

    pub fn is_finished(&self) -> bool {
        matches!(self.state, State::Done | State::Failed)
    }

    /// bytes to send to the peer (empty — nothing now).
    pub fn take_output(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.out)
    }

    pub fn poll_event(&mut self) -> Option<Event> {
        self.events.pop_front()
    }

    /// bytes received from the peer.
    pub(crate) fn feed(&mut self, env: &impl Env, bytes: &[u8]) {
        if self.is_finished() {
            return;
        }
        self.reader.push(bytes);
        self.drain(env);
    }

    /// the waiting import, if any.
    pub fn import_files(&self) -> Option<&PendingImport> {
        self.import.as_ref()
    }

    /// the frontend wrote the import files: `paths[i]` — where file `i` went
    /// (`None` — not written). Saves the portion and answers the peer.
    pub(crate) fn import_done(
        &mut self,
        env: &impl Env,
        paths: &[Option<PathBuf>],
    ) -> Result<(), SessionError> {
        let pending = self.import.as_ref().ok_or(SessionError::NoImport)?;
        if paths.len() != pending.replace.len() {
            return Err(SessionError::PathCount);
        }
        let pending = self.import.take().ok_or(SessionError::NoImport)?;
        self.state = State::AwaitData;
        self.commit(env, &pending.session, paths);
        self.drain(env);
        Ok(())
    }

    /// breaks the session off and tells the peer.
    pub(crate) fn cancel(&mut self, env: &impl Env) {
        if !self.is_finished() {
            self.fail(env, WireStatus::Cancelled, false);
        }
    }

    // ───────────────────────────── frames ─────────────────────────────

    fn drain(&mut self, env: &impl Env) {
        while !self.is_finished() && self.state != State::Importing {
            match self.reader.next_frame() {
                Ok(None) => break,
                Ok(Some(Frame::Unknown)) => self.invalid(env),
                Ok(Some(Frame::Known {
                    tag,
                    status,
                    header,
                    payload,
                })) => self.on_frame(env, tag, status, &header, &payload),
                Err(_) => self.fail(env, WireStatus::InvalidPacket, false),
            }
        }
    }

    fn on_frame(
        &mut self,
        env: &impl Env,
        tag: Tag,
        status: WireStatus,
        header: &[u8],
        payload: &[u8],
    ) {
        if tag == Tag::Abort {
            return self.fail(env, status, true);
        }
        if tag != Tag::Ack && status != WireStatus::Ok {
            return self.invalid(env);
        }
        let ok = status == WireStatus::Ok;
        match (self.state, tag) {
            (State::AwaitHello, Tag::Hello) => self.on_hello(env, payload),
            (State::SentIndex, Tag::Ack) if ok => self.state = State::AwaitWant,
            (State::AwaitWant, Tag::Want) => self.on_want(env, header, payload),
            (State::SentData, Tag::Ack) => {
                self.events.push_back(match ok {
                    true => Event::Sent(self.sending),
                    false => Event::PeerRejected(status),
                });
                self.send_next_portion(env);
            }
            (State::SentDone, Tag::Ack) if ok => self.end_round(env),
            (State::AwaitIndex, Tag::Index) => self.on_index(env, header, payload),
            (State::SentWant, Tag::Ack) if ok => self.state = State::AwaitData,
            (State::AwaitData, Tag::Data) => self.on_data(env, header, payload),
            (State::AwaitData, Tag::Done) => {
                self.send(Tag::Ack, WireStatus::Ok, &[]);
                self.end_round(env);
            }
            (_, Tag::Ack) => self.fail(env, status, true),
            _ => self.fail(env, WireStatus::Unexpected, false),
        }
    }

    fn on_hello(&mut self, env: &impl Env, payload: &[u8]) {
        let (Ok(key), Some(peer)) = (
            <[u8; KEY_LEN]>::try_from(payload.get(..KEY_LEN).unwrap_or_default()),
            payload.get(KEY_LEN).copied().and_then(Mode::from_u8),
        ) else {
            return self.invalid(env);
        };
        if payload.len() != KEY_LEN + 1 {
            return self.invalid(env);
        }
        if !self.mode.fits(peer) {
            return self.fail(env, WireStatus::ModeMismatch, false);
        }
        let con = self.con;
        let paired = env.run(|mgr, db| {
            let state = mgr.pair(db, con, &key).map_err(vpair_status)?;
            let id = mgr.device(con).and_then(|d| d.paired_id());
            Ok((state, id))
        });
        match paired {
            Ok((VPairState::Paired, Some(paired_id))) => {
                self.events.push_back(Event::Paired { paired_id });
                if self.host {
                    self.send_hello(env);
                }
                match (self.mode, self.host) {
                    (Mode::Sync, true) => self.start_giving(env),
                    (Mode::Sync, false) => self.start_receiving(env, State::AwaitIndex),
                    // a one-way send is a single (last) round.
                    (Mode::Send, _) => {
                        self.round = 2;
                        self.start_pushing(env);
                    }
                    (Mode::Receive, _) => {
                        self.round = 2;
                        self.start_receiving(env, State::AwaitData);
                    }
                }
            }
            Ok(_) => self.fail(env, WireStatus::PairFailed, false),
            Err(st) => self.fail(env, st, false),
        }
    }

    fn on_index(&mut self, env: &impl Env, header: &[u8], payload: &[u8]) {
        let want = self
            .open(env, header, payload)
            .and_then(|index| env.run(|_, db| library::want(db, &index)))
            .and_then(|want| want.encode());
        match want {
            Ok(want) => {
                self.send(Tag::Ack, WireStatus::Ok, &[]);
                match self.send_sealed(env, Tag::Want, &want) {
                    Ok(()) => self.state = State::SentWant,
                    Err(st) => self.fail(env, st, false),
                }
            }
            Err(st) => self.reject_fatal(env, st),
        }
    }

    fn on_want(&mut self, env: &impl Env, header: &[u8], payload: &[u8]) {
        let plan = self
            .open(env, header, payload)
            .and_then(|want| Want::decode(&want))
            .and_then(|want| env.run(|_, db| Plan::new(db, &want)));
        match plan {
            Ok(plan) => {
                self.send(Tag::Ack, WireStatus::Ok, &[]);
                self.plan = Some(plan);
                self.send_next_portion(env);
            }
            Err(st) => self.reject_fatal(env, st),
        }
    }

    fn on_data(&mut self, env: &impl Env, header: &[u8], payload: &[u8]) {
        let packet = match self.open(env, header, payload) {
            Ok(p) => p,
            Err(st) => return self.reject_fatal(env, st),
        };
        let session = match cdc::prepare(env.store(), &packet, self.force, true) {
            Ok(s) => s,
            Err(st) => return self.ack_import(env, import_status(st)),
        };
        if session.planned().next().is_none() {
            return self.commit(env, &session, &[]);
        }
        // a newer version of a known song goes over its file; a new song
        // with the same file name as a local one replaces it.
        let local = env
            .run(|_, db| db.get_track_paths().map_err(|_| WireStatus::Internal))
            .unwrap_or_default();
        let by_name: HashMap<String, PathBuf> = local
            .into_iter()
            .filter_map(|(_, p)| Some((p.file_name()?.to_string_lossy().into_owned(), p)))
            .collect();
        let replace = session
            .planned()
            .zip(session.replaces())
            .map(|((kind, name, _), known)| match (kind, known) {
                (_, Some(path)) => Some(path.to_path_buf()),
                (FFIImportFileKind::Song, None) => by_name.get(name).cloned(),
                (FFIImportFileKind::Cover | FFIImportFileKind::SongCover, None) => None,
            })
            .collect();
        self.import = Some(PendingImport { session, replace });
        self.state = State::Importing;
        self.events.push_back(Event::Import);
    }

    fn commit(&mut self, env: &impl Env, session: &ImportSession, paths: &[Option<PathBuf>]) {
        let songs = session
            .planned()
            .zip(paths)
            .filter(|((kind, ..), path)| *kind == FFIImportFileKind::Song && path.is_some())
            .count();
        let stats = PortionStats {
            songs: (songs + session.meta_updates()) as u32,
            playlists: (session.playlists.len() + session.deleted.len()) as u32,
        };
        match cdc::commit(env.store(), session, paths) {
            Ok(()) => {
                self.events.push_back(Event::Received(stats));
                self.send(Tag::Ack, WireStatus::Ok, &[]);
            }
            Err(st) => self.ack_import(env, import_status(st)),
        }
    }

    /// answers a data portion that was not saved: a rejected / unwritten
    /// portion is a warning for the peer, anything else ends the session.
    fn ack_import(&mut self, env: &impl Env, status: WireStatus) {
        match status {
            WireStatus::Rejected | WireStatus::WriteFailed => self.send(Tag::Ack, status, &[]),
            _ => self.reject_fatal(env, status),
        }
    }

    // ───────────────────────────── rounds ─────────────────────────────

    fn start_giving(&mut self, env: &impl Env) {
        let con = self.con;
        let index = env.run(|mgr, db| {
            mgr.start_sending(con).map_err(vpair_status)?;
            library::index(db)
        });
        match index.and_then(|index| self.send_sealed(env, Tag::Index, &index)) {
            Ok(()) => self.state = State::SentIndex,
            Err(st) => self.fail(env, st, false),
        }
    }

    /// starts receiving; `next` — what is awaited first (`Index` in a sync,
    /// `Data` in a one-way send).
    fn start_receiving(&mut self, env: &impl Env, next: State) {
        let con = self.con;
        match env.run(|mgr, _| mgr.start_receiving(con).map_err(vpair_status)) {
            Ok(()) => self.state = next,
            Err(st) => self.fail(env, st, false),
        }
    }

    /// starts a one-way send of the chosen selection.
    fn start_pushing(&mut self, env: &impl Env) {
        let con = self.con;
        match env.run(|mgr, _| mgr.start_sending(con).map_err(vpair_status)) {
            Ok(()) => {
                self.plan = self.push.take();
                self.send_next_portion(env);
            }
            Err(st) => self.fail(env, st, false),
        }
    }

    fn send_next_portion(&mut self, env: &impl Env) {
        match self.plan.as_mut().and_then(Plan::next_portion) {
            Some(Ok((portion, stats))) => {
                self.sending = stats;
                match self.send_sealed(env, Tag::Data, &portion) {
                    Ok(()) => self.state = State::SentData,
                    Err(st) => self.fail(env, st, false),
                }
            }
            Some(Err(st)) => self.fail(env, st, false),
            None => {
                self.plan = None;
                self.send(Tag::Done, WireStatus::Ok, &[]);
                self.state = State::SentDone;
            }
        }
    }

    /// a round is over: after the first one the roles swap, after the second
    /// the session is done.
    fn end_round(&mut self, env: &impl Env) {
        self.finish_process(env);
        if self.round == 2 {
            self.state = State::Done;
            self.events.push_back(Event::Done);
            return;
        }
        self.round = 2;
        match self.host {
            true => self.start_receiving(env, State::AwaitIndex),
            false => self.start_giving(env),
        }
    }

    // ───────────────────────────── helpers ─────────────────────────────

    fn send_hello(&mut self, env: &impl Env) {
        match env.run(|mgr, _| Ok(*mgr.pub_crypt())) {
            Ok(key) => {
                let hello = [&key[..], &[self.mode as u8]].concat();
                self.send(Tag::Hello, WireStatus::Ok, &hello);
            }
            Err(st) => self.fail(env, st, false),
        }
    }

    fn send(&mut self, tag: Tag, status: WireStatus, payload: &[u8]) {
        self.out
            .extend_from_slice(&frame::header(tag, status, payload.len()));
        self.out.extend_from_slice(payload);
    }

    /// sends `plain` encrypted for the peer, the header bound to it.
    fn send_sealed(&mut self, env: &impl Env, tag: Tag, plain: &[u8]) -> Result<(), WireStatus> {
        let header = frame::header(tag, WireStatus::Ok, plain.len() + OVERHEAD);
        let con = self.con;
        let sealed = env.run(|mgr, _| {
            mgr.encrypt(con, &header, plain)
                .map_err(|_| WireStatus::Internal)
        })?;
        self.out.extend_from_slice(&header);
        self.out.extend_from_slice(&sealed);
        Ok(())
    }

    fn open(&self, env: &impl Env, header: &[u8], payload: &[u8]) -> Result<Vec<u8>, WireStatus> {
        let con = self.con;
        env.run(|mgr, _| {
            mgr.decrypt(con, header, payload).map_err(|e| match e {
                VPairError::Crypt(_) => WireStatus::DecryptFailed,
                e => vpair_status(e),
            })
        })
    }

    /// answers the last frame with an error and ends the session.
    fn reject_fatal(&mut self, env: &impl Env, status: WireStatus) {
        self.send(Tag::Ack, status, &[]);
        self.fail(env, status, false);
    }

    fn invalid(&mut self, env: &impl Env) {
        self.reject_fatal(env, WireStatus::InvalidPacket);
    }

    /// ends the session; a local failure is reported to the peer by `Abort`.
    fn fail(&mut self, env: &impl Env, status: WireStatus, by_peer: bool) {
        if self.is_finished() {
            return;
        }
        if !by_peer {
            self.send(Tag::Abort, status, &[]);
        }
        self.state = State::Failed;
        self.plan = None;
        self.import = None;
        self.finish_process(env);
        self.events.push_back(Event::Failed { status, by_peer });
    }

    fn finish_process(&self, env: &impl Env) {
        let con = self.con;
        // not paired (yet) / already free — nothing to finish.
        let _ = env.run(|mgr, _| mgr.finish(con).map_err(|_| WireStatus::Internal));
    }
}

fn vpair_status(e: VPairError) -> WireStatus {
    match e {
        VPairError::Busy(_) => WireStatus::Busy,
        VPairError::WrongState(_) => WireStatus::Unexpected,
        VPairError::Crypt(_) => WireStatus::DecryptFailed,
        _ => WireStatus::Internal,
    }
}

/// an import failure -> what the peer is told.
fn import_status(st: FFICdcStatus) -> WireStatus {
    match st as u32 {
        _ if st == FFICdcStatus::Outdated => WireStatus::Rejected,
        _ if st == FFICdcStatus::WriteFailed => WireStatus::WriteFailed,
        // 1..=99 — the packet doesn't decode.
        1..=99 => WireStatus::InvalidPacket,
        _ => WireStatus::Internal,
    }
}
