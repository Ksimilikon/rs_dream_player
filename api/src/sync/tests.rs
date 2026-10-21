//! two devices in one process: real sessions, dbs and keys, bytes pumped
//! between them as a transport would.

use std::{cell::RefCell, fs, path::Path};

use audio_structs::{playlist::Playlist, track_virtual::TrackVirtual};
use tempfile::TempDir;
use vpair::DeviceManager;

use super::frame::{self, Tag};
use super::session::{Env, Event, Kind, SyncSession};
use super::*;
use crate::cdc::{FFICdcStatus, Store};
use storage::traits::indexator::Indexator;

/// one device: its db, keys, music directory and connection entry.
struct Device {
    dir: TempDir,
    db: Db,
    mgr: RefCell<DeviceManager>,
    con: u64,
}

impl Store for Device {
    fn with<R>(&self, f: impl FnOnce(&Db) -> Result<R, FFICdcStatus>) -> Result<R, FFICdcStatus> {
        f(&self.db)
    }
}

impl Env for Device {
    type Store = Self;

    fn run<R>(
        &self,
        f: impl FnOnce(&mut DeviceManager, &Db) -> Result<R, WireStatus>,
    ) -> Result<R, WireStatus> {
        f(&mut self.mgr.borrow_mut(), &self.db)
    }

    fn store(&self) -> &Self {
        self
    }
}

impl Device {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let db = Db::init(dir.path().join("db.sqlite")).unwrap();
        let mut mgr = DeviceManager::new(&dir.path().join("keys")).unwrap();
        let con = mgr.add();
        mgr.connect(con).unwrap();
        fs::create_dir_all(dir.path().join("music")).unwrap();
        Self {
            dir,
            db,
            mgr: RefCell::new(mgr),
            con,
        }
    }

    fn music(&self) -> std::path::PathBuf {
        self.dir.path().join("music")
    }

    /// a playlist of songs copied from `test_data` under the given names.
    fn playlist(&self, name: &str, updated: u64, songs: &[(&str, &str)]) {
        let tracks = songs
            .iter()
            .map(|(file, sample)| {
                let path = self.music().join(file);
                let sample = Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("../test_data")
                    .join(sample);
                fs::copy(sample, &path).unwrap();
                TrackVirtual::from_file(path, false).unwrap()
            })
            .collect();
        let mut p = Playlist::from_tracks(tracks);
        p.set_name(name.into());
        p.set_updated_at(updated);
        self.db.save_playlist(p).unwrap();
    }

    /// playlist name -> its song hashes.
    fn playlists(&self) -> Vec<(String, Vec<String>)> {
        self.db
            .get_playlist_hashes(None)
            .unwrap()
            .into_iter()
            .map(|p| (p.name, p.hashes))
            .collect()
    }

    fn hashes(&self) -> Vec<String> {
        let mut h: Vec<String> = self
            .db
            .get_track_hashes(None)
            .unwrap()
            .into_iter()
            .map(|(_, h, _)| h)
            .collect();
        h.sort();
        h
    }

    /// what a frontend does on `Import`: writes every file (over `replace`
    /// if given) and reports the paths.
    fn import(&self, s: &mut SyncSession) {
        let pending = s.import_files().unwrap();
        let paths: Vec<Option<std::path::PathBuf>> = pending
            .session
            .planned()
            .zip(&pending.replace)
            .map(|((_, name, data), replace)| {
                let path = replace.clone().unwrap_or_else(|| self.music().join(name));
                fs::write(&path, data).unwrap();
                Some(path)
            })
            .collect();
        s.import_done(self, &paths).unwrap();
    }
}

/// pumps bytes and imports until both sides are quiet; returns the events
/// of the host and the client.
fn pump(
    host: &Device,
    hs: &mut SyncSession,
    client: &Device,
    cs: &mut SyncSession,
) -> (Vec<Event>, Vec<Event>) {
    let (mut he, mut ce) = (Vec::new(), Vec::new());
    for _ in 0..10_000 {
        for (dev, s, events) in [(host, &mut *hs, &mut he), (client, &mut *cs, &mut ce)] {
            while let Some(ev) = s.poll_event() {
                if ev == Event::Import {
                    dev.import(s);
                }
                events.push(ev);
            }
        }
        let (to_client, to_host) = (hs.take_output(), cs.take_output());
        if to_client.is_empty() && to_host.is_empty() {
            break;
        }
        cs.feed(client, &to_client);
        hs.feed(host, &to_host);
    }
    (he, ce)
}

fn sessions(host: &Device, client: &Device) -> (SyncSession, SyncSession) {
    (
        SyncSession::new(host, host.con, true, Kind::Sync),
        SyncSession::new(client, client.con, false, Kind::Sync),
    )
}

#[test]
fn both_libraries_end_up_with_everything() {
    let host = Device::new();
    let client = Device::new();
    host.playlist("host mix", 10, &[("a.mp3", "2.mp3")]);
    client.playlist("client mix", 20, &[("b.mp3", "was_wallen_wir_trinken.mp3")]);
    // a song outside every playlist is not synced.
    let solo = host.music().join("solo.mp3");
    fs::copy(host.music().join("a.mp3"), &solo).unwrap();
    host.db
        .save_track(TrackVirtual::from_file(solo, false).unwrap())
        .unwrap();

    let (mut hs, mut cs) = sessions(&host, &client);
    let (he, ce) = pump(&host, &mut hs, &client, &mut cs);

    assert!(hs.is_finished() && cs.is_finished());
    assert_eq!(he.last(), Some(&Event::Done), "{he:?}");
    assert_eq!(ce.last(), Some(&Event::Done), "{ce:?}");
    assert!(matches!(he[0], Event::Paired { .. }));
    assert_eq!(host.hashes().len(), 3);
    assert_eq!(client.hashes().len(), 2, "the solo song stayed on the host");
    let mut hp = host.playlists();
    let mut cp = client.playlists();
    hp.sort();
    cp.sort();
    assert_eq!(hp, cp);
    // both directions moved one song and one playlist.
    assert!(ce.contains(&Event::Received(library::PortionStats {
        songs: 1,
        playlists: 0
    })));
    assert!(he.contains(&Event::Received(library::PortionStats {
        songs: 1,
        playlists: 0
    })));

    // a second sync has nothing to move.
    let (mut hs, mut cs) = {
        host.mgr.borrow_mut().disconnect(host.con).unwrap();
        host.mgr.borrow_mut().connect(host.con).unwrap();
        client.mgr.borrow_mut().disconnect(client.con).unwrap();
        client.mgr.borrow_mut().connect(client.con).unwrap();
        sessions(&host, &client)
    };
    let (he, ce) = pump(&host, &mut hs, &client, &mut cs);
    assert_eq!(he.last(), Some(&Event::Done));
    assert!(
        !ce.iter()
            .any(|e| matches!(e, Event::Received(_) | Event::Import)),
        "{ce:?}"
    );
}

#[test]
fn same_file_name_replaces_the_local_song() {
    let host = Device::new();
    let client = Device::new();
    host.playlist("mix", 10, &[("song.mp3", "2.mp3")]);
    // the client has its own copy under the same name (another hash).
    client.playlist("mine", 5, &[("song.mp3", "2.mp3")]);

    let (mut hs, mut cs) = sessions(&host, &client);
    let (he, _) = pump(&host, &mut hs, &client, &mut cs);
    assert_eq!(he.last(), Some(&Event::Done), "{he:?}");

    // the client's file took the host hash; its playlist follows it.
    assert_eq!(client.hashes(), host.hashes());
    assert_eq!(client.hashes().len(), 1);
    let mine = client
        .playlists()
        .into_iter()
        .find(|(n, _)| n == "mine")
        .unwrap();
    assert_eq!(mine.1, host.hashes());
    assert_eq!(
        fs::read_dir(client.music()).unwrap().count(),
        1,
        "no duplicate file"
    );
}

#[test]
fn newer_local_playlist_is_kept() {
    let host = Device::new();
    let client = Device::new();
    host.playlist("mix", 10, &[("a.mp3", "2.mp3")]);
    client.playlist("mix", 50, &[("b.mp3", "was_wallen_wir_trinken.mp3")]);

    let (mut hs, mut cs) = sessions(&host, &client);
    pump(&host, &mut hs, &client, &mut cs);

    // the client's newer "mix" won on both sides; the host's older version
    // (and its song) didn't reach the client.
    let client_mix = client.playlists();
    assert_eq!(host.playlists(), client_mix);
    assert_eq!(client.hashes().len(), 1);
    assert_eq!(client_mix[0].1, client.hashes());
}

#[test]
fn unknown_tag_is_answered_as_invalid() {
    let host = Device::new();
    let mut hs = SyncSession::new(&host, host.con, true, Kind::Sync);
    let mut bad = frame::header(Tag::Hello, WireStatus::Ok, 0).to_vec();
    bad[5] = 0xEE;
    hs.feed(&host, &bad);

    let out = hs.take_output();
    let mut r = frame::Reader::default();
    r.push(&out);
    assert!(matches!(
        r.next_frame().unwrap(),
        Some(frame::Frame::Known {
            tag: Tag::Ack,
            status: WireStatus::InvalidPacket,
            ..
        })
    ));
    assert!(hs.is_finished());
    assert_eq!(
        hs.poll_event(),
        Some(Event::Failed {
            status: WireStatus::InvalidPacket,
            by_peer: false
        })
    );
}

#[test]
fn foreign_key_mid_session_fails_decryption() {
    let host = Device::new();
    let client = Device::new();
    let stranger = Device::new();
    host.playlist("mix", 10, &[("a.mp3", "2.mp3")]);

    let (mut hs, mut cs) = sessions(&host, &client);
    // the client's hello reaches the host; the host's index goes to a
    // stranger that paired with the host key but has another key itself.
    hs.feed(&host, &cs.take_output());
    let to_client = hs.take_output();
    let mut ss = SyncSession::new(&stranger, stranger.con, false, Kind::Sync);
    let _ = ss.take_output();
    ss.feed(&stranger, &to_client);
    let events: Vec<Event> = std::iter::from_fn(|| ss.poll_event()).collect();
    assert!(
        events.contains(&Event::Failed {
            status: WireStatus::DecryptFailed,
            by_peer: false
        }),
        "{events:?}"
    );
}

#[test]
fn cancel_tells_the_peer() {
    let host = Device::new();
    let client = Device::new();
    let (mut hs, mut cs) = sessions(&host, &client);
    hs.feed(&host, &cs.take_output());
    cs.cancel(&client);
    hs.feed(&host, &cs.take_output());
    assert!(hs.is_finished());
    let events: Vec<Event> = std::iter::from_fn(|| hs.poll_event()).collect();
    assert!(events.contains(&Event::Failed {
        status: WireStatus::Cancelled,
        by_peer: true
    }));
}

#[test]
fn ffi_before_init() {
    let mut s = ptr::dangling_mut();
    assert_eq!(
        unsafe { sync_new(0, true, &mut s) },
        FFISyncStatus::NotInitialized
    );
    assert!(s.is_null());
    assert_eq!(
        unsafe { sync_feed(ptr::null_mut(), ptr::null(), 0) },
        FFISyncStatus::NullPointer
    );
    assert!(unsafe { sync_finished(ptr::null_mut()) });
}

// ─────────────────────────── one-way send ───────────────────────────

fn send_plan(dev: &Device, what: Selection, flags: Flags) -> Plan {
    Plan::selection(&dev.db, &what, flags).unwrap()
}

/// `sender` sends to `receiver`; either may be the host.
fn push(
    sender: &Device,
    receiver: &Device,
    sender_is_host: bool,
    what: Selection,
    flags: Flags,
    force: bool,
) -> (Vec<Event>, Vec<Event>) {
    let mut ss = SyncSession::new(
        sender,
        sender.con,
        sender_is_host,
        Kind::Send(send_plan(sender, what, flags)),
    );
    let mut rs = SyncSession::new(
        receiver,
        receiver.con,
        !sender_is_host,
        Kind::Receive { force },
    );
    let (se, re) = match sender_is_host {
        true => pump(sender, &mut ss, receiver, &mut rs),
        false => {
            let (re, se) = pump(receiver, &mut rs, sender, &mut ss);
            (se, re)
        }
    };
    assert!(ss.is_finished() && rs.is_finished());
    (se, re)
}

#[test]
fn sends_a_playlist_one_way() {
    let a = Device::new();
    let b = Device::new();
    a.playlist("mix", 10, &[("a.mp3", "2.mp3")]);
    a.playlist("other", 10, &[("o.mp3", "was_wallen_wir_trinken.mp3")]);
    b.playlist("mine", 10, &[("b.mp3", "was_wallen_wir_trinken.mp3")]);

    // the client sends; nothing comes back.
    let (se, re) = push(
        &a,
        &b,
        false,
        Selection::Playlists(vec!["mix".into()]),
        Flags::ALL,
        false,
    );
    assert_eq!(se.last(), Some(&Event::Done), "{se:?}");
    assert_eq!(re.last(), Some(&Event::Done), "{re:?}");

    let names: Vec<String> = b.playlists().into_iter().map(|(n, _)| n).collect();
    assert!(names.contains(&"mix".to_string()) && !names.contains(&"other".to_string()));
    assert_eq!(b.hashes().len(), 2);
    assert_eq!(a.hashes().len(), 2, "the sender got nothing");
}

#[test]
fn send_without_audio_carries_only_playlists() {
    let a = Device::new();
    let b = Device::new();
    a.playlist("mix", 10, &[("a.mp3", "2.mp3")]);
    let flags = Flags {
        audio: false,
        names: true,
        covers: true,
    };
    let (se, _) = push(
        &a,
        &b,
        true,
        Selection::Playlists(vec!["mix".into()]),
        flags,
        false,
    );
    assert_eq!(se.last(), Some(&Event::Done));
    // the playlist refers to a song the receiver doesn't have.
    assert_eq!(b.playlists(), a.playlists());
    assert!(b.hashes().is_empty());
}

#[test]
fn send_tracks_without_playlists() {
    let a = Device::new();
    let b = Device::new();
    a.playlist("mix", 10, &[("a.mp3", "2.mp3")]);
    let id = a.db.get_track_hashes(None).unwrap()[0].0;
    push(&a, &b, true, Selection::Tracks(vec![id]), Flags::ALL, false);
    assert_eq!(b.hashes(), a.hashes());
    assert!(b.playlists().is_empty());
}

#[test]
fn receiver_force_decides_on_outdated_playlists() {
    let a = Device::new();
    let b = Device::new();
    a.playlist("mix", 10, &[("a.mp3", "2.mp3")]);
    b.playlist("mix", 50, &[("b.mp3", "was_wallen_wir_trinken.mp3")]);
    let what = || Selection::Playlists(vec!["mix".into()]);

    let (se, _) = push(&a, &b, true, what(), Flags::ALL, false);
    assert!(
        se.contains(&Event::PeerRejected(WireStatus::Rejected)),
        "{se:?}"
    );
    assert_ne!(b.playlists(), a.playlists());

    for dev in [&a, &b] {
        let mut mgr = dev.mgr.borrow_mut();
        mgr.disconnect(dev.con).unwrap();
        mgr.connect(dev.con).unwrap();
    }
    let (se, _) = push(&a, &b, true, what(), Flags::ALL, true);
    assert!(
        !se.iter().any(|e| matches!(e, Event::PeerRejected(_))),
        "{se:?}"
    );
    assert_eq!(b.playlists(), a.playlists());
}

#[test]
fn modes_that_dont_fit_are_refused() {
    let a = Device::new();
    let b = Device::new();
    let mut hs = SyncSession::new(&a, a.con, true, Kind::Sync);
    let plan = send_plan(&b, Selection::Library, Flags::ALL);
    let mut cs = SyncSession::new(&b, b.con, false, Kind::Send(plan));
    let (he, ce) = pump(&a, &mut hs, &b, &mut cs);
    assert!(he.contains(&Event::Failed {
        status: WireStatus::ModeMismatch,
        by_peer: false
    }));
    assert!(ce.contains(&Event::Failed {
        status: WireStatus::ModeMismatch,
        by_peer: true
    }));
}

#[test]
fn unknown_selection_is_not_found() {
    let a = Device::new();
    let missing = Plan::selection(
        &a.db,
        &Selection::Playlists(vec!["nope".into()]),
        Flags::ALL,
    );
    assert!(matches!(missing, Err(PlanError::NotFound)));
    let missing = Plan::selection(&a.db, &Selection::Tracks(vec![42]), Flags::ALL);
    assert!(matches!(missing, Err(PlanError::NotFound)));
}

/// a new pair of sessions over the same devices (a second sync).
fn again(host: &Device, client: &Device) -> (SyncSession, SyncSession) {
    for d in [host, client] {
        d.mgr.borrow_mut().disconnect(d.con).unwrap();
        d.mgr.borrow_mut().connect(d.con).unwrap();
    }
    sessions(host, client)
}

fn deleted_names(d: &Device) -> Vec<String> {
    d.db.get_deleted_playlists(None).unwrap().into_iter().map(|p| p.name).collect()
}

#[test]
fn a_deletion_reaches_the_other_device() {
    let host = Device::new();
    let client = Device::new();
    host.playlist("mix", 10, &[("a.mp3", "2.mp3")]);
    let (mut hs, mut cs) = sessions(&host, &client);
    pump(&host, &mut hs, &client, &mut cs);
    assert_eq!(client.playlists().len(), 1);

    host.db.remove_playlist("mix").unwrap();
    let (mut hs, mut cs) = again(&host, &client);
    let (he, ce) = pump(&host, &mut hs, &client, &mut cs);
    assert_eq!(he.last(), Some(&Event::Done), "{he:?}");
    assert_eq!(ce.last(), Some(&Event::Done), "{ce:?}");

    assert!(client.playlists().is_empty(), "deleted on the client too");
    assert_eq!(deleted_names(&client), vec!["mix".to_string()]);
    // the song stays (songs are deleted physically, by hand).
    assert_eq!(client.hashes().len(), 1);

    // nothing left to move afterwards.
    let (mut hs, mut cs) = again(&host, &client);
    let (_, ce) = pump(&host, &mut hs, &client, &mut cs);
    assert!(!ce.iter().any(|e| matches!(e, Event::Received(_))), "{ce:?}");
}

#[test]
fn a_later_change_beats_a_deletion() {
    let host = Device::new();
    let client = Device::new();
    host.playlist("mix", 10, &[("a.mp3", "2.mp3")]);
    // deleted on the host at 100, changed on the client at 500.
    host.db
        .save_import(
            &[],
            &[],
            &[],
            &[],
            &[storage::db::DeletedPlaylistRow {
                name: "mix".into(),
                deleted_at: 100,
            }],
        )
        .unwrap();
    client.playlist("mix", 500, &[("b.mp3", "was_wallen_wir_trinken.mp3")]);

    let (mut hs, mut cs) = sessions(&host, &client);
    let (he, _) = pump(&host, &mut hs, &client, &mut cs);
    assert_eq!(he.last(), Some(&Event::Done), "{he:?}");

    // the client's version came back on the host; nobody keeps the deletion.
    assert_eq!(host.playlists(), client.playlists());
    assert_eq!(host.playlists().len(), 1);
    assert!(deleted_names(&host).is_empty());
    assert!(deleted_names(&client).is_empty());
}

/// the sync row of the only song with the file `name` on `d`.
fn song_row(d: &Device, name: &str) -> storage::db::TrackSyncRow {
    let hashes: Vec<String> = d
        .db
        .get_track_hashes(None)
        .unwrap()
        .into_iter()
        .filter(|(_, _, p)| p.file_name().unwrap() == name)
        .map(|(_, h, _)| h)
        .collect();
    d.db.get_track_sync(&hashes).unwrap().remove(0)
}

#[test]
fn changed_files_and_metadata_reach_the_other_side() {
    let host = Device::new();
    let client = Device::new();
    host.playlist(
        "mix",
        10,
        &[("a.mp3", "2.mp3"), ("b.mp3", "was_wallen_wir_trinken.mp3")],
    );
    let (mut hs, mut cs) = sessions(&host, &client);
    let (he, _) = pump(&host, &mut hs, &client, &mut cs);
    assert_eq!(he.last(), Some(&Event::Done), "{he:?}");
    assert_eq!(client.hashes().len(), 2);

    // on the host: the file of a.mp3 changes (e.g. an embedded cover), the
    // db title of b.mp3 is edited.
    let a = song_row(&host, "a.mp3");
    let b = song_row(&host, "b.mp3");
    fs::copy(host.music().join("b.mp3"), host.music().join("a.mp3")).unwrap();
    host.db.set_track_file_changed(a.id).unwrap();
    host.db.set_track_meta(b.id, Some("Edited"), None).unwrap();

    let (mut hs, mut cs) = again(&host, &client);
    let (he, ce) = pump(&host, &mut hs, &client, &mut cs);
    assert_eq!(he.last(), Some(&Event::Done), "{he:?}");
    // one file and one metadata update.
    assert!(
        ce.contains(&Event::Received(library::PortionStats {
            songs: 1,
            playlists: 0
        })),
        "{ce:?}"
    );

    // the client file of a.mp3 was written over, not duplicated.
    let ca = song_row(&client, &a.path.file_name().unwrap().to_string_lossy());
    assert_eq!(fs::read(&ca.path).unwrap(), fs::read(host.music().join("a.mp3")).unwrap());
    assert_eq!(fs::read_dir(client.music()).unwrap().count(), 2);
    assert_eq!(ca.updated_at_file, song_row(&host, "a.mp3").updated_at_file);
    // the client got the edited title and its date.
    let cb = song_row(&client, "b.mp3");
    assert_eq!(cb.meta.title, "Edited");
    assert_eq!(cb.updated_at, song_row(&host, "b.mp3").updated_at);

    // both sides agree now: nothing moves in either direction.
    let (mut hs, mut cs) = again(&host, &client);
    let (he, ce) = pump(&host, &mut hs, &client, &mut cs);
    assert_eq!(he.last(), Some(&Event::Done));
    for events in [&he, &ce] {
        assert!(
            !events.iter().any(|e| matches!(e, Event::Received(_) | Event::Import)),
            "{events:?}"
        );
    }
}

#[test]
fn a_newer_playlist_cover_syncs() {
    let host = Device::new();
    let client = Device::new();
    host.playlist("mix", 10, &[("a.mp3", "2.mp3")]);
    let (mut hs, mut cs) = sessions(&host, &client);
    pump(&host, &mut hs, &client, &mut cs);

    let cover = host.music().join("mix.png");
    fs::write(&cover, [0x89, b'P', b'N', b'G', 1, 2, 3]).unwrap();
    assert!(host.db.set_playlist_cover("mix", Some(&cover)).unwrap());

    let (mut hs, mut cs) = again(&host, &client);
    let (he, _) = pump(&host, &mut hs, &client, &mut cs);
    assert_eq!(he.last(), Some(&Event::Done), "{he:?}");
    let got = client.db.get_playlist_hashes(None).unwrap().remove(0);
    let got = got.cover_art.expect("the cover reached the client");
    assert_eq!(fs::read(got).unwrap(), fs::read(&cover).unwrap());
}

/// a valid 1×1 PNG — an image lofty can embed.
const PNG_1X1: &[u8] = &[
    137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 1, 0, 0, 0, 1, 8, 2, 0,
    0, 0, 144, 119, 83, 222, 0, 0, 0, 12, 73, 68, 65, 84, 120, 156, 99, 248, 207, 192, 0, 0, 3, 1,
    1, 0, 201, 254, 146, 239, 0, 0, 0, 0, 73, 69, 78, 68, 174, 66, 96, 130,
];

#[test]
fn a_song_cover_file_syncs_an_embedded_one_stays_in_the_file() {
    let host = Device::new();
    let client = Device::new();
    host.playlist(
        "mix",
        10,
        &[("a.mp3", "2.mp3"), ("b.mp3", "was_wallen_wir_trinken.mp3")],
    );
    let (mut hs, mut cs) = sessions(&host, &client);
    pump(&host, &mut hs, &client, &mut cs);

    // a: a cover file only (gif can't be embedded); b: the same picture as
    // a cover file and embedded into the music file.
    let covers = host.dir.path().join("covers");
    fs::create_dir_all(&covers).unwrap();
    let gif = covers.join("front.gif");
    fs::write(&gif, b"GIF89a\x01\x00\x01\x00").unwrap();
    let png = covers.join("b.png");
    fs::write(&png, PNG_1X1).unwrap();
    let (a, b) = (song_row(&host, "a.mp3"), song_row(&host, "b.mp3"));
    host.db.set_track_cover(a.id, Some(&gif)).unwrap();
    audio_structs::track_metadata::TrackMetadata::write_cover(&b.path, PNG_1X1).unwrap();
    host.db.set_track_cover(b.id, Some(&png)).unwrap();
    host.db.set_track_file_changed(b.id).unwrap();

    let (mut hs, mut cs) = again(&host, &client);
    let (he, _) = pump(&host, &mut hs, &client, &mut cs);
    assert_eq!(he.last(), Some(&Event::Done), "{he:?}");

    // a: the metadata update brought the cover file.
    let ca = song_row(&client, "a.mp3");
    let got = ca.cover_art.expect("the cover file reached the client");
    assert_eq!(fs::read(got).unwrap(), fs::read(&gif).unwrap());
    assert_eq!(ca.updated_at, song_row(&host, "a.mp3").updated_at);
    // b: the newer file carries the picture; no separate cover file.
    let cb = song_row(&client, "b.mp3");
    assert_eq!(cb.cover_art, None);
    assert_eq!(
        audio_structs::track_metadata::TrackMetadata::read_cover(&cb.path).as_deref(),
        Some(PNG_1X1)
    );
    assert!(!client.music().join("b.png").exists());
}
