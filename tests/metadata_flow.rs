//! end-to-end check of the new command logic on real mp3 samples: directory
//! indexing, metadata editing (file tag + db), file presence checks and
//! removal of missing files from the index. We repeat exactly the steps the
//! orchestrator manager (`src/orchestrator/manager.rs`) performs, but through
//! the public `storage` and `audio_structs` APIs, without the TUI / channels.

use std::fs;
use std::path::PathBuf;

use audio_structs::track_metadata::TrackMetadata;
use storage::{DB_FILE_NAME, Db};
use tempfile::tempdir;

/// directory with real mp3 samples in the repository.
fn test_data() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("test_data")
}

#[test]
fn scan_edit_and_check_flow() {
    let src = test_data().join("2.mp3");
    if !src.exists() {
        eprintln!("skipping: fixture missing at {}", src.display());
        return;
    }

    let work = tempdir().unwrap();
    // copy two samples into a work directory, so edits don't touch the repository.
    let a = work.path().join("a.mp3");
    let b = work.path().join("b.mp3");
    fs::copy(&src, &a).unwrap();
    fs::copy(&src, &b).unwrap();

    let db = Db::init(work.path().join(DB_FILE_NAME)).unwrap();

    // --- task 3: scanning a directory fills the index ---
    db.index_dir(work.path()).unwrap();
    let tracks = db.get_tracks(None, None, None, None).unwrap();
    assert_eq!(tracks.len(), 2, "both samples should be indexed");

    // take the id of the track stored in a.mp3.
    let id = tracks
        .iter()
        .find(|t| t.get_path() == Some(a.as_path()))
        .and_then(|t| t.index_id())
        .expect("indexed track has an id");

    // --- task 1: metadata editing (file tag + db), including album and genres ---
    TrackMetadata::write_tags(
        &a,
        "Edited Title",
        &["New Artist".into()],
        Some("New Album"),
        &["Rock".into(), "Indie".into()],
    )
    .unwrap();
    db.set_track_meta(id, Some("Edited Title"), Some(&["New Artist".to_string()]))
        .unwrap();
    db.set_track_album(id, Some("New Album")).unwrap();
    db.set_track_genres(id, &["Rock".into(), "Indie".into()])
        .unwrap();

    // the tag in the file really changed...
    let on_disk = TrackMetadata::from_path(&a).unwrap();
    assert_eq!(on_disk.title, "Edited Title");
    assert_eq!(on_disk.artist, vec!["New Artist".to_string()]);
    assert_eq!(on_disk.album.as_deref(), Some("New Album"));
    assert_eq!(on_disk.genres, vec!["Rock".to_string(), "Indie".to_string()]);
    // ...and so did the index (genres come back from the db sorted by name).
    let reloaded = db.get_tracks(None, None, Some(id), None).unwrap();
    let meta = reloaded[0].get_metadata().unwrap();
    assert_eq!(meta.title, "Edited Title");
    assert_eq!(meta.album.as_deref(), Some("New Album"));
    assert_eq!(meta.genres, vec!["Indie".to_string(), "Rock".to_string()]);

    // db-only labels: color and text label.
    db.set_track_color(id, Some("red")).unwrap();
    db.set_track_label(id, Some("fav")).unwrap();
    let track = &db.get_tracks(None, None, Some(id), None).unwrap()[0];
    assert_eq!(track.color.as_deref(), Some("red"));
    assert_eq!(track.user_label.as_deref(), Some("fav"));

    // --- task 2/3: a missing file is marked invalid (not removed) ---
    fs::remove_file(&b).unwrap();
    let b_id = db
        .get_track_paths()
        .unwrap()
        .into_iter()
        .find(|(_, p)| !p.exists())
        .map(|(id, _)| id)
        .expect("missing file detected");
    db.set_track_invalid(b_id, true).unwrap();
    // the track is still in the index but marked invalid.
    let b_track = &db.get_tracks(None, None, Some(b_id), None).unwrap()[0];
    assert!(b_track.invalid);
    assert_eq!(db.get_tracks(None, None, None, None).unwrap().len(), 2);

    // --- task 3: give the invalid track a new path (dedup) ---
    // put the file back under a new name and repoint the path.
    let b_new = work.path().join("b_renamed.mp3");
    fs::copy(&a, &b_new).unwrap();
    db.reassign_track_path(b_id, &b_new).unwrap();
    let b_track = &db.get_tracks(None, None, Some(b_id), None).unwrap()[0];
    assert!(!b_track.invalid);
    assert_eq!(b_track.get_path(), Some(b_new.as_path()));

    // --- task 3: purge invalid tracks ---
    // break b again and clean up.
    fs::remove_file(&b_new).unwrap();
    db.set_track_invalid(b_id, true).unwrap();
    assert_eq!(db.remove_invalid_tracks().unwrap(), 1);
    let after = db.get_tracks(None, None, None, None).unwrap();
    assert_eq!(after.len(), 1, "only the present track remains");
    assert_eq!(after[0].get_path(), Some(a.as_path()));
}
