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

use crate::gui::ui::hint::Hint as _;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use super::parts;
use crate::gui::pages::PageCtx;
use crate::gui::ui::{icons, theme, widgets};
use crate::plan::Rules;
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
    /// The template picked in the Template section (not applied yet).
    template_pick: Option<String>,
    /// The name typed for Save as template.
    template_name: String,
    /// The props as typed (a comma list), while being edited.
    props_draft: Option<String>,
    /// What the last template action did.
    template_note: Option<String>,
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

/// The widest the settings' form rows grow.
const FORM_WIDTH: f32 = 760.0;

/// What a settings row's buttons asked for.
enum RowAction {
    /// Pick a new path.
    Change,
    /// Forget it.
    Clear,
}

/// One settings row: the label, the path as a read-only field (or
/// `placeholder`), "Change…", and a clear button when `clearable`.
fn path_row(
    ctx: &PageCtx<'_>,
    ui: &mut egui::Ui,
    label: &str,
    glyph: char,
    path: Option<&Path>,
    placeholder: &str,
    clearable: bool,
) -> Option<RowAction> {
    let theme = ctx.theme;
    let mut action = None;
    parts::form_row(ui, theme, label, |ui| {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = theme::SPACE_XS;
            if clearable && path.is_some() {
                if widgets::icon_btn(ui, theme, icons::X, &format!("Clear {label}"), "").clicked() {
                    action = Some(RowAction::Clear);
                }
            } else {
                ui.add_space(theme::CONTROL_HEIGHT + theme::SPACE_XS);
            }
            if widgets::outline_button(ui, theme, "Change\u{2026}")
                .hint(format!("Choose the {}", label.to_lowercase()))
                .clicked()
            {
                action = Some(RowAction::Change);
            }
            ui.add_space(theme::SPACE_XS);
            let width = ui.available_width();
            parts::path_box(ui, theme, glyph, path, placeholder, width);
        });
    });
    ui.add_space(theme::SPACE_XS + 2.0);
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
    parts::section(
        ui,
        ctx.theme,
        "Project settings",
        "Saved with the project. A new project starts with the last one's.",
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
    ui.scope(|ui| {
        ui.set_max_width(FORM_WIDTH);
        if let Some(p) = ctx.project.as_mut() {
            let folder = p.folder_name();
            let mut name = p.settings.name.clone().unwrap_or_default();
            parts::form_row(ui, ctx.theme, "Name", |ui| {
                let field = ui
                    .allocate_ui(egui::vec2(320.0, theme::CONTROL_HEIGHT), |ui| {
                        widgets::text_field(ui, ctx.theme, &mut name, &folder)
                            .accessible_name("Project name")
                    })
                    .inner
                    .hint("What this project is called; empty uses its folder's name");
                if field.changed() {
                    let trimmed = name.trim();
                    p.settings.name = (!trimmed.is_empty()).then(|| trimmed.to_owned());
                    if let Err(e) = p.save_settings() {
                        p.warnings.push(format!("Could not save ave.toml: {e:#}"));
                    }
                }
            });
            ui.add_space(theme::SPACE_XS + 2.0);
        }
        if let Some(a) = path_row(
            ctx,
            ui,
            "Footage folder",
            icons::FOLDER,
            footage.as_deref(),
            "Not set: pick the folder of clips",
            false,
        ) {
            change = Some(("footage".to_owned(), a));
        }
        if let Some(a) = path_row(
            ctx,
            ui,
            "Music track",
            icons::MUSIC,
            track.as_deref(),
            if ctx
                .project
                .as_ref()
                .is_some_and(crate::project::Project::no_music)
            {
                super::wizard::NO_MUSIC_NOTE
            } else {
                "Not set: pick the approved track"
            },
            true,
        ) {
            change = Some(("track".to_owned(), a));
        }
        for key in &keys {
            let label = format!("End card {key}");
            let hint = if used.contains(key) {
                "Not set: these deliverables end without a card"
            } else {
                "Not set (no deliverable uses it)"
            };
            if let Some(a) = path_row(
                ctx,
                ui,
                &label,
                icons::IMAGE,
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
            let mut dialog = rfd::FileDialog::new().set_title("The folder your clips are in");
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
                .set_title("The track the cut is timed to")
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
        // No track is no music: the old track's map is replaced, or it
        // would bring the track straight back.
        ("track", RowAction::Clear) => {
            let cleared = settings.track.take().is_some();
            if cleared && let Err(e) = crate::beats::write_no_music(&project.work) {
                project
                    .warnings
                    .push(format!("Could not clear the music: {e:#}"));
            }
            cleared
        }
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
    if changed {
        project.reload_music();
    }
}

/// A recipe table's column heading: 11 px Medium, muted.
fn head(ui: &mut egui::Ui, theme: &crate::theme::Palette, text: &str) {
    ui.label(
        egui::RichText::new(text)
            .font(theme::medium(ui.ctx(), theme::SMALL_SIZE))
            .color(theme::rgb(theme.text_muted)),
    );
}

/// The deliverables table and the recipe-wide naming and output options.
fn recipe_table(ctx: &mut PageCtx<'_>, state: &mut State, ui: &mut egui::Ui) {
    template_section(ui, ctx.theme, state);
    ui.add_space(theme::SPACE_XL);
    let Some(loaded) = state.loaded.as_ref() else {
        return;
    };
    let theme = ctx.theme;
    let from = match loaded.source {
        Source::Night => "Saved with the project.",
        Source::UserDefault => {
            "From your last saved recipe; saved to this project on the first edit."
        }
        Source::BuiltIn => "The built-in set; saved to this project on the first edit.",
    };
    parts::section(
        ui,
        theme,
        "Deliverables",
        &format!(
            "What this project makes. Each deliverable is drafted as the plan of the same name. {from}"
        ),
    );
    for w in &loaded.warnings {
        parts::status_line(ui, theme, parts::Kind::Warning, w);
    }
    if let Some(e) = &state.error {
        parts::status_line(ui, theme, parts::Kind::Danger, e);
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
    let mut changed = false;
    let mut remove: Option<usize> = None;
    let recipe_view = loaded.recipe.clone();
    egui::Grid::new("night-recipe")
        .num_columns(8)
        .spacing([theme::SPACE_SM, theme::SPACE_XS + 2.0])
        .min_row_height(theme::CONTROL_HEIGHT)
        .show(ui, |ui| {
            for text in [
                "Id",
                "Shape",
                "Length",
                "End card",
                "Card holds",
                "File name template",
                "Delivers as",
                "",
            ] {
                head(ui, theme, text);
            }
            ui.end_row();
            for (i, d) in loaded.recipe.deliverables.iter_mut().enumerate() {
                changed |= deliverable_row(ui, theme, i, d, &card_keys);
                let name = recipe_view.filename(d, recipe::MAIN_VERSION, &names);
                let exists = out.join(&name).is_file();
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = theme::SPACE_XS + 2.0;
                    if exists {
                        icons::icon(
                            ui,
                            icons::CIRCLE_CHECK,
                            icons::SMALL,
                            theme::rgb(theme.success),
                        );
                    } else {
                        icons::icon(
                            ui,
                            icons::CIRCLE,
                            icons::SMALL,
                            theme::rgb(theme.text_faint),
                        );
                    }
                    ui.label(
                        egui::RichText::new(&name)
                            .font(theme::font(theme::BODY_SIZE))
                            .color(theme::rgb(if exists {
                                theme.text
                            } else {
                                theme.text_muted
                            })),
                    )
                })
                .response
                .hint(if exists {
                    "Already delivered (a new render replaces it)"
                } else {
                    "Not delivered yet"
                });
                if widgets::icon_btn(
                    ui,
                    theme,
                    icons::TRASH,
                    &format!("Remove deliverable {}", i + 1),
                    "",
                )
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
        ui.spacing_mut().item_spacing.x = theme::SPACE_SM;
        if widgets::button_with(
            ui,
            theme,
            widgets::ButtonKind::Outline,
            Some(icons::PLUS),
            "Add deliverable",
        )
        .clicked()
        {
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
    });
    ui.add_space(theme::SPACE_XL);
    parts::section(
        ui,
        theme,
        "Output",
        "How the project's videos are named, where they go and how they are encoded.",
    );
    ui.scope(|ui| {
        ui.set_max_width(FORM_WIDTH);
        parts::form_row(ui, theme, "File names", |ui| {
            let field = ui
                .allocate_ui(egui::vec2(280.0, theme::CONTROL_HEIGHT), |ui| {
                    widgets::text_field(
                        ui,
                        theme,
                        &mut loaded.recipe.filename,
                        recipe::DEFAULT_FILENAME,
                    )
                    .accessible_name("File names")
                })
                .inner;
            changed |= widgets::tip(
                field,
                theme,
                "Fill-ins: {night} {id} {aspect} {version} {date}",
                "",
            )
            .changed();
        });
        ui.add_space(theme::SPACE_XS + 2.0);
        parts::form_row(ui, theme, "Output folder", |ui| {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = theme::SPACE_XS;
                if loaded.recipe.output_dir.is_some() {
                    if widgets::icon_btn(
                        ui,
                        theme,
                        icons::ROTATE_CCW,
                        "Back to the default output folder",
                        "",
                    )
                    .clicked()
                    {
                        loaded.recipe.output_dir = None;
                        changed = true;
                    }
                } else {
                    ui.add_space(theme::CONTROL_HEIGHT + theme::SPACE_XS);
                }
                if widgets::outline_button(ui, theme, "Change\u{2026}").clicked() {
                    let night = recipe::night_dir(&loaded.work);
                    if let Some(dir) = rfd::FileDialog::new()
                        .set_title("Where the project's videos go")
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
                ui.add_space(theme::SPACE_XS);
                let width = ui.available_width();
                parts::path_box(ui, theme, icons::FOLDER, Some(&out), "", width);
            });
        });
        ui.add_space(theme::SPACE_XS + 2.0);
        parts::form_row(ui, theme, "Encoder", |ui| {
            let items: Vec<(recipe::EncoderChoice, &str)> = recipe::EncoderChoice::ALL
                .iter()
                .map(|c| (*c, c.label()))
                .collect();
            changed |= widgets::segmented(ui, theme, &mut loaded.recipe.encoder, &items);
        });
        ui.add_space(theme::SPACE_XS + 2.0);
        parts::form_row(ui, theme, "Timeline", |ui| {
            ui.allocate_ui(egui::vec2(360.0, theme::CONTROL_HEIGHT), |ui| {
                changed |= widgets::switch(
                    ui,
                    theme,
                    &mut loaded.recipe.otio,
                    "Also export an OTIO timeline with each render",
                )
                .changed();
            });
        });
    });
    ui.add_space(theme::SPACE_XL);
    let no_music = ctx
        .project
        .as_ref()
        .is_some_and(crate::project::Project::no_music);
    let template_rules = crate::template::catalog()
        .into_iter()
        .find(|t| t.name == loaded.recipe.template)
        .map_or_else(|| Recipe::default().rules, |t| t.recipe.rules);
    changed |= rules_section(
        ui,
        theme,
        &mut loaded.recipe.rules,
        no_music,
        &template_rules,
    );
    if changed {
        state.dirty = true;
    }
    let editing = ui.ctx().egui_wants_keyboard_input() || ui.ctx().egui_is_using_pointer();
    state.save_if_done(editing);
}

/// The measures the shot browser can show as bars, by recipe name.
const BAR_CHOICES: [&str; 8] = ["q", "focus", "exp", "luma", "move", "light", "fire", "led"];

/// The Template section: start from a template (or save this project's
/// setup as one), and say what the footage is, the props in it and which
/// measures the shot browser shows.
fn template_section(ui: &mut egui::Ui, theme: &crate::theme::Palette, state: &mut State) {
    let Some(loaded) = state.loaded.as_mut() else {
        return;
    };
    let mut changed = false;
    parts::section(
        ui,
        theme,
        "Template",
        "What kind of project this is. A template sets the deliverables, the rules, what the footage is and what the shot browser measures; applying one replaces those, and keeps the naming and output.",
    );
    let catalog = crate::template::catalog();
    let current = loaded.recipe.template.clone();
    ui.scope(|ui| {
        ui.set_max_width(FORM_WIDTH);
        parts::form_row(ui, theme, "Start from", |ui| {
            let pick = state
                .template_pick
                .clone()
                .unwrap_or_else(|| if current.is_empty() { "General".to_owned() } else { current.clone() });
            egui::ComboBox::from_id_salt("recipe-template")
                .width(220.0)
                .selected_text(pick.as_str())
                .show_ui(ui, |ui| {
                    for t in &catalog {
                        if ui
                            .selectable_label(pick == t.name, &t.name)
                            .hint(&t.description)
                            .clicked()
                        {
                            state.template_pick = Some(t.name.clone());
                        }
                    }
                })
                .response
                .accessible_name("Template")
                .hint("The template to apply");
            let chosen = catalog.iter().find(|t| t.name == pick);
            if widgets::outline_button(ui, theme, "Apply")
                .hint("Replace this project's deliverables, rules, footage words and measures with the template's")
                .clicked()
                && let Some(t) = chosen
            {
                crate::template::apply(&mut loaded.recipe, t);
                state.props_draft = None;
                state.template_note = Some(format!("Applied {}.", t.name));
                changed = true;
            }
        });
        ui.add_space(theme::SPACE_XS + 2.0);
        parts::form_row(ui, theme, "Save as", |ui| {
            ui.allocate_ui(egui::vec2(220.0, theme::CONTROL_HEIGHT), |ui| {
                widgets::text_field(ui, theme, &mut state.template_name, "template name")
                    .accessible_name("New template name")
            });
            let name = state.template_name.trim().to_owned();
            if ui
                .add_enabled_ui(!name.is_empty(), |ui| widgets::outline_button(ui, theme, "Save template"))
                .inner
                .hint("Keep this project's deliverables, rules, footage words and measures as a template of your own")
                .clicked()
            {
                state.template_note = Some(match crate::template::save(&name, &loaded.recipe) {
                    Ok(path) => format!("Saved {name} to {}.", path.display()),
                    Err(e) => format!("Could not save the template: {e:#}"),
                });
                state.template_name.clear();
            }
        });
        if let Some(note) = &state.template_note {
            ui.add_space(theme::SPACE_XS);
            parts::status_line(ui, theme, parts::Kind::Info, note);
        }
        ui.add_space(theme::SPACE_XS + 2.0);
        parts::form_row(ui, theme, "The footage is", |ui| {
            let field = ui
                .allocate_ui(egui::vec2(FORM_WIDTH - 200.0, theme::CONTROL_HEIGHT), |ui| {
                    widgets::text_field(ui, theme, &mut loaded.recipe.about, recipe::DEFAULT_ABOUT)
                        .accessible_name("What the footage is")
                })
                .inner;
            changed |= field
                .hint("A few words the captioning model and the chat are told, e.g. a wedding, a skate session")
                .changed();
        });
        ui.add_space(theme::SPACE_XS + 2.0);
        parts::form_row(ui, theme, "Props", |ui| {
            let draft = state
                .props_draft
                .get_or_insert_with(|| loaded.recipe.props.join(", "));
            let field = ui
                .allocate_ui(egui::vec2(FORM_WIDTH - 200.0, theme::CONTROL_HEIGHT), |ui| {
                    widgets::text_field(ui, theme, draft, "none listed: whatever people hold")
                        .accessible_name("Props")
                })
                .inner;
            if field
                .hint("Things people may hold, comma-separated, so captions name them (fire staff, LED hoop)")
                .changed()
            {
                loaded.recipe.props = draft
                    .split(',')
                    .map(str::trim)
                    .filter(|p| !p.is_empty())
                    .map(str::to_owned)
                    .collect();
                changed = true;
            }
        });
        ui.add_space(theme::SPACE_XS + 2.0);
        parts::form_row(ui, theme, "Browser shows", |ui| {
            while loaded.recipe.metrics.len() < 4 {
                let n = loaded.recipe.metrics.len();
                loaded
                    .recipe
                    .metrics
                    .push(recipe::DEFAULT_METRICS.get(n).copied().unwrap_or("q").to_owned());
            }
            for (i, slot) in loaded.recipe.metrics.iter_mut().take(4).enumerate() {
                let shown = crate::search::Metric::bars(std::slice::from_ref(slot))[0].display();
                egui::ComboBox::from_id_salt(("recipe-bar", i))
                    .width(96.0)
                    .selected_text(shown)
                    .show_ui(ui, |ui| {
                        for name in BAR_CHOICES {
                            let label = crate::search::Metric::bars(&[name.to_owned()])[0].display();
                            if ui.selectable_label(slot == name, label).clicked() {
                                *slot = name.to_owned();
                                changed = true;
                            }
                        }
                    })
                    .response
                    .accessible_name(format!("Bar {}", i + 1))
                    .hint("A measure the shot browser shows as a bar on each shot");
            }
        });
    });
    if changed {
        state.dirty = true;
    }
}

/// Problems a shot's source range can be flagged with, in the words the
/// index uses (see `analyze::spans`).
const FLAGS: [(&str, &str); 5] = [
    ("dark", "Too dark"),
    ("blown", "Blown out"),
    ("soft", "Out of focus"),
    ("shaky", "Shaky"),
    ("whip", "Whip pan"),
];

/// Caption moments a shot can be rejected for.
const MOMENTS: [(&str, &str); 3] = [
    ("transition", "Setting up or walking in"),
    ("idle", "Standing still"),
    ("crowd", "Mostly audience"),
];

/// The rules every draft of this project keeps, editable; returns whether
/// any changed. They live in the recipe, so a new project starts from them.
fn rules_section(
    ui: &mut egui::Ui,
    theme: &crate::theme::Palette,
    rules: &mut Rules,
    no_music: bool,
    defaults: &Rules,
) -> bool {
    let mut changed = false;
    parts::section(
        ui,
        theme,
        "Rules",
        "What every draft keeps to. The check holds a cut to them too; a cut you already drafted keeps the rules it was drafted with until you redraft it.",
    );
    ui.scope(|ui| {
        ui.set_max_width(FORM_WIDTH);
        parts::form_row(ui, theme, "Same clip", |ui| {
            changed |= ui
                .add(
                    egui::DragValue::new(&mut rules.same_clip_gap)
                        .range(0.0..=600.0)
                        .speed(1.0)
                        .suffix(" s apart"),
                )
                .hint("Two shots from one clip start at least this far apart in it")
                .accessible_name("Same clip gap")
                .changed();
        });
        ui.add_space(theme::SPACE_XS + 2.0);
        parts::form_row(ui, theme, "Same person", |ui| {
            changed |= ui
                .add(
                    egui::DragValue::new(&mut rules.person_gap)
                        .range(0..=30)
                        .speed(0.1)
                        .prefix("not again within ")
                        .suffix(" shots"),
                )
                .hint(
                    "Unnamed footage counts as one person per clip, and per 3 minutes of a long clip. 0 turns it off.",
                )
                .accessible_name("Person gap")
                .changed();
        });
        ui.add_space(theme::SPACE_XS + 2.0);
        parts::form_row(ui, theme, "Look-alikes", |ui| {
            let mut on = rules.lookalike > 0.0;
            if widgets::switch(
                ui,
                theme,
                &mut on,
                "Keep shots that look the same apart (same person, same spot)",
            )
            .changed()
            {
                rules.lookalike = if on { crate::plan::LOOKALIKE } else { 0.0 };
                changed = true;
            }
        });
        ui.add_space(theme::SPACE_XS + 2.0);
        parts::form_row(ui, theme, "Reused moments", |ui| {
            changed |= widgets::switch(
                ui,
                theme,
                &mut rules.reject_reused,
                "Flag a moment another project's cut already used",
            )
            .changed();
        });
        ui.add_space(theme::SPACE_XS + 2.0);
        parts::form_row(ui, theme, "Avoid footage", |ui| {
            ui.horizontal_wrapped(|ui| {
                for (flag, label) in FLAGS {
                    changed |= list_box(ui, theme, &mut rules.reject_flags, flag, label);
                }
            });
        });
        ui.add_space(theme::SPACE_XS + 2.0);
        parts::form_row(ui, theme, "Avoid moments", |ui| {
            ui.horizontal_wrapped(|ui| {
                for (moment, label) in MOMENTS {
                    changed |= list_box(ui, theme, &mut rules.reject_moments, moment, label);
                }
                changed |= widgets::checkbox(ui, theme, &mut rules.reject_blocked, "View blocked")
                    .hint("Needs captions")
                    .changed();
            });
        });
        ui.add_space(theme::SPACE_XS + 2.0);
        // Only a cut to music has a drop.
        if !no_music {
        parts::form_row(ui, theme, "On the drop", |ui| {
            let items = [("", "Any shot"), ("fire", "Fire"), ("led", "LED")];
            let mut current = items
                .iter()
                .position(|(v, _)| *v == rules.drop_subject)
                .unwrap_or(0);
            let labels: Vec<(usize, &str)> =
                items.iter().enumerate().map(|(i, (_, l))| (i, *l)).collect();
            if widgets::segmented(ui, theme, &mut current, &labels) {
                rules.drop_subject = items.get(current).map_or("", |(v, _)| *v).to_owned();
                changed = true;
            }
        });
        }
        ui.add_space(theme::SPACE_XS + 2.0);
        parts::form_row(ui, theme, "", |ui| {
            if ui
                .add_enabled_ui(rules != defaults, |ui| {
                    widgets::ghost_button(ui, theme, "Back to the template's rules")
                })
                .inner
                .clicked()
            {
                rules.clone_from(defaults);
                changed = true;
            }
        });
    });
    changed
}

/// A checkbox for whether `value` is in `list`; returns whether it changed.
fn list_box(
    ui: &mut egui::Ui,
    theme: &crate::theme::Palette,
    list: &mut Vec<String>,
    value: &str,
    label: &str,
) -> bool {
    let mut on = list.iter().any(|v| v == value);
    if widgets::checkbox(ui, theme, &mut on, label).changed() {
        if on {
            list.push(value.to_owned());
        } else {
            list.retain(|v| v != value);
        }
        return true;
    }
    false
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
        .allocate_ui(egui::vec2(96.0, theme::CONTROL_HEIGHT), |ui| {
            ui.set_min_width(96.0);
            widgets::text_field(ui, theme, &mut d.id, "id")
                .accessible_name(format!("Deliverable {} id", i + 1))
        })
        .inner
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
        })
        .response
        .accessible_name(format!("Deliverable {} shape", i + 1));
    changed |= ui
        .add(
            egui::DragValue::new(&mut d.seconds)
                .range(5.0..=600.0)
                .speed(0.5)
                .suffix(" s"),
        )
        .accessible_name(format!("Deliverable {} length", i + 1))
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
        })
        .response
        .accessible_name(format!("Deliverable {} end card", i + 1));
    changed |= ui
        .add(
            egui::DragValue::new(&mut d.card_bars)
                .range(0..=8)
                .speed(0.05)
                .suffix(" bars"),
        )
        .hint("How long the end card holds, in bars (two seconds each with no music)")
        .accessible_name(format!("Deliverable {} card length", i + 1))
        .changed();
    let mut template = d.filename.clone().unwrap_or_default();
    let response = ui
        .allocate_ui(egui::vec2(160.0, theme::CONTROL_HEIGHT), |ui| {
            ui.set_min_width(160.0);
            widgets::text_field(ui, theme, &mut template, "recipe default")
                .accessible_name(format!("Deliverable {} file name", i + 1))
        })
        .inner;
    if response.changed() {
        d.filename = Some(template).filter(|t| !t.trim().is_empty());
        changed = true;
    }
    changed
}
