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

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crossbeam_channel::Receiver;

use crate::gui::app::Action;
use crate::gui::pages::PageCtx;
use crate::gui::ui::{theme, widgets};
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
    let out = recipe.output_dir(work, &names);
    let delivered = recipe
        .deliverables
        .iter()
        .filter(|d| {
            out.join(recipe.filename(d, recipe::MAIN_VERSION, &names))
                .is_file()
        })
        .count();
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
    Summary {
        work: work.to_path_buf(),
        name: names.night,
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
    let mut dialog = rfd::FileDialog::new().set_title("New night: pick this week's footage folder");
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

/// Draws the column. Returns a footage folder the user picked for a new
/// night, for the page to set up.
pub(crate) fn draw(ctx: &mut PageCtx<'_>, state: &mut State, ui: &mut egui::Ui) -> Option<PathBuf> {
    refresh(ctx, state, false);
    let mut notice = None;
    let theme = ctx.theme;
    ui.horizontal(|ui| {
        if widgets::button(ui, theme, "New night…", widgets::Emphasis::Primary)
            .on_hover_text(
                "Pick this week's footage folder; the recipe, end cards and track \
                 settings come from last week",
            )
            .clicked()
        {
            notice = pick_footage(ctx, state);
        }
        if widgets::button(ui, theme, "Open…", widgets::Emphasis::Secondary)
            .on_hover_text("Open a night's footage folder or _ave folder (Ctrl+O)")
            .clicked()
        {
            ctx.actions.push(Action::PickProject);
        }
    });
    ui.add_space(theme::SPACE_MD);
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new("RECENT NIGHTS")
                .small()
                .strong()
                .color(theme::rgb(theme.text_muted)),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if state.scanning.is_some() {
                ui.add(egui::Spinner::new().size(12.0));
            } else if ui
                .small_button("↻")
                .on_hover_text("Rescan the nights")
                .clicked()
            {
                refresh(ctx, state, true);
            }
        });
    });
    ui.add_space(theme::SPACE_XS);
    if state.list.is_empty() {
        let text = if state.scanning.is_some() {
            "Looking for nights…"
        } else {
            "Nights you open show up here, with last week's beside them."
        };
        widgets::muted(ui, theme, text);
        return notice;
    }
    let open = ctx.project.as_ref().map(|p| p.work.clone());
    egui::ScrollArea::vertical()
        .id_salt("home-recent-list")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for night in state.list.clone() {
                row(
                    ctx,
                    ui,
                    &night,
                    open.as_deref() == Some(night.work.as_path()),
                );
            }
        });
    notice
}

/// One night in the list.
fn row(ctx: &mut PageCtx<'_>, ui: &mut egui::Ui, night: &Summary, open: bool) {
    let theme = ctx.theme;
    let width = ui.available_width();
    let height = 64.0;
    let (rect, response) = ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::click());
    let lift = widgets::hover_t(ui, response.id, response.hovered());
    let fill = if open {
        theme.selection
    } else {
        theme.panel.lerp(theme.hover, lift)
    };
    ui.painter().rect_filled(
        rect,
        egui::CornerRadius::same(theme::RADIUS),
        theme::rgb(fill),
    );
    if open {
        let bar = egui::Rect::from_min_size(
            rect.left_top() + egui::vec2(0.0, theme::SPACE_SM),
            egui::vec2(theme::SELECTION_BAR, height - theme::SPACE_SM * 2.0),
        );
        ui.painter()
            .rect_filled(bar, egui::CornerRadius::same(2), theme::rgb(theme.accent));
    }
    let thumb = egui::Rect::from_min_size(
        rect.left_top() + egui::vec2(theme::SPACE_SM, theme::SPACE_SM),
        egui::vec2(85.0, height - theme::SPACE_SM * 2.0),
    );
    if let Some((path, t)) = &night.thumb {
        ctx.thumbs
            .paint(ui, thumb, path, *t, 0.5, theme::rgb(theme.raised));
    } else {
        ui.painter()
            .rect_filled(thumb, egui::CornerRadius::same(4), theme::rgb(theme.raised));
    }
    let x = thumb.right() + theme::SPACE_SM;
    let painter = ui.painter();
    painter.text(
        egui::pos2(x, rect.top() + 10.0),
        egui::Align2::LEFT_TOP,
        &night.name,
        theme::font(theme::BODY_SIZE),
        theme::rgb(theme.text),
    );
    let sub = if night.clips > 0 {
        format!("{} clips", night.clips)
    } else {
        "no footage yet".to_owned()
    };
    let sub = if night.delivered.0 > 0 && night.progress != Progress::Delivered {
        format!("{sub} · {}/{} out", night.delivered.0, night.delivered.1)
    } else {
        sub
    };
    painter.text(
        egui::pos2(x, rect.top() + 29.0),
        egui::Align2::LEFT_TOP,
        sub,
        theme::font(theme::BODY_SIZE - 2.0),
        theme::rgb(theme.text_muted),
    );
    let (dot, label) = match night.progress {
        Progress::Delivered => (theme.success, night.progress.label()),
        Progress::Planned | Progress::Captioned => (theme.accent, night.progress.label()),
        Progress::Indexed => (theme.warning, night.progress.label()),
        Progress::New => (theme.text_faint, night.progress.label()),
    };
    let y = rect.top() + 47.0;
    painter.circle_filled(egui::pos2(x + 3.5, y), 3.5, theme::rgb(dot));
    painter.text(
        egui::pos2(x + 11.0, y),
        egui::Align2::LEFT_CENTER,
        label,
        theme::font(theme::BODY_SIZE - 3.0),
        theme::rgb(theme.text_muted),
    );
    let tip = if night.date.is_empty() || night.name.starts_with(&night.date) {
        night.work.display().to_string()
    } else {
        format!("{} · {}", night.date, night.work.display())
    };
    let response = response.on_hover_text(tip);
    if response.clicked() && !open {
        ctx.actions.push(Action::OpenProject(night.work.clone()));
    }
    response.context_menu(|ui| {
        if ui.button("Open").clicked() {
            ctx.actions.push(Action::OpenProject(night.work.clone()));
            ui.close();
        }
        if ui.button("Show in Explorer").clicked() {
            crate::gui::pages::deliver::files::open_folder(&recipe::night_dir(&night.work));
            ui.close();
        }
        if ui.button("Remove from recent").clicked() {
            let key = night.work.display().to_string();
            if let Some(list) = ctx.config.recent_projects.as_mut() {
                list.retain(|p| *p != key);
            }
            ui.close();
        }
    });
    ui.add_space(2.0);
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
