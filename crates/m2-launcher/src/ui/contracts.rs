//! The Contracts tab: which of the story's contracts play through in this
//! build, from `docs/contracts-status.toml` (built in; see `m2-tracker` and
//! docs/contract-tracker.md). A progress bar by status, filter chips, and a
//! row per contract with its note, issue numbers and when it was checked.
//! Only our own text is shown: ids, statuses and notes. Colours invented.

use egui::{
    Align2, Color32, CornerRadius, Id, Rect, Sense, Stroke, StrokeKind, Ui, UiBuilder, pos2, vec2,
};
use m2_tracker::{Status, Tracker, group};

use super::{App, BAR, BG, DIM, FAINT, LINE, PAD, TEXT};
use crate::assets::Role;

/// The tracker as built in.
const STATUS_FILE: &str = include_str!("../../../../docs/contracts-status.toml");

pub(super) fn tracker() -> Result<Tracker, String> {
    Tracker::parse(STATUS_FILE)
}

pub(super) fn colour(s: Status) -> Color32 {
    match s {
        Status::Playable => Color32::from_rgb(0x3c, 0x85, 0x27),
        Status::PlayableWithGaps => Color32::from_rgb(0xc8, 0x8a, 0x2a),
        Status::NotYet => Color32::from_rgb(0xa8, 0x3a, 0x2e),
        Status::Unknown => Color32::from_rgb(0x55, 0x55, 0x55),
    }
}

/// `text` cut to fit `room` (by `width`), ending in an ellipsis when cut.
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

/// The filter chips: everything, or one status.
pub(super) fn shown(filter: Option<Status>, s: Status) -> bool {
    filter.is_none_or(|f| f == s)
}

impl App {
    pub(super) fn contracts_tab(&mut self, ui: &mut Ui, rect: Rect) {
        let p = ui.painter_at(rect);
        p.rect_filled(rect, 0.0, BG);
        let t = match tracker() {
            Ok(t) => t,
            Err(e) => {
                self.fonts.text(
                    &p,
                    Role::Body,
                    rect.center(),
                    Align2::CENTER_CENTER,
                    16.0,
                    DIM,
                    &format!("The contract list could not be read: {e}"),
                );
                return;
            }
        };
        let counts = t.counts();
        let total = t.contracts.len().max(1);

        // The progress panel.
        let top = Rect::from_min_size(
            pos2(rect.left() + PAD, rect.top() + 20.0),
            vec2(rect.width() - 2.0 * PAD, 118.0),
        );
        p.rect_filled(top, 6.0, BAR);
        p.rect_stroke(top, 6.0, Stroke::new(1.0, LINE), StrokeKind::Inside);
        let playable = counts[&Status::Playable] + counts[&Status::PlayableWithGaps];
        self.fonts.text(
            &p,
            Role::Heading,
            top.left_top() + vec2(18.0, 14.0),
            Align2::LEFT_TOP,
            20.0,
            TEXT,
            &format!(
                "{playable} OF {} CONTRACTS COMPLETE ({} WITHOUT GAPS)",
                t.contracts.len(),
                counts[&Status::Playable]
            ),
        );
        self.fonts.text(
            &p,
            Role::Body,
            pos2(top.right() - 18.0, top.top() + 18.0),
            Align2::RIGHT_TOP,
            13.0,
            FAINT,
            &format!(
                "Updated {} - from recorded autoplay and playtest runs",
                t.updated
            ),
        );
        let bar = Rect::from_min_size(
            top.left_top() + vec2(18.0, 50.0),
            vec2(top.width() - 36.0, 16.0),
        );
        p.rect_filled(bar, 4.0, Color32::from_rgb(0x30, 0x30, 0x30));
        let mut x = bar.left();
        for s in Status::ALL {
            let w = bar.width() * counts[&s] as f32 / total as f32;
            if w > 0.0 {
                p.rect_filled(
                    Rect::from_min_size(pos2(x, bar.top()), vec2(w, bar.height())),
                    CornerRadius::ZERO,
                    colour(s),
                );
            }
            x += w;
        }

        // Legend chips double as filters.
        let key = Id::new("contracts-filter");
        let mut filter: Option<Status> = ui.ctx().data(|d| d.get_temp(key)).flatten();
        let mut cx = bar.left();
        let chips: Vec<(Option<Status>, String)> =
            std::iter::once((None, format!("All {}", t.contracts.len())))
                .chain(
                    Status::ALL
                        .iter()
                        .map(|&s| (Some(s), format!("{} {}", s.label(), counts[&s]))),
                )
                .collect();
        for (i, (f, label)) in chips.iter().enumerate() {
            let w = self.fonts.width(&p, Role::Body, 13.0, label) + 34.0;
            let chip = Rect::from_min_size(pos2(cx, bar.bottom() + 12.0), vec2(w, 26.0));
            let r = ui.interact(chip, Id::new(("contracts-chip", i)), Sense::click());
            let on = filter == *f;
            p.rect_filled(
                chip,
                13.0,
                if on || r.hovered() {
                    Color32::from_rgb(0x3a, 0x3a, 0x3a)
                } else {
                    Color32::from_rgb(0x2a, 0x2a, 0x2a)
                },
            );
            if on {
                p.rect_stroke(chip, 13.0, Stroke::new(1.0, TEXT), StrokeKind::Inside);
            }
            let dot = f.map_or(TEXT, colour);
            p.circle_filled(chip.left_center() + vec2(14.0, 0.0), 5.0, dot);
            self.fonts.text(
                &p,
                Role::Body,
                chip.left_center() + vec2(24.0, 0.0),
                Align2::LEFT_CENTER,
                13.0,
                if on { TEXT } else { DIM },
                label,
            );
            if r.clicked() {
                filter = *f;
            }
            cx += w + 8.0;
        }
        ui.ctx().data_mut(|d| d.insert_temp(key, filter));

        // The list.
        let list = Rect::from_min_max(
            pos2(rect.left() + PAD, top.bottom() + 16.0),
            rect.max - vec2(PAD, 12.0),
        );
        let rows: Vec<_> = t
            .contracts
            .iter()
            .filter(|c| shown(filter, c.status))
            .cloned()
            .collect();
        let mut child = ui.new_child(UiBuilder::new().max_rect(list));
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(&mut child, |ui| {
                for c in &rows {
                    let (row, hover) =
                        ui.allocate_exact_size(vec2(ui.available_width(), 58.0), Sense::hover());
                    let p = ui.painter().clone();
                    p.rect_filled(row.shrink2(vec2(0.0, 3.0)), 6.0, BAR);
                    p.rect_filled(
                        Rect::from_min_size(
                            row.left_top() + vec2(0.0, 3.0),
                            vec2(5.0, row.height() - 6.0),
                        ),
                        CornerRadius::same(2),
                        colour(c.status),
                    );
                    self.fonts.text(
                        &p,
                        Role::BodyBold,
                        row.left_top() + vec2(18.0, 11.0),
                        Align2::LEFT_TOP,
                        17.0,
                        TEXT,
                        &c.id,
                    );
                    self.fonts.text(
                        &p,
                        Role::Body,
                        row.left_top() + vec2(18.0, 33.0),
                        Align2::LEFT_TOP,
                        12.0,
                        FAINT,
                        group(&c.id),
                    );
                    let pill_text = c.status.label();
                    let pw = self.fonts.width(&p, Role::Body, 13.0, pill_text) + 20.0;
                    let pill =
                        Rect::from_min_size(row.left_top() + vec2(140.0, 16.0), vec2(pw, 24.0));
                    p.rect_filled(pill, 12.0, colour(c.status));
                    self.fonts.text(
                        &p,
                        Role::Body,
                        pill.center(),
                        Align2::CENTER_CENTER,
                        13.0,
                        TEXT,
                        pill_text,
                    );
                    let note = if c.note.is_empty() {
                        "No recorded run yet."
                    } else {
                        c.note.as_str()
                    };
                    let mut refs: Vec<String> = c.issues.iter().map(|n| format!("#{n}")).collect();
                    if let Some(d) = &c.checked {
                        refs.push(format!("checked {d}"));
                    }
                    // One line, cut to fit; the whole note on hover.
                    let room = (row.width() - 330.0 - 16.0).max(40.0);
                    let shown_note = fit(note, room, |s| self.fonts.width(&p, Role::Body, 13.0, s));
                    if shown_note.len() < note.len() {
                        hover.clone().on_hover_text(note);
                    }
                    self.fonts.text(
                        &p,
                        Role::Body,
                        row.left_top() + vec2(330.0, 11.0),
                        Align2::LEFT_TOP,
                        13.0,
                        DIM,
                        &shown_note,
                    );
                    self.fonts.text(
                        &p,
                        Role::Body,
                        row.left_top() + vec2(330.0, 33.0),
                        Align2::LEFT_TOP,
                        12.0,
                        FAINT,
                        &refs.join("  "),
                    );
                }
            });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_built_in_list_reads() {
        let t = tracker().expect("the built-in status file");
        assert_eq!(t.contracts.len(), 52);
    }

    #[test]
    fn long_notes_are_cut_to_fit() {
        let w = |s: &str| s.chars().count() as f32 * 10.0;
        assert_eq!(fit("short", 100.0, w), "short");
        assert_eq!(fit("a long note here", 100.0, w), "a long...");
        assert_eq!(fit("abcdef", 20.0, w), "...");
    }

    #[test]
    fn filters_pick_one_status_or_all() {
        assert!(shown(None, Status::Unknown));
        assert!(shown(Some(Status::NotYet), Status::NotYet));
        assert!(!shown(Some(Status::NotYet), Status::Playable));
        // Every status has its own colour.
        let mut c: Vec<_> = Status::ALL.iter().map(|&s| colour(s).to_array()).collect();
        c.dedup();
        assert_eq!(c.len(), 4);
    }
}
