// data classes and constants of the core JNI contract. The constructors are
// called from Rust (`android_jni`) by their JVM signatures: changing a
// parameter's type or order breaks the binding at runtime.

package dev.dreamplayer.core

/** packet format version written by default (`codec::Version::V0_1`). */
const val PACKET_VERSION: Int = 1

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
    const val COVER = 2
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
}
