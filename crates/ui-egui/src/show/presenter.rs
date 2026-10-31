use super::keys::keyboard;
use super::render::{fit, next_rect};
use super::zoom::{Zoom, grid_indices, paint_grid, see_all_hit, slide_aspect, toggle_blank, zoom_frame};
use super::{Align2, Color32, CornerRadius, Pos2, Presentation, Rect, RenderOpts, Sense, Show, Stroke, Ui, pos2, theme, vec2};
use crate::icons::{self, Icon};

/// DeckCraft's own brand orange — the presenter chrome is always dark regardless of the app's
/// light/dark setting (a presenter screen is read in a dim room), so this is the fixed accent
/// for it rather than a theme lookup that could resolve to the light palette.
const ACCENT: Color32 = Color32::from_rgb(0xF2, 0x6B, 0x1D);

pub(super) const NOTES_DEFAULT: f32 = 18.0;

pub(super) const NOTES_MIN: f32 = 12.0;

pub(super) const NOTES_MAX: f32 = 48.0;

pub(super) const NOTES_STEP: f32 = 4.0;

/// Presenter-view elapsed time. Pausing freezes the number; the show keeps playing.
#[derive(Clone, Copy)]
pub(super) struct PresenterTimer {
    origin: f64,
    frozen: f64,
    paused: bool,
}

impl PresenterTimer {
    pub(super) fn start(now: f64) -> Self {
        PresenterTimer { origin: finite_time(now), frozen: 0.0, paused: false }
    }

    pub(super) fn elapsed(self, now: f64) -> f64 {
        if self.paused {
            return finite_secs(self.frozen);
        }
        let dt = now - self.origin;
        finite_secs(self.frozen + if dt.is_finite() && dt > 0.0 { dt } else { 0.0 })
    }

    pub(super) fn paused(self) -> bool {
        self.paused
    }

    pub(super) fn toggle(&mut self, now: f64) {
        if self.paused {
            self.origin = finite_time(now);
            self.paused = false;
        } else {
            self.frozen = self.elapsed(now);
            self.paused = true;
        }
    }

    /// Zero the elapsed time. A paused timer stays paused at zero.
    pub(super) fn reset(&mut self, now: f64) {
        self.frozen = 0.0;
        self.origin = finite_time(now);
    }
}

fn finite_time(now: f64) -> f64 {
    if now.is_finite() { now } else { 0.0 }
}

fn finite_secs(n: f64) -> f64 {
    if n.is_finite() && n >= 0.0 { n } else { 0.0 }
}

pub(super) fn format_elapsed(secs: f64) -> String {
    let s = finite_secs(secs) as u64;
    let s = s.min(99 * 3600 + 59 * 60 + 59);
    format!("{:02}:{:02}:{:02}", s / 3600, (s / 60) % 60, s % 60)
}

/// Local wall clock for presenter view. `--:--` where there is no clock (wasm).
pub(super) fn wall_clock() -> String {
    match deckcraft_text::datetime::DateTime::now_local() {
        Some(t) if t.hour < 24 && t.minute < 60 => format!("{:02}:{:02}", t.hour, t.minute),
        _ => "--:--".to_string(),
    }
}

/// Render the presenter window and set `keep` false when the user ends or closes the show.
/// Single-display fallback: the presenter's interface opens in its own floating window
/// (`show.presenter` on, toggled independently of the audience window beside it).
pub(super) fn presenter_window(ctx: &egui::Context, show: &mut Show, doc: &Presentation, now: f64, keep: &mut bool) {
    ctx.show_viewport_immediate(
        egui::ViewportId::from_hash_of("deckcraft_presenter"),
        egui::ViewportBuilder::default().with_title("Presenter View — DeckCraft").with_inner_size([1100.0, 700.0]),
        |ui, _| presenter_content(ui, show, doc, now, false, keep),
    );
}

/// The presenter's own interface: elapsed time, clock, slide count, current and next slide,
/// speaker notes, and the Pause/Black/White/All Slides/Zoom/End Show controls. Drawn either
/// inside `presenter_window`'s own floating window (single display), or directly into this
/// app's main window while a second display carries `output_window` to the audience
/// (`is_root` — see `Show::dual_monitor`). `is_root` only changes what closing this window
/// means: ending the whole show when it is the app's own window, versus just turning the
/// Presenter View off when it is the extra floating window.
pub(super) fn presenter_content(ui: &mut Ui, show: &mut Show, doc: &Presentation, now: f64, is_root: bool, keep: &mut bool) {
    if !keyboard(ui.ctx(), show, doc, now, true) {
        *keep = false;
    }
    let ctx = ui.ctx().clone();
    ctx.request_repaint_after(std::time::Duration::from_millis(500));
    let cur = show.state.slide;
    let next = show.state.peek_next(doc);
    let cur_tex = show.tex.as_ref().map(|(_, t)| t.clone());
    if let Some(n) = next
        && show.next_tex.as_ref().is_none_or(|(k, _)| *k != n)
    {
        // A very tall slide shape would otherwise ask for a picture taller than the GPU's limit.
        let next_size = crate::textures::fit_size(
            (480, (480.0 * doc.slide_size.height / doc.slide_size.width.max(1.0)).clamp(1.0, 1e9) as u32),
            crate::textures::max_side(&ctx),
        );
        let img = deckcraft_render::render_slide(
            doc,
            n,
            &RenderOpts { scale: next_size.0 as f64 / doc.slide_size.width.max(1.0), size: Some(next_size), ..Default::default() },
        );
        show.next_tex = Some((n, crate::textures::upload(&ctx, "presenter-next", crate::textures::to_color_image(&img, false))));
    }
    let next_tex = show.next_tex.as_ref().filter(|(k, _)| Some(*k) == next).map(|(_, t)| t.clone());
    let no_notes = deckcraft_model::text::TextBody::default();
    let notes = doc.slides.get(cur).map_or(&no_notes, |s| &s.notes);
    let clock = wall_clock();
    let mut actions = vec![];
    let elapsed = show.timer.elapsed(now);
    let paused = show.timer.paused();
    let has_next = next.is_some();
    // A plain dark-chrome text button (End Show; small text chips in the toolbar strip).
    let text_button = |ui: &mut Ui, s: &str, fill: Color32| {
        ui.add(egui::Button::new(egui::RichText::new(s).size(14.0).color(Color32::WHITE)).fill(fill).min_size(vec2(0.0, 28.0)))
    };
    // A square icon button for the toolbar under the current slide (pen, eraser, grid, zoom…):
    // filled with the accent when the thing it controls is on.
    let icon_btn = |ui: &mut Ui, icon: Icon, tip: &str, on: bool| -> egui::Response {
        let (rect, resp) = ui.allocate_exact_size(vec2(36.0, 32.0), Sense::click());
        let painter = ui.painter_at(rect);
        let fill = if on { ACCENT } else if resp.hovered() { Color32::from_gray(74) } else { Color32::from_gray(52) };
        painter.rect_filled(rect, CornerRadius::same(6), fill);
        icons::paint(&painter, rect.shrink(7.0), icon, Color32::WHITE, false);
        resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, tip));
        resp.on_hover_text(tip)
    };
    // A round chevron overlaid on the slide's own edge, to step through the show without
    // reaching for the toolbar — only drawn when that direction actually goes somewhere.
    let nav_btn = |ui: &mut Ui, center: Pos2, icon: Icon, tip: &str| -> egui::Response {
        let rect = Rect::from_center_size(center, vec2(40.0, 40.0));
        let resp = ui.interact(rect, ui.scope_id().with(tip), Sense::click());
        let painter = ui.painter_at(rect);
        painter.circle_filled(rect.center(), rect.width() * 0.5, if resp.hovered() { Color32::from_black_alpha(220) } else { Color32::from_black_alpha(150) });
        icons::paint(&painter, rect.shrink(11.0), icon, Color32::WHITE, false);
        resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, tip));
        resp.on_hover_text(tip)
    };
    egui::CentralPanel::default().frame(egui::Frame::NONE.fill(Color32::from_gray(24)).inner_margin(egui::Margin::same(16))).show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(format_elapsed(elapsed)).size(22.0).color(if paused { Color32::from_gray(160) } else { Color32::WHITE }));
            ui.add_space(12.0);
            ui.label(egui::RichText::new(clock.clone()).size(16.0).color(Color32::from_gray(200)));
            ui.add_space(20.0);
            ui.label(egui::RichText::new(format!("Slide {} of {}", cur + 1, doc.slides.len())).size(16.0).color(Color32::from_gray(200)));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if text_button(ui, "End Show", Color32::from_rgb(0xB0, 0x2E, 0x2E)).clicked() {
                    *keep = false;
                }
            });
        });
        ui.add_space(8.0);
        let avail = ui.available_rect_before_wrap();
        if show.presenter_grid {
            // The grid fills this window; the audience keeps seeing the current slide.
            paint_grid(ui.painter(), ui.ctx(), show, doc, avail);
            let hit = ui.interact(avail, ui.scope_id().with("presenter_grid"), Sense::click());
            if hit.clicked()
                && let Some(p) = hit.interact_pointer_pos()
                && let Some(n) = see_all_hit(avail, grid_indices(doc.slides.len()).len(), slide_aspect(doc), p)
                && let Some(&i) = grid_indices(doc.slides.len()).get(n)
            {
                show.presenter_grid = false;
                show.blank = None;
                show.zoom = None;
                show.zoom_arm = false;
                actions.push(show.state.goto(doc, i));
            }
            return;
        }
        let left = Rect::from_min_max(avail.min, pos2(avail.min.x + avail.width() * 0.62, avail.max.y));
        let right = Rect::from_min_max(pos2(left.max.x + 16.0, avail.min.y), avail.max);
        // The toolbar strip lives under the slide, in the band the old 80%-height current-slide
        // image left empty; the slide itself is fit to what's left above it.
        let toolbar_h = 40.0;
        let slide_band = Rect::from_min_max(left.min, pos2(left.max.x, left.max.y - toolbar_h - 10.0));
        let cr = fit(slide_band, doc);
        let cr = Rect::from_min_size(left.min, cr.size());
        if let Some(t) = &cur_tex {
            ui.painter().image(t.id(), cr, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
        } else {
            ui.painter().rect_filled(cr, CornerRadius::ZERO, Color32::from_gray(40));
        }
        if let Some(z) = show.zoom {
            let mark = zoom_frame(cr, z.uv());
            ui.painter().rect_stroke(mark, CornerRadius::ZERO, Stroke::new(2.0, Color32::WHITE), egui::StrokeKind::Inside);
        }
        if show.zoom_arm {
            let hit = ui.interact(cr, ui.scope_id().with("presenter_zoom"), Sense::click());
            hit.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, "Zoom"));
            if hit.clicked()
                && let Some(p) = hit.interact_pointer_pos()
                && cr.width() > 1.0
                && cr.height() > 1.0
            {
                show.zoom = Some(Zoom::at((p.x - cr.min.x) / cr.width(), (p.y - cr.min.y) / cr.height()));
                show.zoom_arm = false;
            }
        }
        if let Some(c) = show.blank {
            ui.painter().rect_filled(cr, CornerRadius::ZERO, c);
        }
        // Previous/Next as round chevrons over the slide's own left/right edge — PowerPoint's own
        // click-to-advance affordance — instead of the old top-bar text buttons.
        if cur > 0 && cr.width() > 90.0 && nav_btn(ui, pos2(cr.min.x + 26.0, cr.center().y), Icon::ChevronLeft, "Previous").clicked() {
            actions.push(show.state.prev(doc));
        }
        if has_next && cr.width() > 90.0 && nav_btn(ui, pos2(cr.max.x - 26.0, cr.center().y), Icon::ChevronRight, "Next").clicked() {
            actions.push(show.state.next(doc));
        }
        // The icon toolbar strip, directly under the slide.
        let toolbar = Rect::from_min_max(pos2(left.min.x, slide_band.max.y + 10.0), pos2(left.max.x, left.max.y));
        let mut tb = ui.new_child(egui::UiBuilder::new().max_rect(toolbar));
        tb.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            if icon_btn(ui, Icon::Pen, "Pen", show.pen).clicked() {
                show.set_pen(!show.pen);
            }
            if icon_btn(ui, Icon::Eraser, "Erase Ink", false).clicked() {
                show.strokes.clear();
            }
            if icon_btn(ui, Icon::Grid, "See All Slides", show.presenter_grid).clicked() {
                show.presenter_grid = !show.presenter_grid;
            }
            if icon_btn(ui, Icon::Zoom, "Zoom Into Slide", show.zoom_arm || show.zoom.is_some()).clicked() {
                show.zoom_key();
            }
            ui.add_space(10.0);
            if icon_btn(ui, if paused { Icon::Play } else { Icon::Pause }, if paused { "Resume Timer" } else { "Pause Timer" }, paused).clicked() {
                show.timer.toggle(now);
            }
            if text_button(ui, "Reset", Color32::from_gray(52)).on_hover_text("Reset Timer").clicked() {
                show.timer.reset(now);
            }
            if text_button(ui, "Black", if show.blank == Some(Color32::BLACK) { ACCENT } else { Color32::from_gray(52) })
                .on_hover_text("Black Screen")
                .clicked()
            {
                toggle_blank(show, Color32::BLACK);
            }
            if text_button(ui, "White", if show.blank == Some(Color32::WHITE) { ACCENT } else { Color32::from_gray(52) })
                .on_hover_text("White Screen")
                .clicked()
            {
                toggle_blank(show, Color32::WHITE);
            }
        });
        let nr = next_rect(right, doc);
        ui.painter().text(right.min, Align2::LEFT_TOP, "Next", theme::font(13.0), Color32::from_gray(170));
        match &next_tex {
            Some(t) => {
                ui.painter().image(t.id(), nr, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
            }
            None => {
                ui.painter().rect_filled(nr, CornerRadius::ZERO, Color32::from_gray(40));
                ui.painter().text(nr.center(), Align2::CENTER_CENTER, "End of slide show", theme::font(13.0), Color32::from_gray(170));
            }
        }
        let notes_head = Rect::from_min_size(pos2(right.min.x, nr.max.y + 12.0), vec2(right.width(), 28.0));
        let mut head = ui.new_child(egui::UiBuilder::new().max_rect(notes_head));
        head.horizontal(|ui| {
            ui.label(egui::RichText::new("Notes").size(13.0).color(Color32::from_gray(170)));
            if ui.button("A-").clicked() {
                show.nudge_notes(-NOTES_STEP);
            }
            ui.label(egui::RichText::new(format!("{}", show.notes_px as i32)).size(13.0).color(Color32::from_gray(200)));
            if ui.button("A+").clicked() {
                show.nudge_notes(NOTES_STEP);
            }
        });
        let notes_rect = Rect::from_min_max(pos2(right.min.x, notes_head.max.y + 4.0), right.max);
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(notes_rect));
        egui::ScrollArea::vertical().show(&mut child, |ui| {
            presenter_notes(ui, notes, show.notes_px);
        });
    });
    if ui.input(|i| i.viewport().close_requested()) {
        if is_root {
            // No other window is left showing the slide: closing the app's own window ends
            // the show outright, rather than leaving Presenter View off with nothing audible.
            *keep = false;
        } else {
            show.presenter = false;
            // A grid left open in the closed window would hold auto-advance for the rest of the show.
            show.presenter_grid = false;
        }
    }
    for a in actions {
        if !show.apply(a, doc, now) {
            *keep = false;
        }
    }
}

/// The audience's own window: the current slide, full-bleed, letterboxed to the slide's aspect
/// ratio, and nothing else — no toolbar, no notes, no title bar. Placed in true (OS-level)
/// borderless full screen on the other display by the operating system's own monitor order
/// (`ViewportBuilder::with_monitor`): never a fixed or guessed position, and never the same
/// window as the Presenter View, so there is nothing here for the audience to see but the
/// slide. Painted from the same `show.tex` the Presenter View just rendered, so the two windows
/// can never disagree about which slide, step or blank/zoom state is current.
pub(super) fn output_window(ctx: &egui::Context, show: &mut Show, doc: &Presentation, _now: f64, keep: &mut bool) {
    let tex = show.tex.as_ref().map(|(_, t)| t.clone());
    let blank = show.blank;
    let strokes = show.strokes.clone();
    let uv = show.zoom_uv();
    let pen_col = theme::to_color32(doc.show.pen_color);
    ctx.show_viewport_immediate(
        egui::ViewportId::from_hash_of("deckcraft_output"),
        egui::ViewportBuilder::default()
            .with_title("DeckCraft")
            .with_decorations(false)
            .with_titlebar_shown(false)
            .with_title_shown(false)
            .with_taskbar(false)
            .with_resizable(false)
            // Second monitor in the operating system's own display order (never index 0, the
            // display this process already started on): borderless full screen, no mirroring.
            .with_monitor(1),
        |ui, _| {
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(100));
            let full = ui.max_rect();
            let painter = ui.painter_at(full);
            painter.rect_filled(full, CornerRadius::ZERO, Color32::BLACK);
            let srect = fit(full, doc);
            if let Some(t) = &tex {
                painter.image(t.id(), srect, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
            }
            if let Some(c) = blank {
                painter.rect_filled(full, CornerRadius::ZERO, c);
            }
            for s in &strokes {
                let pts: Vec<Pos2> = s.iter().map(|q| super::zoom::slide_to_screen(srect, uv, q.x, q.y)).collect();
                painter.add(egui::Shape::line(pts, Stroke::new(3.0, pen_col)));
            }
            if ui.input(|i| i.viewport().close_requested()) {
                *keep = false;
            }
        },
    );
}

/// The current slide's notes in the presenter's notes pane, `px` for 12 pt text.
pub(super) fn presenter_notes(ui: &mut Ui, notes: &deckcraft_model::text::TextBody, px: f32) -> egui::Response {
    let color = Color32::from_gray(230);
    if notes.text().is_empty() {
        ui.label(egui::RichText::new("No notes.").size(px).color(color))
    } else {
        crate::notes::view(ui, notes, &crate::notes::Look { size: px, color: Some(color) })
    }
}
