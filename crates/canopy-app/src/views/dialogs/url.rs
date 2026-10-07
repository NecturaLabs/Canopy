//! The remote URL builder shared by the clone and add-remote dialogs.

use super::*;

/// Where a remote URL comes from in the clone and add-remote dialogs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UrlSource {
    GitHub,
    GitLab,
    Url,
}

/// Builds a remote URL from `owner/repo` (GitHub, GitLab) or takes it verbatim.
#[derive(Debug, Clone)]
pub struct UrlBuilder {
    pub source: UrlSource,
    pub url: String,
    pub project: String,
    /// GitLab host; empty means gitlab.com.
    pub host: String,
    pub ssh: bool,
}

impl UrlBuilder {
    pub(super) fn new(settings: &Settings) -> Self {
        Self {
            source: UrlSource::GitHub,
            url: String::new(),
            project: String::new(),
            host: String::new(),
            ssh: settings.clone_protocol == CloneProtocol::Ssh,
        }
    }

    /// The URL to use, if complete and acceptable ([`UrlBuilder::problem`]).
    pub fn resolve(&self) -> Option<String> {
        if self.host_problem().is_some() {
            return None;
        }
        self.built()
            .filter(|u| canopy_core::ops::remote_url_problem(u).is_none())
    }

    /// Why the URL typed or built cannot be used, such as credentials in it.
    pub fn problem(&self) -> Option<&'static str> {
        self.host_problem().or_else(|| {
            self.built()
                .and_then(|u| canopy_core::ops::remote_url_problem(&u))
        })
    }

    /// A GitLab host with user information must not silently lose credentials while normalising.
    fn host_problem(&self) -> Option<&'static str> {
        (self.source == UrlSource::GitLab && self.host.contains('@'))
            .then_some("Remove the user name and password from the host.")
    }

    /// The URL as typed or built, before it is checked.
    fn built(&self) -> Option<String> {
        let kind = match self.source {
            UrlSource::Url => {
                let u = self.url.trim();
                return (!u.is_empty()).then(|| u.to_owned());
            }
            UrlSource::GitHub => ForgeKind::GitHub,
            UrlSource::GitLab => ForgeKind::GitLab,
        };
        let host = (kind == ForgeKind::GitLab)
            .then(|| self.host.trim())
            .filter(|h| !h.is_empty());
        forge::clone_url(kind, host, self.project.trim(), self.ssh)
    }

    pub(super) fn ui(&mut self, ui: &mut Ui, focus: bool) {
        let p = palette(ui.ctx());
        let m = theme::metrics();
        Segmented::with_icons(
            &mut self.source,
            &[
                (UrlSource::GitHub, icon::GITHUB_LOGO, "GitHub"),
                (UrlSource::GitLab, icon::GITLAB_LOGO_SIMPLE, "GitLab"),
                (UrlSource::Url, icon::LINK, "URL"),
            ],
        )
        .label("Where the repository is")
        .id_salt("url-source")
        .show(ui);
        match self.source {
            UrlSource::Url => {
                let problem = self.problem();
                let r = TextField::new(&mut self.url)
                    .label("Repository URL")
                    .placeholder("https://host/owner/repo.git or git@host:owner/repo.git")
                    .helper("Copy it from the repository's Clone button on its host.")
                    .error(problem)
                    .id_salt(("dialog-field", "Repository URL"))
                    .show(ui);
                take_focus(ui, &r, focus);
            }
            UrlSource::GitHub | UrlSource::GitLab => {
                if self.source == UrlSource::GitLab {
                    let host_error = self.host_problem();
                    field_with_error(
                        ui,
                        "Host (empty for gitlab.com)",
                        &mut self.host,
                        "gitlab.example.com",
                        false,
                        host_error,
                    );
                }
                let form = if self.source == UrlSource::GitHub {
                    "owner/repository"
                } else {
                    "group/subgroup/project"
                };
                let error = if self.host_problem().is_some() {
                    None
                } else {
                    self.problem().map(str::to_owned).or_else(|| {
                        (self.built().is_none() && !self.project.trim().is_empty())
                            .then(|| format!("Use the form {form}"))
                    })
                };
                let helper = if self.ssh {
                    "SSH: your SSH key signs you in."
                } else {
                    "HTTPS: a token or your Git credential helper signs you in."
                };
                let project = &mut self.project;
                let ssh = &mut self.ssh;
                let (r, ()) = beside(
                    ui,
                    "url-project",
                    |ui, width| {
                        TextField::new(project)
                            .label("Project")
                            .placeholder(form)
                            .helper(helper)
                            .error(error.as_deref())
                            .width(width)
                            .id_salt(("dialog-field", "Project"))
                            .show(ui)
                    },
                    |ui| {
                        Segmented::new(ssh, &[(false, "HTTPS"), (true, "SSH")])
                            .label("Protocol")
                            .option_tooltip(0, "Clone over HTTPS")
                            .option_tooltip(1, "Clone over SSH")
                            .id_salt("url-protocol")
                            .show(ui);
                    },
                );
                take_focus(ui, &r, focus);
            }
        }
        if let Some(u) = self.resolve() {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = theme::SM;
                ui.label(
                    RichText::new(icon::LINK)
                        .size(m.icon_sm)
                        .color(p.text_faint),
                );
                ui.add(
                    egui::Label::new(
                        RichText::new(u)
                            .font(FontId::monospace(m.small_size))
                            .color(p.text_dim),
                    )
                    .truncate(),
                );
            });
        }
    }
}

/// Gives `r` the keyboard when `focus` asks and nothing else has it (not in the frame Enter let
/// go of it, so Enter reaches the dialog).
fn take_focus(ui: &Ui, r: &egui::Response, focus: bool) {
    if focus && !r.lost_focus() && ui.memory(|m| m.focused().is_none()) {
        r.request_focus();
    }
}

#[cfg(test)]
mod tests {
    use super::{UrlBuilder, UrlSource};
    use crate::settings::Settings;

    #[test]
    fn credentials_in_gitlab_host_are_refused_at_host_field() {
        let mut builder = UrlBuilder::new(&Settings::default());
        builder.source = UrlSource::GitLab;
        builder.project = "team/repo".to_owned();

        for host in [
            "https://user:secret@gitlab.example.com",
            "user@gitlab.example.com",
        ] {
            builder.host = host.to_owned();
            assert_eq!(
                builder.host_problem(),
                Some("Remove the user name and password from the host.")
            );
            assert_eq!(builder.problem(), builder.host_problem());
            assert!(builder.resolve().is_none());
        }

        builder.host = "gitlab.example.com".to_owned();
        assert!(builder.host_problem().is_none());
        assert!(builder.problem().is_none());
        assert!(builder.resolve().is_some());
    }
}
