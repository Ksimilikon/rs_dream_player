//! общий стейт интерфейса и типы сообщений между TUI и оркестратором.

/// краткая информация о треке для отображения.
#[derive(Clone)]
pub struct TrackInfo {
    pub title: String,
    pub artists: String,
    /// персональная громкость трека (0.0..=1.0).
    pub volume: f32,
}

/// плейлист в списке (имя + его треки для предпросмотра на вкладке playlists).
pub struct PlaylistEntry {
    pub name: String,
    pub tracks: Vec<TrackInfo>,
    /// виртуальный плейлист «весь пул песен» (грузится особым путём).
    pub pool: bool,
}

/// команды управления, которые TUI шлёт наружу (в оркестратор).
pub enum Control {
    Next,
    Prev,
    PlayPause,
    /// выбрать и проиграть трек по индексу в текущем плейлисте.
    Select(usize),
    /// загрузить плейлист по имени из бд.
    LoadPlaylist(String),
    /// загрузить виртуальный плейлист со всем пулом песен.
    LoadPool,
    /// громкость текущей песни (0.0..=1.0).
    SongVolume(f32),
    /// общая (мастер-) громкость (0.0..=1.0).
    MasterVolume(f32),
}

/// обновления состояния, которые оркестратор шлёт в TUI.
pub enum Update {
    /// индекс текущего трека сменился.
    NowPlaying(usize),
    /// загружен новый плейлист — список треков и его имя.
    Playlist { name: String, tracks: Vec<TrackInfo> },
}

/// общий стейт, который читают вкладки при отрисовке.
pub struct Model {
    /// все плейлисты с их треками (для вкладки playlists и предпросмотра).
    pub playlists: Vec<PlaylistEntry>,
    /// имя текущего (играющего) плейлиста ("---" для анонимного).
    pub playlist_name: String,
    /// треки текущего (играющего) плейлиста.
    pub tracks: Vec<TrackInfo>,
    /// индекс играющего трека.
    pub current: usize,
    /// общая громкость 0.0..=1.0.
    pub master_vol: f32,
}

impl Model {
    /// громкость текущей песни (или 1.0, если трека нет).
    pub fn song_vol(&self) -> f32 {
        self.tracks.get(self.current).map(|t| t.volume).unwrap_or(1.0)
    }
}
