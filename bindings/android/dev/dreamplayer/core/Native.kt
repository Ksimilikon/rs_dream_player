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
//   `IdxStatus`, `cdc*` — `CdcStatus`, `con*` — `ConStatus`, `sync*` —
//   `SyncStatus` codes.

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

    /** songs that are in no playlist (cleanup candidates), by title; [filter] — [OrphanFilter]. */
    @JvmStatic external fun idxGetOrphanTracks(filter: Int): CoreResult<List<TrackMeta>>

    /** the mark "ignore orphanhood": the song is kept though it is in no playlist. */
    @JvmStatic external fun idxSetTrackIgnoreOrphan(id: Long, ignore: Boolean): Int

    /** one track by id, or by [path] if it is not null (`NOT_FOUND` — none). */
    @JvmStatic external fun idxGetTrack(id: Long, path: String?): CoreResult<TrackMeta>

    /**
     * indexes [dir] (null — the music directory from [init]): every file is
     * re-read. A frontend that compares the folder with [idxGetTracks] itself
     * should use [idxAddTracks] / [idxSetTracksInvalid] instead.
     */
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

    /**
     * embeds the picture at [imagePath] (png / jpg only) as the front cover
     * into the tags of the track's audio file; the db is not changed (call
     * [idxSetTrackFileChanged] after). `NOT_FOUND` — no such track / no file,
     * `INVALID_ARGUMENT` — the picture is unreadable or of another format,
     * `WRITE_FAILED` — the audio file couldn't be written.
     */
    @JvmStatic external fun idxEmbedTrackCover(id: Long, imagePath: String): Int

    /** new file path after a rename on disk. */
    @JvmStatic external fun idxSetTrackPath(id: Long, path: String): Int

    @JvmStatic external fun idxSetTrackInvalid(id: Long, invalid: Boolean): Int
    /** the invalid mark of several tracks in one transaction (unknown ids are skipped). */
    @JvmStatic external fun idxSetTracksInvalid(ids: LongArray, invalid: Boolean): Int

    /**
     * adds song files found by the frontend in one transaction (tags are read
     * by the core; a path already in the library is updated). Non-music /
     * unreadable files are skipped; `value` — the number written.
     */
    @JvmStatic external fun idxAddTracks(paths: Array<String>): CoreResult<Long>

    @JvmStatic external fun idxSetTrackVolume(id: Long, volume: Float): Int

    /**
     * dates a change of the song file itself (tags, embedded cover, ...) made
     * by the frontend: sync sends the file again and writes it over the copy
     * on the other device. Call it after changing a file.
     */
    @JvmStatic external fun idxSetTrackFileChanged(id: Long): Int
    @JvmStatic external fun idxRemoveTrack(id: Long): Int

    /** removes every invalid track; `value` — the number removed. */
    @JvmStatic external fun idxRemoveInvalidTracks(): CoreResult<Long>

    /**
     * creates / updates playlist [name] from [trackIds] in this order (ids
     * missing from the library are skipped).
     */
    @JvmStatic external fun idxSavePlaylist(name: String, trackIds: LongArray, cover: String?): Int

    /**
     * cover of playlist [name] without touching its tracks (null clears it);
     * dates the playlist as changed, so the cover syncs. `NOT_FOUND` — no
     * such playlist.
     */
    @JvmStatic external fun idxSetPlaylistCover(name: String, cover: String?): Int

    /**
     * deletes a playlist softly: it disappears, its deletion mark syncs to
     * other devices (and goes into library packets); saving the name again
     * brings it back.
     */
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

    /** tracks by id (without playlists, with their covers) into [fd]. */
    @JvmStatic external fun cdcPackTracks(
        version: Int,
        ids: LongArray,
        withAudio: Boolean,
        withNames: Boolean,
        withCovers: Boolean,
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

    // ── devices (con_*), `ConStatus` codes ──
    //
    // a connection entry lives while the core runs (`id`); a paired device
    // also has a db id that survives restarts. Pairing and encryption happen
    // inside a sync session.

    /** public key of this device (32 bytes, X25519) — show its fingerprint. */
    @JvmStatic external fun conGetPubKey(): CoreResult<ByteArray>

    /** a new entry for an unknown device (`ConState.UNCONNECTED`); `value` — its id. */
    @JvmStatic external fun conAdd(): CoreResult<Long>

    /** the transport is up: `UNCONNECTED` -> `CONNECTED`. */
    @JvmStatic external fun conConnect(id: Long): Int

    /** the transport is down: any -> `UNCONNECTED`, a running transfer is dropped. */
    @JvmStatic external fun conDisconnect(id: Long): Int

    /** drops an entry in any state. */
    @JvmStatic external fun conRemove(id: Long): Int

    /** every entry with its state, ordered by id. */
    @JvmStatic external fun conGetDevices(): CoreResult<List<ConDevice>>

    /** every device this one has paired with (from the db), for viewing. */
    @JvmStatic external fun conGetPaired(): CoreResult<List<PairedDevice>>

    /** names a paired device (for viewing only). */
    @JvmStatic external fun conRename(pairedId: Long, name: String): Int

    /** forgets a paired device; `WRONG_STATE` while it is paired over an entry. */
    @JvmStatic external fun conForget(pairedId: Long): Int

    // ── library sync (sync_*), `SyncStatus` codes ──
    //
    // the core runs the protocol, Kotlin carries the bytes over any transport
    // (the side that listened is the host). Per connection: `conAdd` +
    // `conConnect`, a `syncNew*` session, then until `syncFinished`:
    // `syncWriteOutput` into the transport, handle `syncPollEvent` (on
    // `SyncEventKind.IMPORT`: `syncImportFiles` -> `syncImportWriteFile` for
    // each -> `syncImportDone`), `syncFeed` what was received (a closed stream
    // — `syncCancel`). Then `syncFree`, `conDisconnect` + `conRemove`.

    /** a two-way library sync over a `CONNECTED` entry; `value` — the session handle. */
    @JvmStatic external fun syncNew(con: Long, host: Boolean): CoreResult<Long>

    /**
     * a one-way send of a selection with the export flags ([SyncWhat]; [names]
     * for playlists, [ids] for tracks); the other side runs [syncNewReceive].
     * `NOT_FOUND` — some playlist / track is not in the db.
     */
    @JvmStatic external fun syncNewSend(
        con: Long,
        host: Boolean,
        what: Int,
        names: Array<String>?,
        ids: LongArray?,
        withAudio: Boolean,
        withNames: Boolean,
        withCovers: Boolean,
    ): CoreResult<Long>

    /** a one-way receive; [force] — accept playlists older than the local ones. */
    @JvmStatic external fun syncNewReceive(con: Long, host: Boolean, force: Boolean): CoreResult<Long>

    /** frees the session (0 is ignored); cancel an unfinished one first. */
    @JvmStatic external fun syncFree(session: Long)

    /** bytes received from the peer: `data[0 until len]`. */
    @JvmStatic external fun syncFeed(session: Long, data: ByteArray, len: Int): Int

    /**
     * writes everything to send into [fd] (the transport socket); `IO_FAILED`
     * — it couldn't be written.
     */
    @JvmStatic external fun syncWriteOutput(session: Long, fd: Int): Int

    /** the next event, null — none now. */
    @JvmStatic external fun syncPollEvent(session: Long): SyncEvent?

    /** `true` — the session is over (done or failed). */
    @JvmStatic external fun syncFinished(session: Long): Boolean

    /** the files of the waiting import (after `IMPORT`); `WRONG_STATE` — none is waiting. */
    @JvmStatic external fun syncImportFiles(session: Long): CoreResult<List<SyncFile>>

    /** writes import file [index] into [fd]; `IO_FAILED` — it couldn't be written. */
    @JvmStatic external fun syncImportWriteFile(session: Long, index: Int, fd: Int): Int

    /**
     * the import files are written: `paths[i]` — the final path of file `i`
     * (null — not written). The portion is saved and the peer answered.
     */
    @JvmStatic external fun syncImportDone(session: Long, paths: Array<String?>): Int

    /** breaks the session off; the `Abort` for the peer goes out with [syncWriteOutput]. */
    @JvmStatic external fun syncCancel(session: Long): Int
}
