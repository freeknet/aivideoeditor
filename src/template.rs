// ============================================================================
// Module:       template
// Description:  Project templates: a starting recipe (deliverables, rules,
//               what the footage is, the browser's measures) to use or not.
//
// Dependencies: anyhow, dirs; crate::recipe
// ============================================================================

//! Templates.
//!
//! A project's recipe says what it makes and how: its deliverables, the
//! rules every draft keeps, a few words on what the footage is (which the
//! captioning model and the chat are told), the props people may hold and
//! the four measures the shot browser shows. A template is a recipe to start
//! from: applying one replaces those parts of the project's recipe and
//! leaves its naming and output settings alone.
//!
//! Two are built in: *General* (the default: a 30-second wide cut and a
//! 30-second vertical one, rules that favour no kind of footage) and *Flow
//! Night* (the weekly fire-and-LED promo set the app grew up on). Any
//! project's recipe can be saved as a template of the user's own, a recipe
//! file in `<config dir>/aivideoeditor/templates/<name>.toml`; one with a
//! built-in's name replaces it.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::recipe::Recipe;

/// A recipe to start a project from.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Template {
    /// Its name, as the pickers show it.
    pub(crate) name: String,
    /// One line on what it is for.
    pub(crate) description: String,
    /// What it sets.
    pub(crate) recipe: Recipe,
    /// Saved by the user (it can be deleted) rather than built in.
    pub(crate) user: bool,
}

/// The built-in templates.
pub(crate) fn built_in() -> Vec<Template> {
    vec![
        Template {
            name: "General".to_owned(),
            description: "A 30-second wide cut and a vertical one; no kind of footage favoured"
                .to_owned(),
            recipe: Recipe::default(),
            user: false,
        },
        Template {
            name: "Flow Night".to_owned(),
            description: "The weekly fire-and-LED promo set: seven cuts, fire on the drop"
                .to_owned(),
            recipe: Recipe::flow_night(),
            user: false,
        },
    ]
}

/// Where the user's templates live.
pub(crate) fn user_dir() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join(crate::APP_ID).join("templates"))
}

/// Every template: the built-ins, then the user's (by name), a user one
/// replacing a built-in of the same name.
pub(crate) fn catalog() -> Vec<Template> {
    let mut all = built_in();
    if let Some(dir) = user_dir() {
        for t in read_dir(&dir) {
            match all.iter_mut().find(|b| b.name == t.name) {
                Some(slot) => *slot = t,
                None => all.push(t),
            }
        }
    }
    all
}

/// The user templates in `dir`, sorted by name.
fn read_dir(dir: &Path) -> Vec<Template> {
    let mut found: Vec<Template> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "toml"))
        .filter_map(|p| {
            let text = std::fs::read_to_string(&p).ok()?;
            let (recipe, _) = crate::recipe::parse(&text);
            let stem = p.file_stem()?.to_string_lossy().into_owned();
            let name = if recipe.template.trim().is_empty() {
                stem
            } else {
                recipe.template.clone()
            };
            Some(Template {
                description: format!("Yours: {}", recipe.about),
                name,
                recipe,
                user: true,
            })
        })
        .collect();
    found.sort_by_key(|t| t.name.to_lowercase());
    found
}

/// A file name for a template called `name`: letters, digits, spaces and
/// dashes kept, the rest dropped.
fn file_name(name: &str) -> String {
    let kept: String = name
        .chars()
        .filter(|c| c.is_alphanumeric() || *c == ' ' || *c == '-' || *c == '_')
        .collect();
    let kept = kept.trim();
    format!("{}.toml", if kept.is_empty() { "template" } else { kept })
}

/// Saves `recipe` as the user template `name` in `dir`; returns its path.
pub(crate) fn save_in(dir: &Path, name: &str, recipe: &Recipe) -> Result<PathBuf> {
    let mut recipe = recipe.clone();
    recipe.template = name.trim().to_owned();
    std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    let path = dir.join(file_name(name));
    recipe.save_to(&path)?;
    Ok(path)
}

/// Saves `recipe` as the user template `name`; returns its path.
pub(crate) fn save(name: &str, recipe: &Recipe) -> Result<PathBuf> {
    let dir = user_dir().context("no settings folder to keep templates in")?;
    save_in(&dir, name, recipe)
}

/// Applies `template` to `recipe`: its deliverables, rules, footage words,
/// props and measures replace the recipe's; naming and output stay.
pub(crate) fn apply(recipe: &mut Recipe, template: &Template) {
    let t = &template.recipe;
    recipe.deliverables.clone_from(&t.deliverables);
    recipe.rules.clone_from(&t.rules);
    recipe.about.clone_from(&t.about);
    recipe.props.clone_from(&t.props);
    recipe.metrics.clone_from(&t.metrics);
    recipe.template.clone_from(&template.name);
}

#[cfg(test)]
mod tests {
    use super::{apply, built_in, read_dir, save_in};
    use crate::recipe::Recipe;

    #[test]
    fn general_is_the_default_and_flow_night_keeps_the_promo_set() -> anyhow::Result<()> {
        let all = built_in();
        let names: Vec<&str> = all.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(
            names,
            vec!["General", "Flow Night"],
            "the built-ins, in order"
        );
        assert_eq!(
            all[0].recipe,
            Recipe::default(),
            "General is what a new project gets"
        );
        assert!(
            all[0].recipe.rules.drop_subject.is_empty(),
            "General favours nothing"
        );
        assert_eq!(all[1].recipe.deliverables.len(), 7, "the weekly set");
        assert_eq!(all[1].recipe.rules.drop_subject, "fire", "fire on the drop");
        Ok(())
    }

    #[test]
    fn applying_a_template_keeps_the_projects_naming() -> anyhow::Result<()> {
        let mut recipe = Recipe {
            filename: "{night}-{id}.mp4".to_owned(),
            ..Recipe::default()
        };
        let flow = built_in()
            .into_iter()
            .nth(1)
            .ok_or_else(|| anyhow::anyhow!("no Flow Night"))?;
        apply(&mut recipe, &flow);
        assert_eq!(recipe.deliverables.len(), 7, "the template's cuts");
        assert_eq!(recipe.template, "Flow Night", "named");
        assert_eq!(
            recipe.filename, "{night}-{id}.mp4",
            "naming is the project's"
        );
        Ok(())
    }

    #[test]
    fn a_saved_template_comes_back_by_name() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let recipe = Recipe {
            about: "a wedding".to_owned(),
            ..Recipe::default()
        };
        save_in(dir.path(), "Weddings: 2027", &recipe)?;
        let found = read_dir(dir.path());
        assert_eq!(found.len(), 1, "one saved: {found:?}");
        assert_eq!(found[0].name, "Weddings: 2027", "the name as typed");
        assert_eq!(
            found[0].recipe.about, "a wedding",
            "what it says about the footage"
        );
        assert!(found[0].user, "the user's own");
        Ok(())
    }
}
