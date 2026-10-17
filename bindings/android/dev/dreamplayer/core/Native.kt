// Kotlin side of the core JNI contract. Must match `android_jni` (the Rust
// crate in the core repository) exactly: class / method names, parameter and
// return types. Copy this directory into the frontend as is.
//
// conventions:
// - core failures are status codes, not exceptions: every subsystem has its
//   own set ([InitStatus], [EngineStatus], [IdxStatus], [CdcStatus]; 0 = OK).
//   Calls without a result return the code as `Int`; calls with one return
//   [CoreResult] (`value` is set only for OK). A JNI-level failure surfaces as
//   a RuntimeException.
// - the core never crashes on a panic: the call returns its subsystem's
//   `PANIC` code. Every panic is counted ([Native.corePanicCount]), kept
//   ([Native.coreLastPanic]) and appended to `<dataDir>/core_panic.log`, which
//   also catches panics that do kill the process (core audio threads).
// - packets and audio never go through the Java heap: pass file descriptors
//   (`ParcelFileDescriptor.fd`). The descriptor is borrowed — the caller
//   closes it after the call.
// - `init` returns `InitStatus`, `engine*` — `EngineStatus`, `idx*` —
//   `IdxStatus`, `cdc*` — `CdcStatus` codes.

package dev.dreamplayer.core

object Native {
    init {
        System.loadLibrary("dream_player")
    }

    // ── init ──

    /**
     * initializes the core. Call once before anything else. [context] — any
     * context (its application context is kept): the audio backend needs it.
     * Returns an [InitStatus]; until it is OK every other call returns its
     * `NOT_INITIALIZED`.
     */
    @JvmStatic external fun init(
        context: android.content.Context,
        cacheDir: String,
        configDir: String,
        dataDir: String,
        musicDir: String,
        needDb: Boolean,
    ): Int

    /** end-of-track listener (null — none). Called on the core audio thread. */
    @JvmStatic external fun setTrackEndListener(listener: TrackEndListener?)

    // ── diagnostics ──

    /** number of core panics in this process (caught and uncaught). */
    @JvmStatic external fun corePanicCount(): Long

    /** thread, location and message of the last core panic (null — none). */
    @JvmStatic external fun coreLastPanic(): String?

    // ── playback (engine_*), `EngineStatus` codes ──

    /** plays a library track by id. */
    @JvmStatic external fun engineAddTrackId(id: Long): Int

    /** plays a file by path (its library row is used if it exists). */
    @JvmStatic external fun engineAddTrackPath(path: String): Int

    @JvmStatic external fun enginePlayPause(): Int
    @JvmStatic external fun enginePlay(): Int
    @JvmStatic external fun enginePause(): Int
    @JvmStatic external fun engineStop(): Int
    @JvmStatic external fun engineSetSpeed(speed: Float): Int

    /** seek to [sec] seconds. */
    @JvmStatic external fun engineSetSeek(sec: Long): Int

    /** current position in seconds. */
    @JvmStatic external fun engineGetSeek(): CoreResult<Long>

    /** duration of the current track in seconds (0 — unknown). */
    @JvmStatic external fun engineGetDuration(): CoreResult<Long>

    @JvmStatic external fun engineIsPause(): CoreResult<Boolean>
    @JvmStatic external fun engineIsEmpty(): CoreResult<Boolean>
    @JvmStatic external fun engineSetVolumeTrack(volume: Float): Int

    /** applied and saved; `CONFIG_WRITE_FAILED` — applied, but not saved. */
    @JvmStatic external fun engineSetVolumeMaster(volume: Float): Int
    @JvmStatic external fun engineGetVolumeTrack(): CoreResult<Float>
    @JvmStatic external fun engineGetVolumeMaster(): CoreResult<Float>
    /**
     * output device names; the first one is the system default
     * (`"Default Device"`): its stream follows the system routing, reopen it
     * with [engineSetOutputDevice] after the route changed (headphones).
     */
    @JvmStatic external fun engineGetOutputDevices(): CoreResult<List<String>>

    /** `value` is null on OK too when the device is unknown. */
    @JvmStatic external fun engineGetOutputDevice(): CoreResult<String>
    @JvmStatic external fun engineSetOutputDevice(name: String): Int

    // ── library (idx_*), `IdxStatus` codes ──

    @JvmStatic external fun idxGetPlaylists(): CoreResult<List<PlaylistInfo>>
    @JvmStatic external fun idxGetTracks(): CoreResult<List<TrackMeta>>

    /** one track by id, or by [path] if it is not null (`NOT_FOUND` — none). */
    @JvmStatic external fun idxGetTrack(id: Long, path: String?): CoreResult<TrackMeta>

    /** indexes [dir] (null — the music directory from [init]). */
    @JvmStatic external fun idxIndexDir(dir: String?): Int

    /** null field — leave as is; artists are rewritten entirely. */
    @JvmStatic external fun idxSetTrackMeta(id: Long, title: String?, artists: Array<String>?): Int

    /** genres are rewritten entirely; null / empty — cleared. */
    @JvmStatic external fun idxSetTrackGenres(id: Long, genres: Array<String>?): Int

    // null clears the field.
    @JvmStatic external fun idxSetTrackAlbum(id: Long, album: String?): Int
    @JvmStatic external fun idxSetTrackCover(id: Long, coverPath: String?): Int
    @JvmStatic external fun idxSetTrackColor(id: Long, color: String?): Int
    @JvmStatic external fun idxSetTrackLabel(id: Long, label: String?): Int

    /** new file path after a rename on disk. */
    @JvmStatic external fun idxSetTrackPath(id: Long, path: String): Int

    @JvmStatic external fun idxSetTrackInvalid(id: Long, invalid: Boolean): Int
    @JvmStatic external fun idxSetTrackVolume(id: Long, volume: Float): Int
    @JvmStatic external fun idxRemoveTrack(id: Long): Int

    /** removes every invalid track; `value` — the number removed. */
    @JvmStatic external fun idxRemoveInvalidTracks(): CoreResult<Long>

    /**
     * creates / updates playlist [name] from [trackIds] in this order (ids
     * missing from the library are skipped).
     */
    @JvmStatic external fun idxSavePlaylist(name: String, trackIds: LongArray, cover: String?): Int

    @JvmStatic external fun idxRemovePlaylist(name: String): Int

    // ── packets (cdc_*), `CdcStatus` codes ──

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
