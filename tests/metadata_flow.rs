//! end-to-end проверка логики новых команд на реальных mp3-семплах: индексация
//! каталога, правка метаданных (тег файла + бд), проверка наличия файлов и
//! удаление отсутствующих из индекса. Мы дублируем ровно те шаги, что выполняет
//! менеджер оркестратора (`src/orchestrator/manager.rs`), но через публичные API
//! `storage` и `audio_structs`, без TUI/каналов.

use std::fs;
use std::path::PathBuf;

use audio_structs::track_metadata::TrackMetadata;
use storage::{DB_FILE_NAME, Db};
use tempfile::tempdir;

/// каталог с реальными mp3-семплами в репозитории.
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
    // копируем два семпла в рабочий каталог, чтобы правки не трогали репозиторий.
    let a = work.path().join("a.mp3");
    let b = work.path().join("b.mp3");
    fs::copy(&src, &a).unwrap();
    fs::copy(&src, &b).unwrap();

    let db = Db::init(work.path().join(DB_FILE_NAME)).unwrap();

    // --- задача 3: сканирование каталога наполняет индекс ---
    db.index_dir(work.path()).unwrap();
    let tracks = db.find_track(None, None, None, None).unwrap();
    assert_eq!(tracks.len(), 2, "both samples should be indexed");

    // берём id трека, лежащего в файле a.mp3.
    let id = tracks
        .iter()
        .find(|t| t.get_path() == Some(a.as_path()))
        .and_then(|t| t.index_id())
        .expect("indexed track has an id");

    // --- задача 1: правка метаданных (тег файла + бд), включая альбом и жанры ---
    TrackMetadata::write_tags(
        &a,
        "Edited Title",
        &["New Artist".into()],
        Some("New Album"),
        &["Rock".into(), "Indie".into()],
    )
    .unwrap();
    db.update_track_meta(id, Some("Edited Title"), Some(&["New Artist".to_string()]))
        .unwrap();
    db.set_track_album(id, Some("New Album")).unwrap();
    db.set_track_genres(id, &["Rock".into(), "Indie".into()])
        .unwrap();

    // тег в файле действительно изменился...
    let on_disk = TrackMetadata::from_path(&a).unwrap();
    assert_eq!(on_disk.title, "Edited Title");
    assert_eq!(on_disk.artist, vec!["New Artist".to_string()]);
    assert_eq!(on_disk.album.as_deref(), Some("New Album"));
    assert_eq!(on_disk.genres, vec!["Rock".to_string(), "Indie".to_string()]);
    // ...и индекс тоже (жанры из бд возвращаются отсортированными по имени).
    let reloaded = db.find_track(None, None, Some(id), None).unwrap();
    let meta = reloaded[0].get_metadata().unwrap();
    assert_eq!(meta.title, "Edited Title");
    assert_eq!(meta.album.as_deref(), Some("New Album"));
    assert_eq!(meta.genres, vec!["Indie".to_string(), "Rock".to_string()]);

    // db-only метки: цвет и текстовая метка.
    db.set_track_color(id, Some("red")).unwrap();
    db.set_track_label(id, Some("fav")).unwrap();
    let track = &db.find_track(None, None, Some(id), None).unwrap()[0];
    assert_eq!(track.color.as_deref(), Some("red"));
    assert_eq!(track.user_label.as_deref(), Some("fav"));

    // --- задача 2/3: отсутствующий файл помечается invalid (а не удаляется) ---
    fs::remove_file(&b).unwrap();
    let b_id = db
        .track_paths()
        .unwrap()
        .into_iter()
        .find(|(_, p)| !p.exists())
        .map(|(id, _)| id)
        .expect("missing file detected");
    db.set_track_invalid(b_id, true).unwrap();
    // трек всё ещё в индексе, но помечен недействительным.
    let b_track = &db.find_track(None, None, Some(b_id), None).unwrap()[0];
    assert!(b_track.invalid);
    assert_eq!(db.find_track(None, None, None, None).unwrap().len(), 2);

    // --- задача 3: задать недействительному треку новый путь (дедуп) ---
    // возвращаем файл под новым именем и переуказываем путь.
    let b_new = work.path().join("b_renamed.mp3");
    fs::copy(&a, &b_new).unwrap();
    db.reassign_path(b_id, &b_new).unwrap();
    let b_track = &db.find_track(None, None, Some(b_id), None).unwrap()[0];
    assert!(!b_track.invalid);
    assert_eq!(b_track.get_path(), Some(b_new.as_path()));

    // --- задача 3: purge недействительных ---
    // снова ломаем b и чистим.
    fs::remove_file(&b_new).unwrap();
    db.set_track_invalid(b_id, true).unwrap();
    assert_eq!(db.remove_invalid().unwrap(), 1);
    let after = db.find_track(None, None, None, None).unwrap();
    assert_eq!(after.len(), 1, "only the present track remains");
    assert_eq!(after[0].get_path(), Some(a.as_path()));
}
