// data classes and constants of the core JNI contract. The constructors are
// called from Rust (`android_jni`) by their JVM signatures: changing a
// parameter's type or order breaks the binding at runtime.

package dev.dreamplayer.core

/** packet format version written by default (`codec::Version::V0_1`). */
const val PACKET_VERSION: Int = 1

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
}
