// data classes and constants of the core JNI contract. The constructors are
// called from Rust (`android_jni`) by their JVM signatures: changing a
// parameter's type or order breaks the binding at runtime.

package dev.dreamplayer.core

/**
 * packet format version written by default (`codec::Version::V0_12`: v0.1 +
 * deleted playlists; the songs section carries the song change dates,
 * metadata and cover files, the playlists section — the playlist covers).
 * `2` — v0.11, without the song dates, metadata and covers; `1` — v0.1,
 * without deleted playlists too.
 */
const val PACKET_VERSION: Int = 3

/**
 * result of a call that returns a value: [value] is set only when [status] is
 * 0 (OK) — except where a function documents a null value on OK. [status] is
 * a code of that function's subsystem ([EngineStatus], [IdxStatus]).
 */
data class CoreResult<T>(
    val status: Int,
    val value: T?,
) {
    val isOk: Boolean get() = status == 0
}

/** end-of-track listener; called on the core audio thread. */
fun interface TrackEndListener {
    fun onTrackEnd()
}

/** track metadata (`FFITrackMetadata`). */
data class TrackMeta(
    /** library row id (null — the track is not from the library). */
    val id: Long?,
    val path: String?,
    val title: String,
    val artists: List<String>,
    val album: String?,
    val genres: List<String>,
    val durationS: Long,
    val sampleRate: Int,
    val bitrate: Int,
    val listenCount: Long,
    val coverArt: String?,
    /** saved per-track volume (1.0 — default). */
    val volume: Float,
    /** the file is known-bad, playback is skipped. */
    val invalid: Boolean,
    /** color mark: red/pink/orange/green/blue/cyan/purple (null — none). */
    val color: String?,
    /** free-form user label. */
    val label: String?,
    /** kept on purpose though in no playlist: the orphan collector skips it by default. */
    val ignoreOrphan: Boolean,
)

/** playlist (`FFIPlaylist`). The name is its identifier. */
data class PlaylistInfo(
    val name: String?,
    val trackIds: LongArray,
    val coverArt: String?,
    /** unix seconds. */
    val createdAt: Long?,
    /** unix seconds. */
    val updatedAt: Long?,
)

/** a song of an unpacked packet. */
data class PacketSong(
    /** 16 bytes, big-endian (the same order as the hex hash in the db). */
    val hash: ByteArray,
    val name: String?,
    val data: ByteArray?,
    /** cover file name (v0.12+); null without a cover / `withCovers = false`. */
    val coverName: String?,
    /** cover file not embedded into [data]'s file (v0.12+). */
    val cover: ByteArray?,
)

/** a playlist of an unpacked packet. */
data class PacketPlaylist(
    val name: String?,
    /** song hashes (16 bytes each) in playlist order. */
    val tracks: List<ByteArray>,
    val createdAt: Long?,
    val updatedAt: Long?,
    val coverName: String?,
    val cover: ByteArray?,
)

data class Packet(
    val version: Int,
    val songs: List<PacketSong>,
    val playlists: List<PacketPlaylist>,
)

/** result of [Native.cdcDecode]: `packet` is set only for `CdcStatus.OK`. */
data class DecodeResult(
    val status: Int,
    val packet: Packet?,
)

/** a file the frontend has to write during import. */
data class ImportFile(
    /** [ImportFileKind]. */
    val kind: Int,
    /** suggested name: safe on any OS, always with an extension; not unique. */
    val name: String,
    val size: Long,
)

/** result of [Native.cdcImportPrepare]: `session` is 0 unless `CdcStatus.OK`. */
data class ImportPrepared(
    val status: Int,
    val session: Long,
    val files: List<ImportFile>,
)

object ImportFileKind {
    const val SONG = 1
    /** playlist cover. */
    const val COVER = 2
    /** song cover (not embedded into the audio file), v0.12+. */
    const val SONG_COVER = 3
}

/** status codes of the `cdc*` functions (`api::cdc::FFICdcStatus`). */
object CdcStatus {
    const val OK = 0

    // 1..=99 — codec (packet) errors.
    const val BAD_START_WORD = 1
    const val UNSUPPORTED_VERSION = 2
    const val UNEXPECTED_EOF = 3
    const val TRAILING_BYTES = 4
    const val INVALID_UTF8 = 5
    const val TOO_MANY_ROWS = 6
    const val NAME_TOO_LONG = 7
    const val DATA_TOO_LONG = 8
    const val DUPLICATE_SECTION = 9
    const val COUNT_MISMATCH = 10

    // 100..=199 — import decisions.
    /** the packet is older than local playlists; apply only with force. */
    const val OUTDATED = 100

    // 200.. — core errors.
    const val NULL_POINTER = 200
    const val NO_DATABASE = 201
    const val TRACK_NOT_FOUND = 202
    const val INVALID_HASH = 203
    const val READ_FAILED = 204
    const val PLAYLIST_NOT_FOUND = 205
    const val WRITE_FAILED = 206
    const val INVALID_ARGUMENT = 207
    const val NOT_INITIALIZED = 208
    /** the core panicked; see [Native.coreLastPanic]. */
    const val PANIC = 209
}

/** status codes of [Native.init] (`api::FFIInitStatus`). */
object InitStatus {
    const val OK = 0
    const val NULL_POINTER = 1
    const val INVALID_UTF8 = 2
    /** already initialized; the first instance stays (not a failure). */
    const val ALREADY_INITIALIZED = 3
    const val DB_FAILED = 4
    const val AUDIO_FAILED = 5
    /** the core panicked; see [Native.coreLastPanic]. */
    const val PANIC = 6
}

/** status codes of the `engine*` functions (`api::player::FFIEngineStatus`). */
object EngineStatus {
    const val OK = 0
    const val NOT_INITIALIZED = 1
    const val NULL_POINTER = 2
    const val INVALID_UTF8 = 3
    const val NO_DATABASE = 4
    const val READ_FAILED = 5
    const val TRACK_NOT_FOUND = 6
    /** the track file can't be read / is not audio. */
    const val LOAD_FAILED = 7
    /** the engine can't decode the track. */
    const val PLAYBACK_FAILED = 8
    const val SEEK_FAILED = 9
    /** no output device with this name, or it failed to start. */
    const val DEVICE_FAILED = 10
    /** the value was applied, but the config couldn't be saved. */
    const val CONFIG_WRITE_FAILED = 11
    /** the core panicked; see [Native.coreLastPanic]. */
    const val PANIC = 12
}

/** status codes of the `idx*` functions (`api::idx::FFIIdxStatus`). */
object IdxStatus {
    const val OK = 0
    const val NOT_INITIALIZED = 1
    const val NULL_POINTER = 2
    const val INVALID_UTF8 = 3
    const val NO_DATABASE = 4
    /** no track / playlist with this id / path / name. */
    const val NOT_FOUND = 5
    const val READ_FAILED = 6
    const val WRITE_FAILED = 7
    /** indexing failed (unreadable directory or db error). */
    const val INDEX_FAILED = 8
    /** the core panicked; see [Native.coreLastPanic]. */
    const val PANIC = 9
    /** an argument is out of range (e.g. an unknown [OrphanFilter]). */
    const val INVALID_ARGUMENT = 10
}

/** which songs in no playlist [Native.idxGetOrphanTracks] lists (`api::idx::FFIOrphanFilter`). */
object OrphanFilter {
    /** not marked "ignore orphanhood" (the collector's default). */
    const val NOT_IGNORED = 0
    /** only the marked ones. */
    const val IGNORED = 1
    const val ALL = 2
}

// ── devices (con_*) ──

/** a connection entry (`FFIConDevice`). */
data class ConDevice(
    val id: Long,
    /** [ConState]. */
    val state: Int,
    /** [PairFailure], `NONE` unless `state` is `PAIR_FAILED`. */
    val failure: Int,
    /** [ConProcess]. */
    val process: Int,
    /** db id once paired. */
    val pairedId: Long?,
    /** 32 bytes, known after pairing. */
    val pubKey: ByteArray?,
    /** `Unknown` until known. */
    val name: String,
)

/** a device this one has paired with (`FFIConPaired`). */
data class PairedDevice(
    val id: Long,
    val pubKey: ByteArray,
    /** user-given, for viewing only. */
    val name: String,
)

object ConState {
    const val UNCONNECTED = 0
    const val CONNECTED = 1
    /** keys are exchanged; ready to receive / send. */
    const val PAIRED = 2
    const val PAIR_FAILED = 3
}

object PairFailure {
    const val NONE = 0
    const val REJECTED = 1
    const val TIMEOUT = 2
    const val ERROR = 3
    /** the key differs from the one saved for this device. */
    const val KEY_MISMATCH = 4
    /** this device is already paired over another entry. */
    const val ALREADY_PAIRED = 5
}

object ConProcess {
    const val FREE = 0
    const val RECEIVING = 1
    const val SENDING = 2
}

/** status codes of the `con*` functions (`api::con::FFIConStatus`). */
object ConStatus {
    const val OK = 0
    const val NOT_INITIALIZED = 1
    const val NULL_POINTER = 2
    const val INVALID_UTF8 = 3
    const val INVALID_ARGUMENT = 4
    const val NO_DATABASE = 5
    /** the key pair of this device can't be loaded / created. */
    const val KEYS_FAILED = 6
    const val UNKNOWN_DEVICE = 7
    const val UNKNOWN_PAIRED_DEVICE = 8
    /** the transition is not allowed in the current state. */
    const val WRONG_STATE = 9
    /** the device is already receiving or sending. */
    const val BUSY = 10
    const val DB_FAILED = 11
    /** the core panicked; see [Native.coreLastPanic]. */
    const val PANIC = 12
    const val INVALID_PACKET = 13
    const val WEAK_KEY = 14
    const val DECRYPT_FAILED = 15
    const val TOO_LONG = 16
}

// ── library sync (sync_*) ──

/** what [Native.syncNewSend] sends. */
object SyncWhat {
    const val LIBRARY = 0
    const val PLAYLISTS = 1
    const val TRACKS = 2
}

/** a session event (`FFISyncEvent`). */
data class SyncEvent(
    /** [SyncEventKind]. */
    val kind: Int,
    /** [WireStatus] (`PEER_REJECTED`, `FAILED`). */
    val status: Int,
    /** the failure came from the peer. */
    val byPeer: Boolean,
    /** `PAIRED`: the db id of the peer. */
    val pairedId: Long,
    /** `SENT` / `RECEIVED`: the size of the data portion. */
    val songs: Int,
    val playlists: Int,
)

object SyncEventKind {
    const val NONE = 0
    /** keys are exchanged ([SyncEvent.pairedId]). */
    const val PAIRED = 1
    /** files to write: [Native.syncImportFiles] -> [Native.syncImportDone]. */
    const val IMPORT = 2
    /** the peer accepted a data portion. */
    const val SENT = 3
    /** a received data portion is saved. */
    const val RECEIVED = 4
    /** the peer didn't accept a portion; the session goes on. */
    const val PEER_REJECTED = 5
    const val DONE = 6
    /** the session is over with an error. */
    const val FAILED = 7
}

/** a file of a received data portion; the bytes go with [Native.syncImportWriteFile]. */
data class SyncFile(
    /** [ImportFileKind]. */
    val kind: Int,
    /** suggested name: safe on any OS, with an extension; not unique. */
    val name: String,
    val size: Long,
    /** a local song with the same file name to write over (null — write anywhere). */
    val replace: String?,
)

/** status codes of the `sync*` functions (`api::sync::FFISyncStatus`). */
object SyncStatus {
    const val OK = 0
    const val NOT_INITIALIZED = 1
    const val NULL_POINTER = 2
    /** e.g. the number of import paths differs from the number of files. */
    const val INVALID_ARGUMENT = 3
    const val NO_DATABASE = 4
    const val KEYS_FAILED = 5
    const val UNKNOWN_DEVICE = 6
    /** the entry is not `CONNECTED`, or no import is waiting. */
    const val WRONG_STATE = 7
    const val INVALID_UTF8 = 8
    /** the core panicked; see [Native.coreLastPanic]. */
    const val PANIC = 9
    /** some playlist / track of a send selection is not in the db. */
    const val NOT_FOUND = 10
    const val DB_FAILED = 11

    /** JNI only: the descriptor passed from Kotlin couldn't be written. */
    const val IO_FAILED = -1
}

/** protocol statuses carried by `Ack` / `Abort` (`api::sync::frame::WireStatus`). */
object WireStatus {
    const val OK = 0
    const val INVALID_PACKET = 1
    const val UNEXPECTED = 2
    /** not from / not for this pair of devices. */
    const val DECRYPT_FAILED = 3
    /** e.g. the key differs from the saved one. */
    const val PAIR_FAILED = 4
    /** a data portion was not accepted (its playlists are older). */
    const val REJECTED = 5
    const val WRITE_FAILED = 6
    const val BUSY = 7
    const val CANCELLED = 8
    const val INTERNAL = 9
    /** the two sides run different modes (sync / send / receive). */
    const val MODE_MISMATCH = 10
}
