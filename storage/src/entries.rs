//! playlist entries: the unit two devices merge a playlist by.
//!
//! every entry (one song at one place of a playlist; the same song may be in
//! a playlist several times) has an id unique within its playlist and the
//! time it was added. A removed entry leaves its id behind
//! (`playlist_removed`), so a merge never brings it back. Merging two copies
//! of a playlist:
//! - the entries: every entry either copy has, minus every entry either copy
//!   removed;
//! - the same song added on both sides: an entry of song `s` only one copy
//!   has (added there since the copies last met) is paired with one only
//!   the other copy has of the same song — oldest with oldest — and only the
//!   one added later (`added_at`, ties broken by the id) stays, at its place;
//!   the other one is removed. Unpaired entries stay, so a song added twice
//!   on one side is still there twice;
//! - the order: the copy changed later (`updated_at`, ties broken by the
//!   entry ids) is the base; an entry only the other copy has is put after
//!   the entry it follows there (the nearest one the base has too), or at
//!   the start if nothing the base has comes before it there.
//!
//! the result doesn't depend on which copy is local, so both devices end up
//! with the same playlist.

use std::collections::{HashMap, HashSet};

/// one entry of a playlist.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// unique within the playlist.
    pub id: u64,
    /// the song hash.
    pub hash: String,
    /// when the entry was added (unix seconds).
    pub added_at: u64,
}

/// a random id for a new entry.
pub fn new_entry_id() -> u64 {
    fastrand::u64(..)
}

/// the id of an entry that existed before entries had ids (an old db, a
/// packet without ids): the `nth` (from 0) occurrence of song `hash` in
/// playlist `playlist`. Deterministic, so two devices that had the same
/// playlist give its entries the same ids. 64-bit FNV-1a.
pub fn legacy_entry_id(playlist: &str, hash: &str, nth: u32) -> u64 {
    const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;
    let nth = nth.to_le_bytes();
    let bytes = playlist
        .as_bytes()
        .iter()
        .chain(&[0])
        .chain(hash.as_bytes())
        .chain(&[0])
        .chain(&nth);
    bytes.fold(OFFSET, |h, &b| (h ^ b as u64).wrapping_mul(PRIME))
}

/// legacy entries ([`legacy_entry_id`]) for the songs `hashes` of playlist
/// `playlist`, in order, all added at `added_at`.
pub fn legacy_entries(playlist: &str, hashes: &[String], added_at: u64) -> Vec<Entry> {
    let mut seen: HashMap<&str, u32> = HashMap::new();
    hashes
        .iter()
        .map(|h| {
            let nth = seen.entry(h.as_str()).or_insert(0);
            let id = legacy_entry_id(playlist, h, *nth);
            *nth += 1;
            Entry {
                id,
                hash: h.clone(),
                added_at,
            }
        })
        .collect()
}

/// what [`merge`] makes of two copies.
#[derive(Debug, PartialEq, Eq)]
pub struct Merged {
    /// the live entries, in order.
    pub entries: Vec<Entry>,
    /// the entries dropped as the same song added on the other side later;
    /// to be remembered as removed.
    pub dropped: Vec<u64>,
}

/// merges two copies of a playlist (see the module docs): `local` / `remote`
/// — the live entries in order with the time of the last change of their
/// copy, `removed` — the entries removed in either copy. A repeated entry id
/// is taken once.
pub fn merge(
    local: &[Entry],
    local_at: u64,
    remote: &[Entry],
    remote_at: u64,
    removed: &HashSet<u64>,
) -> Merged {
    let dropped = same_song_added_on_both(local, remote, removed);
    let removed: HashSet<u64> = removed.iter().chain(&dropped).copied().collect();

    let ids = |l: &[Entry]| l.iter().map(|e| e.id).collect::<Vec<_>>();
    let (base, other) = match (local_at, ids(local)) >= (remote_at, ids(remote)) {
        true => (local, remote),
        false => (remote, local),
    };
    let mut seen = HashSet::new();
    let mut out: Vec<Entry> = base
        .iter()
        .filter(|e| !removed.contains(&e.id) && seen.insert(e.id))
        .cloned()
        .collect();
    // where the next entry only `other` has goes: after the last entry of
    // `other` the base has too.
    let mut cursor = 0;
    for e in other {
        if removed.contains(&e.id) {
            continue;
        }
        match seen.insert(e.id) {
            true => {
                out.insert(cursor, e.clone());
                cursor += 1;
            }
            false => {
                if let Some(i) = out.iter().position(|o| o.id == e.id) {
                    cursor = i + 1;
                }
            }
        }
    }
    Merged {
        entries: out,
        dropped,
    }
}

/// the entries of a song added on both sides that lose to the other side's
/// later one (see the module docs).
fn same_song_added_on_both(local: &[Entry], remote: &[Entry], removed: &HashSet<u64>) -> Vec<u64> {
    let (mine, theirs) = (only_here(local, remote, removed), only_here(remote, local, removed));
    let mut dropped = Vec::new();
    for (song, list) in &mine {
        let Some(other) = theirs.get(song) else { continue };
        dropped.extend(list.iter().zip(other).map(|(a, b)| a.min(b).1));
    }
    dropped
}

/// song -> (added_at, id) of the live entries only `mine` has, oldest first.
fn only_here<'a>(
    mine: &'a [Entry],
    theirs: &[Entry],
    removed: &HashSet<u64>,
) -> HashMap<&'a str, Vec<(u64, u64)>> {
    let theirs: HashSet<u64> = theirs.iter().map(|e| e.id).collect();
    let mut by_song: HashMap<&str, Vec<(u64, u64)>> = HashMap::new();
    for e in mine.iter().filter(|e| !removed.contains(&e.id) && !theirs.contains(&e.id)) {
        by_song.entry(e.hash.as_str()).or_default().push((e.added_at, e.id));
    }
    for list in by_song.values_mut() {
        list.sort_unstable();
        list.dedup();
    }
    by_song
}

#[cfg(test)]
mod tests {
    use super::*;

    /// entries `(id, song, added_at)`.
    fn entries(list: &[(u64, &str, u64)]) -> Vec<Entry> {
        list.iter()
            .map(|&(id, hash, added_at)| Entry { id, hash: hash.into(), added_at })
            .collect()
    }

    /// entries of distinct songs (the song is the id), added at 0.
    fn distinct(ids: &[u64]) -> Vec<Entry> {
        ids.iter()
            .map(|&id| Entry { id, hash: id.to_string(), added_at: 0 })
            .collect()
    }

    fn ids(m: &Merged) -> Vec<u64> {
        m.entries.iter().map(|e| e.id).collect()
    }

    fn removed(ids: &[u64]) -> HashSet<u64> {
        ids.iter().copied().collect()
    }

    #[test]
    fn adds_from_both_sides_are_kept() {
        // both started from [1, 2]; local added 3 at the end, remote 4 after 1.
        let merged = merge(&distinct(&[1, 2, 3]), 10, &distinct(&[1, 4, 2]), 20, &removed(&[]));
        assert_eq!(ids(&merged), vec![1, 4, 2, 3]);
        assert!(merged.dropped.is_empty());
    }

    #[test]
    fn a_removed_entry_does_not_come_back() {
        // remote removed 2, local still has it (and added 3).
        let merged = merge(&distinct(&[1, 2, 3]), 30, &distinct(&[1]), 20, &removed(&[2]));
        assert_eq!(ids(&merged), vec![1, 3]);
    }

    #[test]
    fn duplicates_are_separate_entries() {
        // the same song twice: two entries with their own ids.
        let local = entries(&[(1, "x", 0), (2, "x", 0)]);
        let remote = entries(&[(1, "x", 0), (2, "x", 0), (3, "x", 5)]);
        let merged = merge(&local, 1, &remote, 2, &removed(&[1]));
        assert_eq!(ids(&merged), vec![2, 3]);
        assert!(merged.dropped.is_empty(), "3 was added on one side only");
    }

    #[test]
    fn the_same_song_added_on_both_sides_is_kept_once_at_the_later_place() {
        // both had [1, 2]; local added x at the end at 30, remote added x at
        // the front at 40 (and its copy changed earlier).
        let local = entries(&[(1, "a", 0), (2, "b", 0), (10, "x", 30)]);
        let remote = entries(&[(20, "x", 40), (1, "a", 0), (2, "b", 0)]);
        let merged = merge(&local, 50, &remote, 45, &removed(&[]));
        assert_eq!(ids(&merged), vec![20, 1, 2]);
        assert_eq!(merged.dropped, vec![10]);
        // the other way round: the local one is later.
        let local = entries(&[(1, "a", 0), (2, "b", 0), (10, "x", 60)]);
        let merged = merge(&local, 50, &remote, 45, &removed(&[]));
        assert_eq!(ids(&merged), vec![1, 2, 10]);
        assert_eq!(merged.dropped, vec![20]);
    }

    #[test]
    fn a_song_added_twice_on_one_side_and_once_on_the_other_stays_twice() {
        let local = entries(&[(1, "x", 10), (2, "x", 20)]);
        let remote = entries(&[(3, "x", 15)]);
        let merged = merge(&local, 1, &remote, 2, &removed(&[]));
        // the oldest pair (1 at 10, 3 at 15) meets: 3 stays; 2 is unpaired.
        assert_eq!(merged.dropped, vec![1]);
        let mut left = ids(&merged);
        left.sort();
        assert_eq!(left, vec![2, 3]);
    }

    #[test]
    fn a_song_both_sides_already_share_is_not_paired() {
        // x is common (entry 1); remote adds x once more: a second copy.
        let local = entries(&[(1, "x", 0)]);
        let remote = entries(&[(1, "x", 0), (2, "x", 9)]);
        let merged = merge(&local, 1, &remote, 2, &removed(&[]));
        assert_eq!(ids(&merged), vec![1, 2]);
        assert!(merged.dropped.is_empty());
    }

    #[test]
    fn the_later_copy_gives_the_order() {
        let local = distinct(&[1, 2, 3]);
        let remote = distinct(&[3, 2, 1]);
        assert_eq!(ids(&merge(&local, 5, &remote, 9, &removed(&[]))), vec![3, 2, 1]);
        assert_eq!(ids(&merge(&local, 9, &remote, 5, &removed(&[]))), vec![1, 2, 3]);
    }

    #[test]
    fn the_result_does_not_depend_on_the_side() {
        let a = entries(&[(1, "a", 0), (5, "x", 3), (2, "b", 0), (6, "c", 1)]);
        let b = entries(&[(7, "x", 4), (1, "a", 0), (2, "b", 0), (8, "x", 4), (3, "d", 0)]);
        let gone = removed(&[3, 6]);
        for (at_a, at_b) in [(1, 2), (2, 1), (4, 4)] {
            let (ab, ba) = (merge(&a, at_a, &b, at_b, &gone), merge(&b, at_b, &a, at_a, &gone));
            assert_eq!(ab.entries, ba.entries);
            let (mut d1, mut d2) = (ab.dropped, ba.dropped);
            d1.sort();
            d2.sort();
            assert_eq!(d1, d2);
        }
    }

    #[test]
    fn a_repeated_id_is_taken_once() {
        let merged = merge(&distinct(&[1, 1, 2]), 1, &distinct(&[2, 2]), 0, &removed(&[]));
        assert_eq!(ids(&merged), vec![1, 2]);
    }

    #[test]
    fn legacy_ids_are_stable_and_count_occurrences() {
        let hashes: Vec<String> = ["aa", "bb", "aa"].map(String::from).to_vec();
        let list = legacy_entries("mix", &hashes, 7);
        assert_eq!(list, legacy_entries("mix", &hashes, 7));
        assert_eq!(list[0].id, legacy_entry_id("mix", "aa", 0));
        assert_eq!(list[2].id, legacy_entry_id("mix", "aa", 1));
        assert_eq!(list[2].added_at, 7);
        assert_ne!(list[0].id, list[2].id);
        assert_ne!(list[0].id, legacy_entry_id("other", "aa", 0));
        // fixed values: the id must never change between builds / platforms.
        assert_eq!(legacy_entry_id("", "", 0), 0xd7e4_fcfa_299d_713d);
        assert_eq!(legacy_entry_id("mix", "aa", 1), 0x4623_fb2d_b776_a46e);
    }
}
