//! Ignore management: explicit rule destinations, worker previews, ignored paths and explanations.

use std::path::PathBuf;

use canopy_core::{IgnoreRule, IgnoreTarget};

use super::*;
use crate::ui::kit::row::RowKind;
use crate::worker::{IgnoreData, IgnoreQuery};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Page {
    Rule,
    Paths,
    Why,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    File,
    Extension,
    Folder,
    Pattern,
}

/// A form belongs to the repository that opened it, including after a tab switch.
#[derive(Debug, Clone)]
pub struct IgnoreForm {
    pub repo: PathBuf,
    page: Page,
    kind: Kind,
    path: String,
    pattern: String,
    target: IgnoreTarget,
}

impl IgnoreForm {
    pub fn new(repo: PathBuf, path: Option<String>) -> Self {
        Self {
            repo,
            page: Page::Rule,
            kind: if path.is_some() {
                Kind::File
            } else {
                Kind::Pattern
            },
            path: path.unwrap_or_default(),
            pattern: String::new(),
            target: IgnoreTarget::Root,
        }
    }

    pub fn why(mut self) -> Self {
        self.page = Page::Why;
        self
    }

    fn rule(&self) -> IgnoreRule {
        let path = if self.kind == Kind::Pattern && self.path.trim().is_empty() {
            ".gitignore".to_owned()
        } else {
            self.path.trim().to_owned()
        };
        match self.kind {
            Kind::File => IgnoreRule::File(path),
            Kind::Extension => IgnoreRule::Extension(path),
            Kind::Folder => IgnoreRule::Folder(path),
            Kind::Pattern => IgnoreRule::Custom {
                pattern: self.pattern.clone(),
                near: path,
            },
        }
    }

    fn query(&self) -> Option<IgnoreQuery> {
        match self.page {
            Page::Paths => Some(IgnoreQuery::Paths),
            Page::Why if !self.path.trim().is_empty() => {
                Some(IgnoreQuery::Why(self.path.trim().into()))
            }
            Page::Rule
                if (self.kind == Kind::Pattern
                    && !self.pattern.is_empty()
                    && (self.target != IgnoreTarget::Nearest || !self.path.trim().is_empty()))
                    || (self.kind != Kind::Pattern && !self.path.trim().is_empty()) =>
            {
                Some(IgnoreQuery::Preview {
                    rule: self.rule(),
                    target: self.target,
                })
            }
            _ => None,
        }
    }
}

pub(super) fn show(
    ctx: &egui::Context,
    salt: (&'static str, std::mem::Discriminant<Dialog>),
    form: &mut IgnoreForm,
    tab: Option<&RepoTab>,
) -> Outcome {
    let p = palette(ctx);
    let mut actions = Vec::new();
    let validated_query = form.query();
    let footer_page = form.page;
    let can_add = footer_page == Page::Rule && tab.is_some_and(|tab| {
        tab.path == form.repo && tab.running.is_none()
            && validated_query.is_some() && tab.ignore_query == validated_query
            && matches!(&tab.ignore_result, Some(Ok(data)) if matches!(data.as_ref(), IgnoreData::Preview { .. }))
    });
    let result = KitDialog::new(salt).title("Ignore files")
        .icon(icon::EYE_SLASH).width(DialogWidth::Form).initial_focus(InitialFocus::Body)
        .body(|ui| {
            Segmented::new(&mut form.page, &[(Page::Rule, "Add rule"), (Page::Paths, "Ignored files"), (Page::Why, "Why ignored")])
                .label("Ignore management").show(ui);
            ui.add_space(theme::MD);
            match form.page {
                Page::Rule => {
                    let previous_kind = form.kind;
                    Select::new(&mut form.kind, &[(Kind::File, "This file"), (Kind::Extension, "File extension"), (Kind::Folder, "Folder"), (Kind::Pattern, "Custom pattern")])
                        .label("Ignore").show(ui);
                    if form.kind == Kind::Folder && matches!(previous_kind, Kind::File | Kind::Extension) {
                        form.path = form.path.rsplit_once('/').map_or_else(String::new, |(parent, _)| parent.to_owned());
                    }
                    if form.kind == Kind::Pattern {
                        TextField::new(&mut form.pattern).label("Pattern").placeholder("e.g. *.log or /build/").show(ui);
                    }
                    if form.kind != Kind::Pattern || form.target == IgnoreTarget::Nearest {
                        TextField::new(&mut form.path).label(if form.kind == Kind::Pattern { "Path for nearest .gitignore" } else { "Relative path" }).show(ui);
                    }
                    let targets = [(IgnoreTarget::Root, ".gitignore at the root"), (IgnoreTarget::Nearest, "Nearest .gitignore"), (IgnoreTarget::RepositoryExclude, "This repository only"), (IgnoreTarget::Global, "All repositories")];
                    Select::new(&mut form.target, &targets).label("Save rule in").show(ui);
                }
                Page::Paths => {},
                Page::Why => {
                    TextField::new(&mut form.path).label("Relative path").placeholder("e.g. build/output.log").show(ui);
                }
            }
            let query = form.query();
            let Some(tab) = tab.filter(|tab| tab.path == form.repo) else {
                note(ui, "This repository is no longer open. Reopen it to manage its rules.", p.danger);
                return;
            };
            let Some(query) = query else { return; };
            if tab.ignore_query.as_ref() != Some(&query) {
                actions.push(Action::ReadIgnore { repo: form.repo.clone(), query });
                working(ui, "Checking ignore rules…");
                return;
            }
            match &tab.ignore_result {
                None => working(ui, "Checking ignore rules…"),
                Some(Err(error)) => note(ui, error, p.danger),
                Some(Ok(data)) => match data.as_ref() {
                    IgnoreData::Preview { file, preview, truncated } => {
                        if matches!(form.target, IgnoreTarget::Nearest | IgnoreTarget::Global) {
                            note(ui, format!("Save to {}", tilde(file.strip_prefix(&form.repo).unwrap_or(file))), p.text_dim);
                        }
                        if preview.ignored.is_empty() && preview.unignored.is_empty() {
                            note(ui, "No new matches in the files checked.", p.text_dim);
                        } else {
                            note(ui, "Preview", p.text);
                        }
                        if *truncated {
                            note(ui, "Preview is limited to the first 1,000 known paths.", p.warning);
                        }
                        egui::ScrollArea::vertical().id_salt("ignore-preview").max_height(120.0).show(ui, |ui| {
                            for path in &preview.ignored { note(ui, path, p.text_dim); }
                            for path in &preview.unignored { note(ui, format!("Visible: {path}"), p.text_dim); }
                        });
                    }
                    IgnoreData::Paths(listing) => {
                        if listing.paths.is_empty() { note(ui, "No ignored files.", p.text_dim); }
                        if listing.truncated { note(ui, "Showing the first 1,000 ignored paths.", p.warning); }
                        egui::ScrollArea::vertical().id_salt("ignored-paths").max_height(260.0).show(ui, |ui| {
                            for path in &listing.paths {
                                if ListRow::new(("ignored-path", path)).kind(RowKind::File)
                                    .icon(if path.ends_with('/') { icon::FOLDER } else { icon::FILE })
                                    .label(path).ground(p.s().elevated).tooltip(path).show(ui).clicked() {
                                    form.path.clone_from(path);
                                    form.page = Page::Why;
                                }
                            }
                        });
                    }
                    IgnoreData::Why { matched, tracked } => {
                        match matched {
                            None => note(ui, "No matching rule.", p.text_dim),
                            Some(matched) => {
                                note(ui, format!("{} by {}:{}", if matched.negated() { "Included" } else { "Ignored" }, matched.source, matched.line), p.text_dim);
                                note(ui, &matched.pattern, p.text);
                            }
                        }
                        if *tracked {
                            ui.add_space(theme::SM);
                            note(ui, "This path is tracked. Stop tracking it to let ignore rules take effect.", p.text_dim);
                            if Button::new("Stop tracking…").secondary().size(Size::Sm).enabled(tab.running.is_none()).show(ui).clicked() {
                                actions.push(Action::OpenDialog(Dialog::Confirm {
                                    title: format!("Stop tracking {}?", form.path.trim()),
                                    body: "Files on disk are kept. Their removal from Git is staged for commit; collaborators will lose the tracked copy when they pull that commit.".into(),
                                    confirm: "Stop tracking".into(), danger: true,
                                    repo: form.repo.clone(), op: Op::StopTracking(form.path.trim().into()),
                                }));
                            }
                        }
                    }
                },
            }
        })
        .footer(|footer| {
            if footer_page == Page::Rule {
                confirm(footer, "Add rule", can_add, "Enter a valid rule and wait for its preview");
            } else { footer.cancel().cancel_label("Close"); }
        }).show(ctx);
    match result {
        DialogResult::Confirm
            if can_add && form.page == Page::Rule && form.query() == validated_query =>
        {
            Outcome::Run(vec![Action::OpIn {
                repo: form.repo.clone(),
                op: Op::AddIgnore {
                    rule: form.rule(),
                    target: form.target,
                },
            }])
        }
        DialogResult::Cancel => Outcome::Close,
        _ if !actions.is_empty() => Outcome::RunKeep(actions),
        _ => Outcome::Keep,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_validated_ignore_rule_enables_add_and_an_edited_rule_disables_it() {
        use crate::ui::kit::testkit::spoken;
        let ctx = egui::Context::default();
        crate::ui::fonts::install(&ctx);
        theme::apply(&ctx, theme::ThemeId::Midnight, theme::Appearance::STILL);
        let (worker, _, _) = crate::worker::Worker::detached();
        let mut tab = RepoTab::with_worker(
            PathBuf::from("fixture"),
            100,
            crate::settings::DiffPrefs::default(),
            worker,
        );
        let mut form = IgnoreForm::new(tab.path.clone(), None);
        form.pattern = "*.log".into();
        tab.ignore_query = form.query();
        tab.ignore_result = Some(Ok(std::sync::Arc::new(IgnoreData::Preview {
            file: tab.path.join(".gitignore"),
            preview: canopy_core::IgnorePreview::default(),
            truncated: false,
        })));
        let salt = salt(&Dialog::Ignore(Box::new(form.clone())));
        let button_enabled = |form: &mut IgnoreForm| {
            let mut nodes = Vec::new();
            for _ in 0..3 {
                nodes = spoken(&ctx, |_| {
                    show(&ctx, salt, form, Some(&tab));
                });
            }
            let add = nodes
                .iter()
                .find(|node| {
                    node.role == egui::accesskit::Role::Button
                        && node
                            .label
                            .as_deref()
                            .is_some_and(|label| label.starts_with("Add rule"))
                })
                .unwrap_or_else(|| panic!("Add rule button in {nodes:?}"));
            !add.disabled
        };
        assert!(
            button_enabled(&mut form),
            "the completed preview enables the action"
        );
        form.pattern = "*.tmp".into();
        assert!(
            !button_enabled(&mut form),
            "a different rule needs a new preview"
        );
    }
}
