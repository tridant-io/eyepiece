//! the update, as the design draws it (docs/design/settings.html, figs. 32, 34 and 35): the strip's quiet "update"
//! button while a newer release is out, the modal that says what it is and puts it in place, and about's line. what
//! the player knows of it comes in as `Updates`; what is asked of it goes out as actions.

use super::widgets::{CHOSEN, ON_CHOSEN};
use super::{font, text_top, Action, Ui, DIM, FG, HOVER, LINE, MONO_MEDIUM, SANS, SOLID, STRIP, TRACK};
use crate::update::Release;
use egui::text::{LayoutJob, TextFormat};
use egui::{Color32, Galley, Pos2, Rect, Sense, Vec2};
use std::sync::Arc;

/// what the player knows of updates
#[derive(Clone, Default)]
pub enum Updates {
    /// not looked for in this run
    #[default]
    Unchecked,
    Checking,
    /// none newer, as found at this time of day
    Current(String),
    /// a newer release, and how far putting it in place has come
    Found(Release, Applying),
    /// no answer, at this time of day
    Failed(String),
}

#[derive(Clone, PartialEq)]
pub enum Applying {
    No,
    /// bytes come, of all to come
    Downloading(u64, u64),
    Checking,
    Restarting,
    Failed(String),
}

/// the modal's width, and the picture behind it darkened as behind a panel
const WIDTH: f32 = 440.0;
const SHADE: Color32 = Color32::from_rgba_premultiplied(0, 0, 0, 153);
/// the column a change's kind stands in, before its words
const KIND: f32 = 66.0;
/// a failure's edge: gray, since amber is kept for a fault in a file
const EDGE: Color32 = Color32::from_rgb(0x77, 0x77, 0x77);

fn megabytes(bytes: u64) -> String {
    format!("{:.1} MB", bytes as f64 / 1e6)
}

fn words(painter: &egui::Painter, text: &str, size: f32, color: Color32, width: f32) -> Arc<Galley> {
    let mut job = LayoutJob::default();
    job.wrap.max_width = width;
    job.append(text, 0.0, TextFormat { font_id: font(SANS, size), color, line_height: Some(size * 1.45), ..Default::default() });
    painter.layout_job(job)
}

/// an outlined button with its words, or the strong one, filled: true when pressed.
fn button(ui: &egui::Ui, name: &str, rect: Rect, label: &str, strong: bool) -> bool {
    let response = ui.interact(rect, ui.scope_id().with(name), Sense::click());
    let painter = ui.painter();
    if strong {
        painter.rect_filled(rect, 0.0, CHOSEN);
    } else if response.hovered() {
        painter.rect_filled(rect, 0.0, HOVER);
    }
    painter.rect_stroke(rect, 0.0, (1.0, if strong { CHOSEN } else { LINE }), egui::StrokeKind::Inside);
    let color = if strong { ON_CHOSEN } else { FG };
    let galley = painter.layout_no_wrap(label.to_string(), font(SANS, 12.0), color);
    painter.galley(Pos2::new(rect.center().x - galley.size().x / 2.0, text_top(painter, font(SANS, 12.0), rect.center().y)), galley, color);
    response.clicked()
}

fn button_width(ui: &egui::Ui, label: &str) -> f32 {
    ui.painter().layout_no_wrap(label.to_string(), font(SANS, 12.0), FG).size().x + 20.0
}

impl Ui {
    /// what the player knows of updates now.
    pub fn set_updates(&mut self, updates: Updates) {
        if !matches!(updates, Updates::Found(..)) {
            self.update_open = false;
        }
        self.updates = updates;
    }

    /// the modal is up: the player's keys wait while it is.
    pub fn update_up(&self) -> bool {
        self.update_open
    }

    /// esc closes the modal, unless an update is on its way, which only its own cancel stops.
    pub fn update_escape(&mut self) -> bool {
        let busy = matches!(&self.updates, Updates::Found(_, Applying::Downloading(..) | Applying::Checking | Applying::Restarting));
        if self.update_open && !busy {
            self.update_open = false;
        }
        self.update_open || !busy
    }

    /// the strip's "update" button, its right edge at `right`, when a newer release is out (fig. 32): 22 high, its
    /// words in the foreground's ink, a 1 px outline in the line's color, no accent. returns its width, which the
    /// tabs give up.
    pub(super) fn update_button(&mut self, ui: &egui::Ui, strip: Rect, right: f32) -> f32 {
        let Updates::Found(release, _) = &self.updates else { return 0.0 };
        let wide = button_width(ui, "update") - 2.0;
        let rect = Rect::from_min_size(Pos2::new(right - 4.0 - wide, strip.top() + (STRIP - 1.0 - 22.0) / 2.0), Vec2::new(wide, 22.0));
        let response = ui.interact(rect, ui.scope_id().with("update"), Sense::click());
        if response.hovered() || self.update_open {
            ui.painter().rect_filled(rect, 0.0, HOVER);
        }
        ui.painter().rect_stroke(rect, 0.0, (1.0, LINE), egui::StrokeKind::Inside);
        let galley = ui.painter().layout_no_wrap("update".to_string(), font(SANS, 12.0), FG);
        ui.painter().galley(Pos2::new(rect.center().x - galley.size().x / 2.0, text_top(ui.painter(), font(SANS, 12.0), rect.center().y)), galley, FG);
        if response.clicked() {
            self.update_open = !self.update_open;
        }
        self.hint(&response, Some((format!("eyepiece {} is out", release.version), String::new())));
        4.0 + wide + 4.0
    }

    /// the update's modal (fig. 35), over whatever is up, in the window's middle: what release is out, what changed
    /// in it, and update or cancel; then how far the update has come, or why it did not finish. a press outside it
    /// closes it, unless an update is on its way.
    pub(super) fn update_modal(&mut self, ui: &egui::Ui, full: Rect, actions: &mut Vec<Action>) {
        let Updates::Found(release, applying) = self.updates.clone() else { return };
        if !self.update_open {
            return;
        }
        let busy = matches!(applying, Applying::Downloading(..) | Applying::Checking | Applying::Restarting);
        let mut close = false;
        let shown = egui::Area::new(egui::Id::unique("update-modal")).order(egui::Order::Foreground).fixed_pos(full.min).show(ui.ctx(), |ui| {
            let (_, behind) = ui.allocate_exact_size(full.size(), Sense::click());
            ui.painter().rect_filled(Rect::from_min_max(Pos2::new(full.left(), full.top() + STRIP), full.max), 0.0, SHADE);
            let inner = WIDTH - 40.0;
            let painter = ui.painter().clone();
            let current = super::panel::VERSION;
            // what the modal says, top to bottom: (galley, room above it, and a change's kind at its left)
            let mut lines: Vec<(Arc<Galley>, f32, Option<Arc<Galley>>)> = Vec::new();
            let (title, sub) = match &applying {
                Applying::No => (format!("eyepiece {} is available", release.version), format!("you have {current} · released {} · {} in all", release.date, megabytes(release.files.iter().map(|f| f.size).sum()))),
                Applying::Downloading(..) => (format!("updating to {}", release.version), format!("from {current} · the player behind keeps playing")),
                Applying::Checking => (format!("checking {}", release.version), format!("from {current} · nothing installed has changed yet")),
                Applying::Restarting => (format!("restarting into {}", release.version), "a moment. nothing to press.".to_string()),
                Applying::Failed(_) => ("the update did not finish".to_string(), format!("{current} is still installed as it was.")),
            };
            lines.push((painter.layout_no_wrap(title, font("sans-semibold", 15.0), FG), 0.0, None));
            lines.push((words(&painter, &sub, 12.0, DIM, inner), 6.0, None));
            let mut notes_at = None;
            match &applying {
                Applying::No => {
                    notes_at = Some(lines.len());
                    for (kind, text) in release.notes.iter().take(3) {
                        let kind = painter.layout_no_wrap(kind.clone(), font(MONO_MEDIUM, 10.5), DIM);
                        lines.push((words(&painter, text, 12.0, FG, inner - KIND), 6.0, Some(kind)));
                    }
                    if release.notes.len() > 3 {
                        lines.push((words(&painter, &format!("and {} more in the release notes.", release.notes.len() - 3), 12.0, DIM, inner), 8.0, None));
                    }
                    let what = format!("update downloads it from github, checks it is Tridant, Inc.'s, puts it in place of {current} and starts it again on the files open now.");
                    lines.push((words(&painter, &what, 12.0, DIM, inner), 14.0, None));
                }
                Applying::Downloading(done, all) => lines.push((words(&painter, &format!("download {} of {}", megabytes(*done), megabytes(*all)), 12.0, FG, inner), 22.0, None)),
                Applying::Checking => lines.push((words(&painter, "its sha-256 against the release's list, and tridant's signature", 12.0, FG, inner), 22.0, None)),
                Applying::Restarting => {}
                Applying::Failed(why) => {
                    lines.push((words(&painter, why, 12.0, FG, inner - 14.0), 14.0, None));
                    lines.push((words(&painter, "the update button stays on the strip. try again now, or any time.", 12.0, DIM, inner), 14.0, None));
                }
            }
            let buttons = !matches!(applying, Applying::Checking | Applying::Restarting);
            let height = 8.0 + 20.0 + lines.iter().map(|(g, above, _)| g.size().y + above).sum::<f32>() + if buttons { 18.0 + 26.0 } else { 0.0 } + 20.0 + if notes_at.is_some() { 22.0 } else { 0.0 };
            let rect = Rect::from_center_size(Pos2::new(full.center().x, full.center().y), Vec2::new(WIDTH, height)).round();
            ui.interact(rect, ui.scope_id().with("update-body"), Sense::click());
            painter.add(egui::Shadow { offset: [0, 10], blur: 28, spread: 0, color: Color32::from_black_alpha(128) }.as_shape(rect, 0.0));
            painter.rect_filled(rect, 0.0, SOLID);
            painter.rect_stroke(rect, 0.0, (1.0, LINE), egui::StrokeKind::Inside);
            let left = rect.left() + 20.0;
            let mut y = rect.top() + 20.0;
            for (i, (galley, above, kind)) in lines.into_iter().enumerate() {
                if notes_at == Some(i) {
                    // what changed, as the release's notes word it, and the link to all of them
                    y += 14.0;
                    painter.text(Pos2::new(left, y + 4.0), egui::Align2::LEFT_TOP, "WHAT CHANGED", font(MONO_MEDIUM, 9.5), DIM);
                    let link = painter.layout_no_wrap("release notes".to_string(), font(SANS, 12.0), FG);
                    let at = Rect::from_min_size(Pos2::new(rect.right() - 20.0 - link.size().x, y), link.size());
                    painter.galley(at.min, link, FG);
                    painter.rect_filled(Rect::from_min_size(Pos2::new(at.left(), at.bottom()), Vec2::new(at.width(), 1.0)), 0.0, DIM);
                    if ui.interact(at, ui.scope_id().with("update-notes"), Sense::click()).clicked() {
                        actions.push(Action::OpenUrl(release.page.clone()));
                    }
                    y += 16.0;
                }
                y += above;
                if let Some(kind) = kind {
                    // the kind stands on the change's first line, by the line's middle
                    painter.galley(Pos2::new(left, text_top(&painter, font(MONO_MEDIUM, 10.5), y + 12.0 * 1.45 / 2.0)), kind, DIM);
                    painter.galley(Pos2::new(left + KIND, y), galley.clone(), FG);
                    y += galley.size().y;
                    continue;
                }
                if matches!(applying, Applying::Failed(_)) && i == 2 {
                    painter.rect_filled(Rect::from_min_size(Pos2::new(left, y - 4.0), Vec2::new(2.0, galley.size().y + 8.0)), 0.0, EDGE);
                    painter.galley(Pos2::new(left + 14.0, y), galley.clone(), FG);
                } else {
                    painter.galley(Pos2::new(left, y), galley.clone(), FG);
                }
                y += galley.size().y;
            }
            if let Applying::Downloading(done, all) = applying {
                // how far the download has come: a bar in the foreground's ink, not the accent, which is the menus'
                let bar = Rect::from_min_size(Pos2::new(left, y + 10.0), Vec2::new(inner, 4.0));
                painter.rect_filled(bar, 2.0, TRACK);
                painter.rect_filled(Rect::from_min_size(bar.min, Vec2::new(inner * (done as f32 / all.max(1) as f32).min(1.0), 4.0)), 2.0, FG);
            }
            if buttons {
                let row = rect.bottom() - 20.0 - 26.0;
                let (first, second) = match applying {
                    Applying::No => (Some("cancel"), Some("update")),
                    Applying::Failed(_) => (Some("close"), Some("try again")),
                    _ => (Some("cancel"), None),
                };
                let mut right = rect.right() - 20.0;
                if let Some(label) = second {
                    let wide = button_width(ui, label);
                    if button(ui, "update-go", Rect::from_min_size(Pos2::new(right - wide, row), Vec2::new(wide, 26.0)), label, true) {
                        actions.push(Action::StartUpdate);
                    }
                    right -= wide + 6.0;
                }
                if let Some(label) = first {
                    let wide = button_width(ui, label);
                    if button(ui, "update-cancel", Rect::from_min_size(Pos2::new(right - wide, row), Vec2::new(wide, 26.0)), label, false) {
                        if busy {
                            actions.push(Action::StopUpdate);
                        } else {
                            close = true;
                        }
                    }
                }
            }
            if behind.clicked() && !busy && !ui.ctx().pointer_latest_pos().is_some_and(|p| rect.contains(p)) {
                close = true;
            }
        });
        ui.ctx().move_to_top(shown.response.layer_id);
        if close {
            self.update_open = false;
        }
    }

    /// about's update line (fig. 34): what the last look for a release found, and check now, or see the update. on
    /// a mac there is none yet: its updates come with an app bundle, which is not built.
    pub(super) fn update_line(&mut self, ui: &mut egui::Ui, check_at_start: bool, actions: &mut Vec<Action>) {
        let width = ui.available_width();
        let painter = ui.painter().clone();
        let (title, sub, label, busy) = if cfg!(target_os = "macos") {
            ("updates come with the mac's app bundle".to_string(), "it is not built yet: a newer version is installed by hand.".to_string(), None, false)
        } else {
            match &self.updates {
                Updates::Unchecked if !check_at_start => ("checks at start are off".to_string(), "turned off in general. check now still asks, once.".to_string(), Some("check now"), false),
                Updates::Unchecked => ("not looked for in this run".to_string(), "eyepiece asks github at most once a day.".to_string(), Some("check now"), false),
                Updates::Checking => ("checking for updates".to_string(), "asking github for the latest release".to_string(), Some("check now"), true),
                Updates::Current(at) => ("up to date".to_string(), format!("checked today at {at}"), Some("check now"), false),
                Updates::Found(release, _) => (format!("eyepiece {} is out", release.version), format!("you have {} · released {}", super::panel::VERSION, release.date), Some("see the update"), false),
                Updates::Failed(at) => ("could not check".to_string(), format!("github did not answer at {at}. it is asked again at the next start."), Some("check now"), false),
            }
        };
        let room = width - 28.0 - label.map_or(0.0, |l| button_width(ui, l) + 14.0);
        let title = painter.layout_no_wrap(title, font(SANS, 12.5), FG);
        let sub = words(&painter, &sub, 11.5, DIM, room);
        let (rect, _) = ui.allocate_exact_size(Vec2::new(width, 12.0 + title.size().y + 4.0 + sub.size().y + 12.0), Sense::hover());
        painter.rect_stroke(rect, 0.0, (1.0, LINE), egui::StrokeKind::Inside);
        painter.galley(Pos2::new(rect.left() + 14.0, rect.top() + 12.0), title.clone(), FG);
        painter.galley(Pos2::new(rect.left() + 14.0, rect.top() + 12.0 + title.size().y + 4.0), sub, DIM);
        if let Some(label) = label {
            let wide = button_width(ui, label);
            let at = Rect::from_min_size(Pos2::new(rect.right() - 14.0 - wide, rect.center().y - 13.0), Vec2::new(wide, 26.0));
            if busy {
                ui.painter().rect_stroke(at, 0.0, (1.0, LINE), egui::StrokeKind::Inside);
                ui.painter().text(at.center(), egui::Align2::CENTER_CENTER, label, font(SANS, 12.0), DIM);
            } else if button(ui, "about-update", at, label, false) {
                if matches!(self.updates, Updates::Found(..)) {
                    self.update_open = true;
                } else {
                    actions.push(Action::CheckUpdates);
                }
            }
        }
    }
}
