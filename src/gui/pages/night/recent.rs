// ============================================================================
// Module:       gui::pages::night::recent
// Description:  The recent-nights column: each night's frame, date and
//               progress, scanned off the UI thread; New night and Open.
//
// Dependencies: egui, rfd, crossbeam-channel; crate::{index, project, recipe}
// ============================================================================

//! Recent nights.
//!
//! The list is the config's recent nights plus every sibling folder of
//! theirs that has an `_ave` work folder, so last week shows up even if it
//! was never opened in the app. Summarising a night reads its shot log
//! (megabytes of JSON), so the scan runs on its own thread and the column
//! fills in when it lands; it reruns when the recent list changes, when a
//! job finishes, and on demand.

use crate::gui::ui::hint::Hint as _;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crossbeam_channel::Receiver;

use crate::gui::app::Action;
use crate::gui::pages::PageCtx;
use crate::gui::ui::{icons, theme, widgets};
use crate::recipe::{self, Recipe};

/// How far a night got.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Progress {
    /// Nothing run yet.
    New,
    /// Shot log written.
    Indexed,
    /// Captions in the shot log.
    Captioned,
    /// Plans drafted.
    Planned,
    /// Every deliverable in the output folder.
    Delivered,
}

impl Progress {
    /// The word the list shows.
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::New => "new",
            Self::Indexed => "indexed",
            Self::Captioned => "captioned",
            Self::Planned => "planned",
            Self::Delivered => "delivered",
        }
    }
}

/// One night, as the list shows it.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Summary {
    /// Its work folder.
    pub(crate) work: PathBuf,
    /// Its name (the folder above `_ave`).
    pub(crate) name: String,
    /// Its date, `YYYY-MM-DD`, or empty when the name carries none.
    pub(crate) date: String,
    /// How far it got.
    pub(crate) progress: Progress,
    /// Clips in the shot log (or footage folder).
    pub(crate) clips: usize,
    /// Deliverables found in the output folder, of how many.
    pub(crate) delivered: (usize, usize),
    /// The first clip and the time to show from it.
    pub(crate) thumb: Option<(PathBuf, f64)>,
}

/// Reads what the list shows about the night at `work`. Slow (it parses the
/// shot log): call off the UI thread.
pub(crate) fn summarize(work: &Path) -> Summary {
    let names = recipe::Names::for_work(work);
    let settings: crate::project::Settings = std::fs::read_to_string(work.join("ave.toml"))
        .ok()
        .and_then(|t| toml::from_str(&t).ok())
        .unwrap_or_default();
    let index = work
        .join("index.json")
        .exists()
        .then(|| crate::index::Index::load(work).ok())
        .flatten();
    let footage = settings
        .footage
        .clone()
        .or_else(|| index.as_ref().map(|i| PathBuf::from(&i.source)));
    let (clips, thumb) = match &index {
        Some(i) => (
            i.clips.len(),
            i.clips
                .first()
                .map(|c| (PathBuf::from(&c.path), (c.probe.duration * 0.4).max(0.0))),
        ),
        None => {
            let videos = footage
                .as_deref()
                .and_then(|f| crate::index::list_videos(f).ok())
                .unwrap_or_default();
            (videos.len(), videos.first().map(|v| (v.clone(), 1.0)))
        }
    };
    let (recipe, _, _) = Recipe::load(work);
    let delivered = crate::gui::pages::deliver::files::delivered_count(work, &recipe);
    let plans = std::fs::read_dir(work.join("plans"))
        .map(|e| {
            e.filter_map(Result::ok)
                .filter(|e| e.path().extension().is_some_and(|x| x == "toml"))
                .count()
        })
        .unwrap_or(0);
    let captioned = index
        .as_ref()
        .is_some_and(|i| i.segments.iter().any(|s| s.caption.is_some()));
    let total = recipe.deliverables.len();
    let progress = if total > 0 && delivered == total {
        Progress::Delivered
    } else if plans > 0 {
        Progress::Planned
    } else if captioned {
        Progress::Captioned
    } else if index.is_some() {
        Progress::Indexed
    } else {
        Progress::New
    };
    let date = recipe::night_date(&names.night).unwrap_or_default();
    // The name set in the project's settings, else its folder's.
    let named = std::fs::read_to_string(work.join("ave.toml"))
        .ok()
        .and_then(|t| toml::from_str::<crate::project::Settings>(&t).ok())
        .and_then(|s| s.name)
        .filter(|n| !n.trim().is_empty());
    Summary {
        work: work.to_path_buf(),
        name: named.unwrap_or(names.night),
        date,
        progress,
        clips,
        delivered: (delivered, total),
        thumb,
    }
}

/// The nights to list: `recent` (work folders, newest first), the open
/// night, and every sibling of theirs with an `_ave` folder; newest date
/// first, undated ones after in the order they were found.
pub(crate) fn candidates(recent: &[PathBuf]) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    let push = |p: PathBuf, out: &mut Vec<PathBuf>| {
        if !out.contains(&p) {
            out.push(p);
        }
    };
    for work in recent {
        if work.is_dir() {
            push(work.clone(), &mut out);
        }
    }
    let parents: Vec<PathBuf> = recent
        .iter()
        .filter_map(|w| recipe::night_dir(w).parent().map(Path::to_path_buf))
        .collect();
    for parent in parents {
        let Ok(entries) = std::fs::read_dir(&parent) else {
            continue;
        };
        for entry in entries.filter_map(Result::ok) {
            let work = entry.path().join("_ave");
            if work.is_dir() {
                push(work, &mut out);
            }
        }
    }
    let key =
        |p: &PathBuf| recipe::night_date(&recipe::Names::for_work(p).night).unwrap_or_default();
    out.sort_by_key(|p| std::cmp::Reverse(key(p)));
    out.truncate(24);
    out
}

/// The column's state.
#[derive(Debug, Default)]
pub(crate) struct State {
    /// The last scan's result.
    list: Vec<Summary>,
    /// A scan in flight.
    scanning: Option<Receiver<Vec<Summary>>>,
    /// What the last scan was for, and when it ran.
    scanned: Option<(Vec<PathBuf>, Instant)>,
    /// Jobs finished when the last scan started, to rescan after a run.
    jobs_done: usize,
}

impl State {
    /// The most recent night other than `except`, to carry forward from.
    pub(crate) fn last_night(&self, except: Option<&Path>) -> Option<PathBuf> {
        self.list
            .iter()
            .map(|s| s.work.clone())
            .find(|w| Some(w.as_path()) != except)
    }

    /// The nights found by the last scan.
    pub(crate) fn list(&self) -> &[Summary] {
        &self.list
    }

    /// Whether a scan is in flight.
    pub(crate) fn scanning(&self) -> bool {
        self.scanning.is_some()
    }
}

/// How often the list rescans by itself.
const RESCAN: Duration = Duration::from_secs(45);

/// The work folders the list starts from.
fn inputs(ctx: &PageCtx<'_>) -> Vec<PathBuf> {
    let mut recent: Vec<PathBuf> = ctx
        .config
        .recent_projects
        .clone()
        .unwrap_or_default()
        .into_iter()
        .map(PathBuf::from)
        .collect();
    if let Some(p) = ctx.project.as_ref()
        && !recent.contains(&p.work)
    {
        recent.insert(0, p.work.clone());
    }
    recent
}

/// Starts a scan when the inputs changed, a job finished, or it is stale.
pub(crate) fn refresh(ctx: &PageCtx<'_>, state: &mut State, force: bool) {
    if let Some(rx) = &state.scanning {
        if let Ok(list) = rx.try_recv() {
            state.list = list;
            state.scanning = None;
        } else {
            return;
        }
    }
    let inputs = inputs(ctx);
    let done = ctx
        .jobs
        .list
        .iter()
        .filter(|j| j.status != crate::jobs::Status::Running)
        .count();
    let stale = state
        .scanned
        .as_ref()
        .is_none_or(|(was, at)| *was != inputs || at.elapsed() > RESCAN || done != state.jobs_done);
    if !(stale || force) {
        return;
    }
    state.jobs_done = done;
    state.scanned = Some((inputs.clone(), Instant::now()));
    let (tx, rx) = crossbeam_channel::bounded(1);
    std::thread::spawn(move || {
        let list: Vec<Summary> = candidates(&inputs).iter().map(|w| summarize(w)).collect();
        // The receiver has gone when what asked was closed; nobody is left to tell.
        let _ = tx.send(list);
    });
    state.scanning = Some(rx);
}

/// The night a new one carries over from: the open one, else the newest.
pub(crate) fn previous(ctx: &PageCtx<'_>, state: &State) -> Option<PathBuf> {
    ctx.project
        .as_ref()
        .map(|p| p.work.clone())
        .or_else(|| state.last_night(None))
}

/// Asks for this week's footage folder, starting beside the last night.
pub(crate) fn pick_footage(ctx: &PageCtx<'_>, state: &State) -> Option<PathBuf> {
    let mut dialog =
        rfd::FileDialog::new().set_title("New project: pick the folder your clips are in");
    if let Some(parent) = previous(ctx, state)
        .as_deref()
        .and_then(|p| recipe::night_dir(p).parent().map(Path::to_path_buf))
    {
        dialog = dialog.set_directory(parent);
    }
    dialog.pick_folder()
}

/// A night [`set_up`] made.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct SetUp {
    /// Its work folder.
    pub(crate) work: PathBuf,
    /// The night it carried over from.
    pub(crate) previous: Option<PathBuf>,
    /// What was carried over or found, one line each.
    pub(crate) notes: Vec<String>,
}

/// Sets a night up for `footage` from the last one (`recipe::new_night`)
/// and opens it.
pub(crate) fn set_up(
    ctx: &mut PageCtx<'_>,
    state: &State,
    footage: &Path,
) -> anyhow::Result<SetUp> {
    let previous = previous(ctx, state).filter(|p| *p != crate::project::work_dir_for(footage));
    let made = recipe::new_night(footage, previous.as_deref())?;
    ctx.actions.push(Action::OpenProject(made.work.clone()));
    Ok(SetUp {
        work: made.work,
        previous,
        notes: made.notes,
    })
}

/// Draws the column beside the open night: a header with New night, Open
/// and Rescan, then the nights. Returns a footage folder the user picked
/// for a new night, for the page to set up.
pub(crate) fn draw(ctx: &mut PageCtx<'_>, state: &mut State, ui: &mut egui::Ui) -> Option<PathBuf> {
    refresh(ctx, state, false);
    let mut picked = None;
    let theme = ctx.theme;
    let open_keys = crate::gui::ui::keymap::command("open-night")
        .map(|c| c.hint(ctx.keymap))
        .unwrap_or_default();
    let mut rescan = false;
    widgets::panel_header(ui, theme, "Projects", |ui| {
        if state.scanning.is_some() {
            ui.add_space(theme::SPACE_XS);
            ui.add(widgets::spinner(icons::SMALL, theme::rgb(theme.text_muted)));
            ui.add_space(theme::SPACE_XS);
        } else if widgets::icon_btn(ui, theme, icons::REFRESH, "Rescan the projects", "").clicked()
        {
            rescan = true;
        }
        if widgets::icon_btn(
            ui,
            theme,
            icons::FOLDER_OPEN,
            "Open a project's footage or _ave folder",
            &open_keys,
        )
        .clicked()
        {
            ctx.actions.push(Action::PickProject);
        }
        if widgets::icon_btn(
            ui,
            theme,
            icons::PLUS,
            "New project: pick the folder your clips are in",
            "",
        )
        .clicked()
        {
            picked = pick_footage(ctx, state);
        }
    });
    if rescan {
        refresh(ctx, state, true);
    }
    ui.add_space(theme::SPACE_XS);
    if state.list.is_empty() {
        let scanning = state.scanning.is_some();
        widgets::empty_state(
            ui,
            theme,
            if scanning {
                icons::LOADER
            } else {
                icons::FOLDER
            },
            if scanning {
                "Looking for projects"
            } else {
                "No other projects"
            },
            if scanning {
                ""
            } else {
                "Projects you open show up here, with the ones beside them on disk."
            },
            |_| {},
        );
        return picked;
    }
    let open = ctx.project.as_ref().map(|p| p.work.clone());
    egui::ScrollArea::vertical()
        .id_salt("home-recent-list")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            let nights = state.list.clone();
            let here = |night: &Summary| open.as_deref() == Some(night.work.as_path());
            let clicked = rows(ctx, ui, &nights, here, "home-recent-rows");
            for (night, clicked) in nights.iter().zip(clicked) {
                if clicked && !here(night) {
                    ctx.actions.push(Action::OpenProject(night.work.clone()));
                }
            }
        });
    picked
}

/// How tall a night's row is.
pub(crate) const ROW_HEIGHT: f32 = 40.0;

/// The badge a night's progress shows: its word and tone.
fn badge(progress: Progress) -> (&'static str, widgets::Tone) {
    let tone = match progress {
        Progress::Delivered => widgets::Tone::Success,
        Progress::New => widgets::Tone::Warning,
        Progress::Indexed | Progress::Captioned | Progress::Planned => widgets::Tone::Neutral,
    };
    (progress.label(), tone)
}

/// Paints a badge (as `widgets::pill` draws one) with its right edge at
/// `right`, centred on `y`; returns its width.
pub(crate) fn paint_badge(
    ui: &egui::Ui,
    theme: &crate::theme::Palette,
    right: f32,
    y: f32,
    text: &str,
    tone: widgets::Tone,
) -> f32 {
    let (fill, ink) = match tone {
        widgets::Tone::Neutral => (
            theme::ink(theme, theme::INK_10),
            theme::rgb(theme.text_muted),
        ),
        widgets::Tone::Key => (
            theme::translucent(theme.accent, 38),
            theme::rgb(theme.accent),
        ),
        widgets::Tone::Success => (
            theme::translucent(theme.success, 38),
            theme::rgb(theme.success),
        ),
        widgets::Tone::Warning => (
            theme::translucent(theme.warning, 38),
            theme::rgb(theme.warning),
        ),
        widgets::Tone::Danger => (
            theme::translucent(theme.danger, 38),
            theme::rgb(theme.danger),
        ),
    };
    let galley = ui.painter().layout_no_wrap(
        text.to_owned(),
        theme::medium(ui.ctx(), theme::SMALL_SIZE),
        ink,
    );
    let size = egui::vec2(
        galley.size().x + theme::SPACE_SM + theme::SPACE_XS,
        theme::SPACE_LG + theme::SPACE_XS,
    );
    let rect = egui::Rect::from_min_size(egui::pos2(right - size.x, y - size.y / 2.0), size);
    ui.painter()
        .rect_filled(rect, egui::CornerRadius::same(theme::RADIUS), fill);
    ui.painter()
        .galley(rect.center() - galley.size() / 2.0, galley, ink);
    size.x
}

/// Nights as a Material 3 list, one [`ROW_HEIGHT`] row each: its frame,
/// its name, the clip and delivery counts in muted text, and a badge for
/// how far it got, on the list's own container, state layer and selected
/// fill (for the night `open` says is open). Each row names the night to a
/// screen reader and takes the arrow keys; a context menu opens it, shows
/// it in Explorer or drops it from the recent list. Returns, per night,
/// whether it was clicked (or opened from its menu).
pub(crate) fn rows(
    ctx: &mut PageCtx<'_>,
    ui: &mut egui::Ui,
    nights: &[Summary],
    open: impl Fn(&Summary) -> bool,
    id_salt: &str,
) -> Vec<bool> {
    use egui_material3::{ListItem, MaterialList};
    let theme = ctx.theme;
    // Every row's painter needs the thumbnail cache; the rows are built
    // before any of them paints, so they share it.
    let thumbs = std::cell::RefCell::new(&mut *ctx.thumbs);
    let list = nights.iter().fold(
        MaterialList::new()
            .id(ui.make_persistent_id(id_salt))
            .dividers(false)
            .fill_width(true),
        |list, night| {
            let thumbs = &thumbs;
            list.item(
                ListItem::new(night.name.clone())
                    .dense(true)
                    .row_height(ROW_HEIGHT)
                    .selected(open(night))
                    .content_ui(move |ui, item| {
                        paint_row(ui, theme, &mut thumbs.borrow_mut(), night, item.rect);
                    }),
            )
        },
    );
    let shown = list.show(ui);
    nights
        .iter()
        .zip(shown.items)
        .map(|(night, response)| {
            let tip = if night.date.is_empty() || night.name.starts_with(&night.date) {
                night.work.display().to_string()
            } else {
                format!("{} \u{b7} {}", night.date, night.work.display())
            };
            let response = response.hint(tip);
            let mut clicked = response.clicked();
            widgets::context_menu(&response, |ui| {
                if widgets::menu_item(ui, theme, Some(icons::FOLDER_OPEN), "Open", "", false)
                    .clicked()
                {
                    clicked = true;
                    ui.close();
                }
                if widgets::menu_item(
                    ui,
                    theme,
                    Some(icons::EXTERNAL_LINK),
                    "Show in Explorer",
                    "",
                    false,
                )
                .clicked()
                {
                    crate::gui::pages::deliver::files::open_folder(&recipe::night_dir(&night.work));
                    ui.close();
                }
                widgets::menu_separator(ui, theme);
                if widgets::menu_item(ui, theme, Some(icons::X), "Remove from recent", "", false)
                    .clicked()
                {
                    let key = night.work.display().to_string();
                    if let Some(list) = ctx.config.recent_projects.as_mut() {
                        list.retain(|p| *p != key);
                    }
                    ui.close();
                }
            });
            clicked
        })
        .collect()
}

/// Paints one night's row content into `rect`: the frame, the name and
/// counts, the badge.
fn paint_row(
    ui: &egui::Ui,
    theme: &crate::theme::Palette,
    thumbs: &mut crate::gui::thumbs::Thumbs,
    night: &Summary,
    rect: egui::Rect,
) {
    if !ui.is_rect_visible(rect) {
        return;
    }
    let thumb = egui::Rect::from_min_size(
        egui::pos2(rect.left() + theme::SPACE_XS + 2.0, rect.center().y - 13.5),
        egui::vec2(48.0, 27.0),
    );
    super::parts::thumb(
        thumbs,
        ui,
        theme,
        thumb,
        night.thumb.as_ref().map(|(p, t)| (p.as_path(), *t)),
    );
    let (word, tone) = badge(night.progress);
    let badge_w = paint_badge(
        ui,
        theme,
        rect.right() - theme::SPACE_SM,
        rect.center().y,
        word,
        tone,
    );
    let x = thumb.right() + theme::SPACE_SM + 2.0;
    let text_room = egui::Rect::from_min_max(
        egui::pos2(x, rect.top()),
        egui::pos2(
            rect.right() - theme::SPACE_SM * 2.0 - badge_w,
            rect.bottom(),
        ),
    );
    let painter = ui
        .painter()
        .with_clip_rect(text_room.intersect(ui.clip_rect()));
    let name = painter.layout_no_wrap(
        night.name.clone(),
        theme::font(theme::LARGE_SIZE),
        theme::rgb(theme.text),
    );
    let name_w = name.size().x;
    painter.galley(
        egui::pos2(x, rect.center().y - name.size().y / 2.0),
        name,
        theme::rgb(theme.text),
    );
    let mut meta = if night.clips > 0 {
        format!("{} clips", night.clips)
    } else {
        "no footage yet".to_owned()
    };
    if night.delivered.0 > 0 && night.progress != Progress::Delivered {
        meta.push_str(&format!(
            " \u{b7} {}/{} out",
            night.delivered.0, night.delivered.1
        ));
    }
    painter.text(
        egui::pos2(x + name_w + theme::SPACE_SM, rect.center().y),
        egui::Align2::LEFT_CENTER,
        meta,
        theme::font(theme::BODY_SIZE),
        theme::rgb(theme.text_muted),
    );
}

#[cfg(test)]
mod tests {
    use super::{Progress, candidates, summarize};

    #[test]
    fn siblings_with_work_folders_are_listed_newest_first() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        for night in ["2026-11-04", "2026-11-11", "notes"] {
            std::fs::create_dir_all(dir.path().join(night).join("_ave"))?;
        }
        std::fs::create_dir_all(dir.path().join("2026-11-18"))?;
        let recent = vec![dir.path().join("2026-11-04").join("_ave")];
        let found = candidates(&recent);
        let names: Vec<String> = found
            .iter()
            .filter_map(|w| w.parent()?.file_name())
            .map(|n| n.to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            names,
            ["2026-11-11", "2026-11-04", "notes"],
            "dated nights newest first; folders without _ave left out"
        );
        Ok(())
    }

    #[test]
    fn an_empty_night_is_new() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let work = dir.path().join("2026-11-11").join("_ave");
        std::fs::create_dir_all(&work)?;
        let s = summarize(&work);
        assert_eq!(s.progress, Progress::New, "nothing run");
        assert_eq!(s.date, "2026-11-11", "date from the name");
        assert_eq!(s.clips, 0, "no clips");
        Ok(())
    }
}
