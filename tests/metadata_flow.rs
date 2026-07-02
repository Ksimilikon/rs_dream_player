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

    // --- задача 1: правка метаданных (тег файла + бд) ---
    TrackMetadata::write_tags(&a, "Edited Title", &["New Artist".into()]).unwrap();
    db.update_track_meta(id, Some("Edited Title"), Some(&["New Artist".to_string()]))
        .unwrap();

    // тег в файле действительно изменился...
    let on_disk = TrackMetadata::from_path(&a).unwrap();
    assert_eq!(on_disk.title, "Edited Title");
    assert_eq!(on_disk.artist, vec!["New Artist".to_string()]);
    // ...и индекс тоже.
    let reloaded = db.find_track(None, None, Some(id), None).unwrap();
    assert_eq!(reloaded[0].get_metadata().unwrap().title, "Edited Title");

    // --- задача 4 / задача 2: отсутствующий файл выявляется и удаляется ---
    fs::remove_file(&b).unwrap();
    let missing: Vec<_> = db
        .track_paths()
        .unwrap()
        .into_iter()
        .filter(|(_, p)| !p.exists())
        .collect();
    assert_eq!(missing.len(), 1);
    assert_eq!(missing[0].1, b);

    // удаляем отсутствующий из индекса (как делают :check и авто-восстановление).
    db.remove_track(missing[0].0).unwrap();
    let after = db.find_track(None, None, None, None).unwrap();
    assert_eq!(after.len(), 1, "only the present track remains");
    assert_eq!(after[0].get_path(), Some(a.as_path()));
}
