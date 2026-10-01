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

    /// The URL to use, if complete.
    pub fn resolve(&self) -> Option<String> {
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
        ui.horizontal(|ui| {
            for (s, glyph, label) in [
                (UrlSource::GitHub, icon::GITHUB_LOGO, "GitHub"),
                (UrlSource::GitLab, icon::GITLAB_LOGO_SIMPLE, "GitLab"),
                (UrlSource::Url, icon::LINK, "URL"),
            ] {
                ui.selectable_value(&mut self.source, s, format!("{glyph}  {label}"));
            }
        });
        ui.add_space(8.0);
        match self.source {
            UrlSource::Url => {
                field(
                    ui,
                    "Repository URL",
                    &mut self.url,
                    "https://host/owner/repo.git or git@host:owner/repo.git",
                    focus,
                );
            }
            UrlSource::GitHub | UrlSource::GitLab => {
                if self.source == UrlSource::GitLab {
                    field(
                        ui,
                        "Host (empty for gitlab.com)",
                        &mut self.host,
                        "gitlab.example.com",
                        false,
                    );
                }
                let hint = if self.source == UrlSource::GitHub {
                    "owner/repository"
                } else {
                    "group/subgroup/project"
                };
                field(ui, "Project", &mut self.project, hint, focus);
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut self.ssh, false, "HTTPS");
                    ui.selectable_value(&mut self.ssh, true, "SSH");
                });
                ui.add_space(4.0);
                match self.resolve() {
                    Some(u) => {
                        ui.label(RichText::new(u).monospace().size(11.5).color(p.text_faint))
                    }
                    None if !self.project.trim().is_empty() => ui.label(
                        RichText::new("Use the form owner/repository")
                            .size(12.0)
                            .color(p.danger),
                    ),
                    None => ui.label(""),
                };
                ui.add_space(4.0);
            }
        }
    }
}
