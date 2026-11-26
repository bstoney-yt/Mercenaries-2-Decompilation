//! The What's New tab: the merged pull requests, newest first, grouped by
//! the day they were merged, from `docs/changelog.toml` (built in, so the
//! page shows offline). That file is made by `tools/changelog` from the
//! pull requests' public titles and descriptions: our own text, no game
//! data. Layout and colours invented.

use egui::{Align2, Rect, Sense, Stroke, StrokeKind, Ui, UiBuilder, pos2, vec2};
use serde::Deserialize;

use super::{App, BAR, BG, DIM, FAINT, GREEN, LINE, PAD, TEXT};
use crate::assets::Role;

/// The changelog as built in.
const CHANGELOG: &str = include_str!("../../../../docs/changelog.toml");

#[derive(Clone, Debug, PartialEq, Deserialize)]
pub(super) struct Entry {
    pub pr: u32,
    /// `YYYY-MM-DD`, the day it was merged (UTC).
    pub date: String,
    pub title: String,
    #[serde(default)]
    pub summary: String,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
pub(super) struct Changelog {
    pub generated: String,
    #[serde(default, rename = "entry")]
    pub entries: Vec<Entry>,
}

pub(super) fn changelog() -> Result<Changelog, String> {
    toml::from_str(CHANGELOG).map_err(|e| e.to_string())
}

/// The entries by day, newest day first, each day's newest first.
pub(super) fn by_day(entries: &[Entry]) -> Vec<(String, Vec<Entry>)> {
    let mut sorted = entries.to_vec();
    sorted.sort_by(|a, b| b.date.cmp(&a.date).then(b.pr.cmp(&a.pr)));
    let mut days: Vec<(String, Vec<Entry>)> = Vec::new();
    for e in sorted {
        match days.last_mut() {
            Some((d, list)) if *d == e.date => list.push(e),
            _ => days.push((e.date.clone(), vec![e])),
        }
    }
    days
}

/// `2026-11-26` as `26 November 2026`.
pub(super) fn day_text(date: &str) -> String {
    const MONTHS: [&str; 12] = [
        "January",
        "February",
        "March",
        "April",
        "May",
        "June",
        "July",
        "August",
        "September",
        "October",
        "November",
        "December",
    ];
    let mut it = date.split('-');
    match (
        it.next().and_then(|y| y.parse::<u32>().ok()),
        it.next().and_then(|m| m.parse::<usize>().ok()),
        it.next().and_then(|d| d.parse::<u32>().ok()),
    ) {
        (Some(y), Some(m @ 1..=12), Some(d)) => format!("{d} {} {y}", MONTHS[m - 1]),
        _ => date.to_owned(),
    }
}

/// `text` cut to fit `room` (by `width`), ending in "..." when cut.
pub(super) fn fit(text: &str, room: f32, width: impl Fn(&str) -> f32) -> String {
    if width(text) <= room {
        return text.to_owned();
    }
    let mut end = text.len();
    while end > 0 {
        end = text[..end].char_indices().last().map_or(0, |(i, _)| i);
        let cut = format!("{}...", text[..end].trim_end());
        if width(&cut) <= room {
            return cut;
        }
    }
    "...".to_owned()
}

impl App {
    pub(super) fn whats_new_tab(&mut self, ui: &mut Ui, rect: Rect) {
        let p = ui.painter_at(rect);
        p.rect_filled(rect, 0.0, BG);
        let log = match changelog() {
            Ok(l) => l,
            Err(e) => {
                self.fonts.text(
                    &p,
                    Role::Body,
                    rect.center(),
                    Align2::CENTER_CENTER,
                    16.0,
                    DIM,
                    &format!("The changelog could not be read: {e}"),
                );
                return;
            }
        };
        let top = Rect::from_min_size(
            pos2(rect.left() + PAD, rect.top() + 20.0),
            vec2(rect.width() - 2.0 * PAD, 64.0),
        );
        p.rect_filled(top, 6.0, BAR);
        p.rect_stroke(top, 6.0, Stroke::new(1.0, LINE), StrokeKind::Inside);
        self.fonts.text(
            &p,
            Role::Heading,
            top.left_center() + vec2(18.0, 0.0),
            Align2::LEFT_CENTER,
            20.0,
            TEXT,
            "WHAT'S NEW",
        );
        let days = by_day(&log.entries);
        let note = format!(
            "{} changes merged; list made {}",
            log.entries.len(),
            day_text(&log.generated)
        );
        self.fonts.text(
            &p,
            Role::Body,
            top.right_center() - vec2(18.0, 0.0),
            Align2::RIGHT_CENTER,
            13.0,
            FAINT,
            &note,
        );
        let list = Rect::from_min_max(
            pos2(rect.left() + PAD, top.bottom() + 16.0),
            rect.max - vec2(PAD, 12.0),
        );
        let mut child = ui.new_child(UiBuilder::new().max_rect(list));
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(&mut child, |ui| {
                for (date, entries) in &days {
                    let (row, _) =
                        ui.allocate_exact_size(vec2(ui.available_width(), 34.0), Sense::hover());
                    let p = ui.painter().clone();
                    self.fonts.text(
                        &p,
                        Role::Heading,
                        row.left_bottom() + vec2(4.0, -8.0),
                        Align2::LEFT_BOTTOM,
                        15.0,
                        GREEN,
                        &day_text(date).to_uppercase(),
                    );
                    for e in entries {
                        let h = if e.summary.is_empty() { 34.0 } else { 52.0 };
                        let (row, hover) =
                            ui.allocate_exact_size(vec2(ui.available_width(), h), Sense::hover());
                        let p = ui.painter().clone();
                        p.rect_filled(row.shrink2(vec2(0.0, 2.0)), 6.0, BAR);
                        let number = format!("#{}", e.pr);
                        self.fonts.text(
                            &p,
                            Role::Body,
                            row.left_top() + vec2(14.0, 9.0),
                            Align2::LEFT_TOP,
                            13.0,
                            FAINT,
                            &number,
                        );
                        let room = (row.width() - 90.0).max(40.0);
                        let title = fit(&e.title, room, |s| {
                            self.fonts.width(&p, Role::BodyBold, 15.0, s)
                        });
                        self.fonts.text(
                            &p,
                            Role::BodyBold,
                            row.left_top() + vec2(74.0, 8.0),
                            Align2::LEFT_TOP,
                            15.0,
                            TEXT,
                            &title,
                        );
                        let summary = fit(&e.summary, room, |s| {
                            self.fonts.width(&p, Role::Body, 13.0, s)
                        });
                        if !e.summary.is_empty() {
                            self.fonts.text(
                                &p,
                                Role::Body,
                                row.left_top() + vec2(74.0, 29.0),
                                Align2::LEFT_TOP,
                                13.0,
                                DIM,
                                &summary,
                            );
                        }
                        if title.len() < e.title.len() || summary.len() < e.summary.len() {
                            let full = if e.summary.is_empty() {
                                e.title.clone()
                            } else {
                                format!("{}\n{}", e.title, e.summary)
                            };
                            hover.on_hover_text(full);
                        }
                    }
                }
            });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn e(pr: u32, date: &str) -> Entry {
        Entry {
            pr,
            date: date.into(),
            title: format!("change {pr}"),
            summary: String::new(),
        }
    }

    #[test]
    fn the_built_in_changelog_reads() {
        let log = changelog().expect("docs/changelog.toml");
        assert!(!log.entries.is_empty());
        assert!(
            log.entries
                .iter()
                .all(|e| e.date.len() == 10 && !e.title.is_empty())
        );
    }

    #[test]
    fn days_go_newest_first() {
        let days = by_day(&[
            e(1, "2026-11-20"),
            e(5, "2026-11-25"),
            e(3, "2026-11-25"),
            e(2, "2026-11-21"),
        ]);
        let shape: Vec<(String, Vec<u32>)> = days
            .into_iter()
            .map(|(d, l)| (d, l.iter().map(|e| e.pr).collect()))
            .collect();
        assert_eq!(
            shape,
            [
                ("2026-11-25".to_owned(), vec![5, 3]),
                ("2026-11-21".to_owned(), vec![2]),
                ("2026-11-20".to_owned(), vec![1]),
            ]
        );
    }

    #[test]
    fn days_read_as_words() {
        assert_eq!(day_text("2026-11-26"), "26 November 2026");
        assert_eq!(day_text("2026-01-05"), "5 January 2026");
        assert_eq!(day_text("soon"), "soon");
    }

    #[test]
    fn long_lines_are_cut() {
        let w = |s: &str| s.chars().count() as f32 * 10.0;
        assert_eq!(fit("short", 100.0, w), "short");
        assert_eq!(fit("a long title here", 100.0, w), "a long...");
    }
}
