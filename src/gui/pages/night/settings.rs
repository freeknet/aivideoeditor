// ============================================================================
// Module:       gui::pages::night::settings
// Description:  The night's settings (footage, track, end cards) and its
//               recipe (deliverables, naming, output folder, encoder).
//
// Dependencies: egui, rfd; crate::{project, recipe}
// ============================================================================

//! Settings and recipe.
//!
//! Settings are rows with a path and a "Change…" button and are written to
//! `ave.toml` the moment they change. The recipe is a small table of
//! deliverables with a live file name preview per row; it is written to
//! `recipe.toml` (and the user-level default) once an edit is finished,
//! never on open: opening a night must not write into it.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use crate::gui::pages::PageCtx;
use crate::gui::ui::{theme, widgets};
use crate::recipe::{self, Deliverable, Recipe, Source};

/// The recipe as loaded for the open night.
#[derive(Debug)]
struct Loaded {
    /// The night it belongs to.
    work: PathBuf,
    /// The recipe being edited.
    recipe: Recipe,
    /// Where it came from.
    source: Source,
    /// Parse warnings.
    warnings: Vec<String>,
    /// `recipe.toml`'s modification time when read or written.
    mtime: Option<SystemTime>,
    /// When the file was last looked at.
    checked: Instant,
}

/// The section's state.
#[derive(Debug, Default)]
pub(crate) struct State {
    /// The recipe, once loaded.
    loaded: Option<Loaded>,
    /// Edited but not yet saved.
    dirty: bool,
    /// The last save error.
    error: Option<String>,
}

/// `recipe.toml`'s modification time.
fn mtime(work: &Path) -> Option<SystemTime> {
    std::fs::metadata(work.join(recipe::FILE))
        .and_then(|m| m.modified())
        .ok()
}

impl State {
    /// Loads the open night's recipe, or reloads it when the file changed
    /// underneath (the export panel edits it too).
    pub(crate) fn sync(&mut self, ctx: &PageCtx<'_>) {
        let Some(work) = ctx.project.as_ref().map(|p| p.work.clone()) else {
            self.loaded = None;
            return;
        };
        let reload = match &mut self.loaded {
            Some(l) if l.work == work => {
                if !self.dirty && l.checked.elapsed() > Duration::from_secs(1) {
                    l.checked = Instant::now();
                    mtime(&work) != l.mtime
                } else {
                    false
                }
            }
            Some(_) | None => true,
        };
        if reload {
            let (recipe, warnings, source) = Recipe::load(&work);
            self.loaded = Some(Loaded {
                mtime: mtime(&work),
                work,
                recipe,
                source,
                warnings,
                checked: Instant::now(),
            });
            self.dirty = false;
        }
    }

    /// The recipe for the open night (the built-in one before [`sync`]).
    ///
    /// [`sync`]: State::sync
    pub(crate) fn recipe(&self) -> Recipe {
        self.loaded
            .as_ref()
            .map(|l| l.recipe.clone())
            .unwrap_or_default()
    }

    /// Writes the recipe if it was edited and no edit is in progress.
    fn save_if_done(&mut self, editing: bool) {
        if !self.dirty || editing {
            return;
        }
        let Some(l) = &mut self.loaded else { return };
        let ids: Vec<&str> = l.recipe.deliverables.iter().map(|d| d.id.trim()).collect();
        let unique = ids
            .iter()
            .enumerate()
            .all(|(i, id)| !id.is_empty() && !ids.iter().take(i).any(|o| o == id));
        if !unique {
            self.error = Some("Each deliverable needs its own id; not saved yet.".to_owned());
            return;
        }
        match l.recipe.save(&l.work) {
            Ok(()) => {
                l.mtime = mtime(&l.work);
                l.source = Source::Night;
                self.error = None;
            }
            Err(e) => self.error = Some(format!("Could not save the recipe: {e:#}")),
        }
        self.dirty = false;
    }
}

/// A section title with a muted line under it.
fn section(ctx: &PageCtx<'_>, ui: &mut egui::Ui, title: &str, sub: &str) {
    widgets::title(ui, ctx.theme, title);
    widgets::muted(ui, ctx.theme, sub);
    ui.add_space(theme::SPACE_SM);
}

/// A path shortened from the left to about `max` characters.
fn short(path: &Path, max: usize) -> String {
    let s = path.display().to_string();
    let n = s.chars().count();
    if n <= max {
        return s;
    }
    let tail: String = s.chars().skip(n - max + 1).collect();
    format!("…{tail}")
}

/// What a settings row's buttons asked for.
enum RowAction {
    /// Pick a new path.
    Change,
    /// Forget it.
    Clear,
}

/// One settings row: label, path (or a hint), Change… and Clear.
fn path_row(
    ctx: &PageCtx<'_>,
    ui: &mut egui::Ui,
    label: &str,
    path: Option<&Path>,
    hint: &str,
    clearable: bool,
) -> Option<RowAction> {
    let mut action = None;
    ui.label(egui::RichText::new(label).color(theme::rgb(ctx.theme.text_muted)));
    match path {
        Some(p) => {
            let missing = !p.exists();
            let color = if missing {
                ctx.theme.warning
            } else {
                ctx.theme.text
            };
            let text = if missing {
                format!("{} (missing)", short(p, 64))
            } else {
                short(p, 70)
            };
            ui.label(egui::RichText::new(text).color(theme::rgb(color)))
                .on_hover_text(p.display().to_string());
        }
        None => {
            ui.label(egui::RichText::new(hint).color(theme::rgb(ctx.theme.text_faint)));
        }
    }
    ui.horizontal(|ui| {
        if ui.button("Change…").clicked() {
            action = Some(RowAction::Change);
        }
        if clearable && path.is_some() && ui.button("Clear").clicked() {
            action = Some(RowAction::Clear);
        }
    });
    ui.end_row();
    action
}

/// The settings rows and the recipe table.
pub(crate) fn draw(ctx: &mut PageCtx<'_>, state: &mut State, ui: &mut egui::Ui) {
    state.sync(ctx);
    settings_rows(ctx, state, ui);
    ui.add_space(theme::SPACE_XL);
    recipe_table(ctx, state, ui);
}

/// Footage, track and one end card per aspect the recipe uses.
fn settings_rows(ctx: &mut PageCtx<'_>, state: &State, ui: &mut egui::Ui) {
    section(
        ctx,
        ui,
        "Night settings",
        "Saved in the night's ave.toml. A new night starts with last week's.",
    );
    let Some(p) = ctx.project.as_ref() else {
        return;
    };
    let footage = p.settings.footage.clone();
    let track = p.settings.track.clone();
    let cards = p.settings.end_cards.clone();
    let mut keys: Vec<String> = recipe::ASPECTS
        .iter()
        .map(|a| recipe::aspect_key(*a).to_owned())
        .collect();
    for d in &state.recipe().deliverables {
        let key = d.end_card_key().to_owned();
        if !keys.contains(&key) {
            keys.push(key);
        }
    }
    let used: Vec<String> = state
        .recipe()
        .deliverables
        .iter()
        .map(|d| d.end_card_key().to_owned())
        .collect();

    let mut change: Option<(String, RowAction)> = None;
    egui::Grid::new("night-settings")
        .num_columns(3)
        .spacing([theme::SPACE_LG, theme::SPACE_SM])
        .striped(false)
        .show(ui, |ui| {
            if let Some(a) = path_row(
                ctx,
                ui,
                "Footage folder",
                footage.as_deref(),
                "not set: pick the folder of clips",
                false,
            ) {
                change = Some(("footage".to_owned(), a));
            }
            if let Some(a) = path_row(
                ctx,
                ui,
                "Music track",
                track.as_deref(),
                "not set: pick the approved track",
                true,
            ) {
                change = Some(("track".to_owned(), a));
            }
            for key in &keys {
                let label = format!("End card {key}");
                let hint = if used.contains(key) {
                    "not set: the deliverables in this shape end without a card"
                } else {
                    "not set (no deliverable uses it)"
                };
                if let Some(a) = path_row(
                    ctx,
                    ui,
                    &label,
                    cards.get(key).map(PathBuf::as_path),
                    hint,
                    true,
                ) {
                    change = Some((format!("card:{key}"), a));
                }
            }
        });
    let Some((what, action)) = change else { return };
    let fallback = ctx.project.as_ref().map(|p| recipe::night_dir(&p.work));
    let start = |p: Option<&Path>| {
        p.and_then(Path::parent)
            .map(Path::to_path_buf)
            .or_else(|| fallback.clone())
    };
    let Some(project) = ctx.project.as_mut() else {
        return;
    };
    let settings = &mut project.settings;
    let changed = match (what.as_str(), action) {
        ("footage", RowAction::Change) => {
            let mut dialog = rfd::FileDialog::new().set_title("The night's footage folder");
            if let Some(dir) = start(footage.as_deref()) {
                dialog = dialog.set_directory(dir);
            }
            dialog
                .pick_folder()
                .map(|f| settings.footage = Some(f))
                .is_some()
        }
        ("footage", RowAction::Clear) => false,
        ("track", RowAction::Change) => {
            let mut dialog = rfd::FileDialog::new()
                .set_title("The night's music track")
                .add_filter(
                    "Audio",
                    &["wav", "mp3", "flac", "m4a", "aac", "ogg", "opus"],
                );
            if let Some(dir) = start(track.as_deref()) {
                dialog = dialog.set_directory(dir);
            }
            dialog
                .pick_file()
                .map(|f| settings.track = Some(f))
                .is_some()
        }
        ("track", RowAction::Clear) => settings.track.take().is_some(),
        (key, action) => {
            let key = key.trim_start_matches("card:").to_owned();
            match action {
                RowAction::Change => {
                    let mut dialog = rfd::FileDialog::new()
                        .set_title(format!("End card for {key}"))
                        .add_filter("Image", &["png", "jpg", "jpeg"]);
                    if let Some(dir) = start(cards.get(&key).map(PathBuf::as_path)) {
                        dialog = dialog.set_directory(dir);
                    }
                    dialog
                        .pick_file()
                        .map(|f| settings.end_cards.insert(key, f))
                        .is_some()
                }
                RowAction::Clear => settings.end_cards.remove(&key).is_some(),
            }
        }
    };
    if changed && let Err(e) = project.save_settings() {
        project
            .warnings
            .push(format!("Could not save ave.toml: {e:#}"));
    }
}

/// The deliverables table and the recipe-wide naming and output options.
fn recipe_table(ctx: &mut PageCtx<'_>, state: &mut State, ui: &mut egui::Ui) {
    let Some(loaded) = state.loaded.as_ref() else {
        return;
    };
    let from = match loaded.source {
        Source::Night => "Saved in the night's recipe.toml.",
        Source::UserDefault => {
            "From your last saved recipe; saved to this night on the first edit."
        }
        Source::BuiltIn => "The built-in week; saved to this night on the first edit.",
    };
    section(
        ctx,
        ui,
        "Deliverables",
        &format!(
            "What this night produces. Each deliverable is drafted as the plan of the same name. {from}"
        ),
    );
    for w in &loaded.warnings {
        ui.colored_label(theme::rgb(ctx.theme.warning), w);
    }
    if let Some(e) = &state.error {
        ui.colored_label(theme::rgb(ctx.theme.danger), e);
    }
    let card_keys: Vec<String> = ctx
        .project
        .as_ref()
        .map(|p| p.settings.end_cards.keys().cloned().collect())
        .unwrap_or_default();
    let Some(loaded) = state.loaded.as_mut() else {
        return;
    };
    let names = recipe::Names::for_work(&loaded.work);
    let out = loaded.recipe.output_dir(&loaded.work, &names);
    let theme = ctx.theme;
    let mut changed = false;
    let mut remove: Option<usize> = None;
    let recipe_view = loaded.recipe.clone();
    egui::Grid::new("night-recipe")
        .num_columns(7)
        .spacing([theme::SPACE_MD, theme::SPACE_XS])
        .show(ui, |ui| {
            for head in [
                "Id",
                "Shape",
                "Length",
                "End card",
                "File name template",
                "Delivers as",
                "",
            ] {
                ui.label(
                    egui::RichText::new(head)
                        .small()
                        .strong()
                        .color(theme::rgb(theme.text_muted)),
                );
            }
            ui.end_row();
            for (i, d) in loaded.recipe.deliverables.iter_mut().enumerate() {
                changed |= deliverable_row(ui, theme, i, d, &card_keys);
                let name = recipe_view.filename(d, recipe::MAIN_VERSION, &names);
                let exists = out.join(&name).is_file();
                let color = if exists {
                    theme.success
                } else {
                    theme.text_muted
                };
                ui.label(egui::RichText::new(&name).color(theme::rgb(color)))
                    .on_hover_text(if exists {
                        "Already delivered (a new render replaces it)"
                    } else {
                        "Not delivered yet"
                    });
                if ui
                    .small_button("✕")
                    .on_hover_text("Remove this deliverable")
                    .clicked()
                {
                    remove = Some(i);
                }
                ui.end_row();
            }
        });
    if let Some(i) = remove
        && i < loaded.recipe.deliverables.len()
    {
        loaded.recipe.deliverables.remove(i);
        changed = true;
    }
    ui.add_space(theme::SPACE_SM);
    ui.horizontal(|ui| {
        if ui.button("+ Add deliverable").clicked() {
            let mut n = loaded.recipe.deliverables.len() + 1;
            let mut id = format!("cut-{n}");
            while loaded.recipe.get(&id).is_some() {
                n += 1;
                id = format!("cut-{n}");
            }
            loaded.recipe.deliverables.push(Deliverable::new(
                &id,
                crate::plan::Aspect::Vertical,
                30.0,
            ));
            changed = true;
        }
        if ui
            .button("Reset to the Flow Night week")
            .on_hover_text("Three reels, a story, a feed cut, a wide cut and a recap")
            .clicked()
        {
            loaded.recipe.deliverables = Recipe::default().deliverables;
            changed = true;
        }
    });
    ui.add_space(theme::SPACE_MD);
    egui::Grid::new("night-recipe-options")
        .num_columns(2)
        .spacing([theme::SPACE_LG, theme::SPACE_SM])
        .show(ui, |ui| {
            ui.label(egui::RichText::new("File names").color(theme::rgb(theme.text_muted)));
            ui.horizontal(|ui| {
                changed |= ui
                    .add(
                        egui::TextEdit::singleline(&mut loaded.recipe.filename)
                            .desired_width(200.0)
                            .hint_text(recipe::DEFAULT_FILENAME),
                    )
                    .changed();
                ui.label(
                    egui::RichText::new("{night} {id} {aspect} {version} {date}")
                        .small()
                        .color(theme::rgb(theme.text_faint)),
                );
            });
            ui.end_row();
            ui.label(egui::RichText::new("Output folder").color(theme::rgb(theme.text_muted)));
            ui.horizontal(|ui| {
                ui.label(short(&out, 60))
                    .on_hover_text(out.display().to_string());
                if ui.button("Change…").clicked() {
                    let night = recipe::night_dir(&loaded.work);
                    if let Some(dir) = rfd::FileDialog::new()
                        .set_title("Where the night's videos go")
                        .set_directory(&night)
                        .pick_folder()
                    {
                        loaded.recipe.output_dir = Some(
                            dir.strip_prefix(&night)
                                .map_or(dir.clone(), Path::to_path_buf)
                                .display()
                                .to_string(),
                        );
                        changed = true;
                    }
                }
                if loaded.recipe.output_dir.is_some() && ui.button("Default").clicked() {
                    loaded.recipe.output_dir = None;
                    changed = true;
                }
            });
            ui.end_row();
            ui.label(egui::RichText::new("Encoder").color(theme::rgb(theme.text_muted)));
            ui.horizontal(|ui| {
                for choice in recipe::EncoderChoice::ALL {
                    changed |= ui
                        .selectable_value(&mut loaded.recipe.encoder, choice, choice.label())
                        .changed();
                }
            });
            ui.end_row();
            ui.label(egui::RichText::new("Timeline").color(theme::rgb(theme.text_muted)));
            changed |= ui
                .checkbox(
                    &mut loaded.recipe.otio,
                    "Also export an OTIO timeline with each render",
                )
                .changed();
            ui.end_row();
        });
    if changed {
        state.dirty = true;
    }
    let editing = ui.ctx().egui_wants_keyboard_input() || ui.ctx().egui_is_using_pointer();
    state.save_if_done(editing);
}

/// One deliverable's editable cells (id, shape, length, end card, template).
/// Returns whether anything changed.
fn deliverable_row(
    ui: &mut egui::Ui,
    theme: &crate::theme::Palette,
    i: usize,
    d: &mut Deliverable,
    card_keys: &[String],
) -> bool {
    let mut changed = false;
    changed |= ui
        .add(
            // min_size: a TextEdit in a Grid shrinks to the remembered column width
            // (the header's), so desired_width alone leaves it clipped.
            egui::TextEdit::singleline(&mut d.id)
                .desired_width(80.0)
                .min_size(egui::vec2(80.0, 0.0)),
        )
        .changed();
    egui::ComboBox::from_id_salt(("recipe-aspect", i))
        .width(64.0)
        .selected_text(recipe::aspect_key(d.aspect))
        .show_ui(ui, |ui| {
            for aspect in recipe::ASPECTS {
                changed |= ui
                    .selectable_value(&mut d.aspect, aspect, recipe::aspect_key(aspect))
                    .changed();
            }
        });
    changed |= ui
        .add(
            egui::DragValue::new(&mut d.seconds)
                .range(5.0..=600.0)
                .speed(0.5)
                .suffix(" s"),
        )
        .changed();
    let shown = d
        .end_card
        .clone()
        .unwrap_or_else(|| format!("auto ({})", recipe::aspect_key(d.aspect)));
    egui::ComboBox::from_id_salt(("recipe-card", i))
        .width(96.0)
        .selected_text(shown)
        .show_ui(ui, |ui| {
            changed |= ui
                .selectable_value(&mut d.end_card, None, "auto (by shape)")
                .changed();
            for key in card_keys {
                changed |= ui
                    .selectable_value(&mut d.end_card, Some(key.clone()), key)
                    .changed();
            }
        });
    let mut template = d.filename.clone().unwrap_or_default();
    let response = ui.add(
        egui::TextEdit::singleline(&mut template)
            .desired_width(140.0)
            .min_size(egui::vec2(140.0, 0.0))
            .hint_text(egui::RichText::new("recipe default").color(theme::rgb(theme.text_faint))),
    );
    if response.changed() {
        d.filename = Some(template).filter(|t| !t.trim().is_empty());
        changed = true;
    }
    changed
}
