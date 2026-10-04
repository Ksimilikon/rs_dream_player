//! прогресс-бар воспроизведения: `#` — проигранное, `-` — остаток, в конце
//! `mm:ss/mm:ss`.

/// форматирует секунды в `m:ss`.
pub fn fmt_time(secs: u64) -> String {
    format!("{}:{:02}", secs / 60, secs % 60)
}

/// строит строку прогресс-бара под ширину `width` колонок.
pub fn progress_bar(pos: u64, total: u64, width: usize) -> String {
    let pos = if total > 0 { pos.min(total) } else { pos };
    let time = format!(" {}/{}", fmt_time(pos), fmt_time(total));
    let bar_w = width.saturating_sub(time.len());
    let filled = if total > 0 && bar_w > 0 {
        bar_w * (pos as usize) / (total as usize)
    } else {
        0
    };
    format!(
        "{}{}{time}",
        "#".repeat(filled),
        "-".repeat(bar_w.saturating_sub(filled))
    )
}
