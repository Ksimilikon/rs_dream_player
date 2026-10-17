// Kotlin side of the core JNI contract. Must match `android_jni` (the Rust
// crate in the core repository) exactly: class / method names, parameter and
// return types. Copy this directory into the frontend as is.
//
// conventions:
// - core failures are values (`false`, `null`, status codes), not exceptions;
//   a JNI-level failure surfaces as a RuntimeException.
// - packets and audio never go through the Java heap: pass file descriptors
//   (`ParcelFileDescriptor.fd`). The descriptor is borrowed — the caller
//   closes it after the call.
// - `cdc*` functions return `CdcStatus` codes.

package dev.dreamplayer.core

object Native {
    init {
        System.loadLibrary("dream_player")
    }

    // ── init ──

    /**
     * initializes the core. Call once before anything else. [context] — any
     * context (its application context is kept): the audio backend needs it.
     */
    @JvmStatic external fun init(
        context: android.content.Context,
        cacheDir: String,
        configDir: String,
        dataDir: String,
        musicDir: String,
        needDb: Boolean,
    )

    /** end-of-track listener (null — none). Called on the core audio thread. */
    @JvmStatic external fun setTrackEndListener(listener: TrackEndListener?)

    // ── playback (engine_*) ──

    /** plays a library track by id. */
    @JvmStatic external fun engineAddTrackId(id: Long): Boolean

    /** plays a file by path (its library row is used if it exists). */
    @JvmStatic external fun engineAddTrackPath(path: String): Boolean

    @JvmStatic external fun enginePlayPause()
    @JvmStatic external fun enginePlay()
    @JvmStatic external fun enginePause()
    @JvmStatic external fun engineStop()
    @JvmStatic external fun engineSetSpeed(speed: Float)

    /** seek to [sec] seconds. */
    @JvmStatic external fun engineSetSeek(sec: Long)

    /** current position in seconds. */
    @JvmStatic external fun engineGetSeek(): Long

    /** duration of the current track in seconds (0 — unknown). */
    @JvmStatic external fun engineGetDuration(): Long

    @JvmStatic external fun engineIsPause(): Boolean
    @JvmStatic external fun engineIsEmpty(): Boolean
    @JvmStatic external fun engineSetVolumeTrack(volume: Float)
    @JvmStatic external fun engineSetVolumeMaster(volume: Float)
    @JvmStatic external fun engineGetVolumeTrack(): Float
    @JvmStatic external fun engineGetVolumeMaster(): Float
    @JvmStatic external fun engineGetOutputDevices(): List<String>
    @JvmStatic external fun engineGetOutputDevice(): String?
    @JvmStatic external fun engineSetOutputDevice(name: String): Boolean

    // ── library (idx_*) ──

    @JvmStatic external fun idxGetPlaylists(): List<PlaylistInfo>
    @JvmStatic external fun idxGetTracks(): List<TrackMeta>

    /** one track by id, or by [path] if it is not null. */
    @JvmStatic external fun idxGetTrack(id: Long, path: String?): TrackMeta?

    /** indexes [dir] (null — the music directory from [init]). */
    @JvmStatic external fun idxIndexDir(dir: String?): Boolean

    /** null field — leave as is; artists are rewritten entirely. */
    @JvmStatic external fun idxSetTrackMeta(id: Long, title: String?, artists: Array<String>?): Boolean

    /** genres are rewritten entirely; null / empty — cleared. */
    @JvmStatic external fun idxSetTrackGenres(id: Long, genres: Array<String>?): Boolean

    // null clears the field.
    @JvmStatic external fun idxSetTrackAlbum(id: Long, album: String?): Boolean
    @JvmStatic external fun idxSetTrackCover(id: Long, coverPath: String?): Boolean
    @JvmStatic external fun idxSetTrackColor(id: Long, color: String?): Boolean
    @JvmStatic external fun idxSetTrackLabel(id: Long, label: String?): Boolean

    /** new file path after a rename on disk. */
    @JvmStatic external fun idxSetTrackPath(id: Long, path: String): Boolean

    @JvmStatic external fun idxSetTrackInvalid(id: Long, invalid: Boolean): Boolean
    @JvmStatic external fun idxSetTrackVolume(id: Long, volume: Float): Boolean
    @JvmStatic external fun idxRemoveTrack(id: Long): Boolean

    /** removes every invalid track; the number removed (-1 on error). */
    @JvmStatic external fun idxRemoveInvalidTracks(): Long

    /** creates / updates playlist [name] from [trackIds] in this order. */
    @JvmStatic external fun idxSavePlaylist(name: String, trackIds: LongArray, cover: String?): Boolean

    @JvmStatic external fun idxRemovePlaylist(name: String): Boolean

    // ── packets (cdc_*) ──

    /** snapshot of the whole library into [fd]. */
    @JvmStatic external fun cdcPackLibrary(
        version: Int,
        withAudio: Boolean,
        withNames: Boolean,
        withCovers: Boolean,
        fd: Int,
    ): Int

    /** tracks by id (without playlists) into [fd]. */
    @JvmStatic external fun cdcPackTracks(
        version: Int,
        ids: LongArray,
        withAudio: Boolean,
        withNames: Boolean,
        fd: Int,
    ): Int

    /** playlists by name (their identifier) with all their songs into [fd]. */
    @JvmStatic external fun cdcPackPlaylists(
        version: Int,
        names: Array<String>,
        withAudio: Boolean,
        withNames: Boolean,
        withCovers: Boolean,
        fd: Int,
    ): Int

    /**
     * unpacks a packet held in memory. Meant for previews with
     * `withAudio = false`: with audio everything lands in the Java heap.
     */
    @JvmStatic external fun cdcDecode(
        data: ByteArray,
        withAudio: Boolean,
        withNames: Boolean,
        withCovers: Boolean,
    ): DecodeResult

    /**
     * import step 1: reads the packet from [fd], checks it and the dates.
     * The library doesn't change. On `CdcStatus.OK` the session must end with
     * [cdcImportCommit] or [cdcImportCancel].
     */
    @JvmStatic external fun cdcImportPrepare(fd: Int, force: Boolean): ImportPrepared

    /** import step 2: writes the bytes of file [index] into [fd]. */
    @JvmStatic external fun cdcImportWriteFile(session: Long, index: Int, fd: Int): Int

    /**
     * import step 3: saves the import. `paths[i]` — final path of file `i`
     * (null — not written). Frees the session in any case; on a non-OK status
     * delete the written files yourself.
     */
    @JvmStatic external fun cdcImportCommit(session: Long, paths: Array<String?>): Int

    /** cancels the import and frees the session (0 is ignored). */
    @JvmStatic external fun cdcImportCancel(session: Long)
}
