//! Creating pull and merge requests and checking them out.

use super::*;

impl CanopyApp {
    /// Opens the new pull/merge request dialog, or the provider's page when the API is off or
    /// has no token.
    pub(super) fn create_request(&mut self, ctx: &egui::Context, source: &str) {
        let Some(t) = self.tabs.get(self.active) else {
            return;
        };
        let Some((remote, forge)) = t.forge().cloned() else {
            return;
        };
        let target = t
            .snap
            .as_ref()
            .and_then(|s| s.remotes.iter().find(|r| r.name == remote))
            .and_then(|r| r.default_branch.clone());
        let local = t
            .snap
            .as_ref()
            .and_then(|s| s.branches.iter().find(|b| b.name == source));
        // The request's head is the branch on the remote: a local `topic` tracking
        // `origin/feature/topic` opens a request from `feature/topic`.
        let prefix = format!("{remote}/");
        let source = local
            .and_then(|b| b.upstream.as_deref())
            .and_then(|u| u.strip_prefix(&prefix))
            .map_or_else(|| source.to_owned(), str::to_owned);
        let head_commit = local.and_then(|b| b.target);
        let has_token = matches!(t.requests.token, Some(Some(_)));
        if !self.settings.hosting_api || !has_token {
            let url = forge.new_request_url(&source, target.as_deref());
            if let Err(e) = crate::platform::open_url(&url) {
                self.toasts
                    .push(ctx, ToastKind::Error, "Could not open the browser", e);
            }
            return;
        }
        // Prefill from the branch's newest commit: its summary and body.
        let (title, body) = head_commit
            .and_then(|oid| t.snap.as_ref()?.commits.iter().find(|c| c.oid == oid))
            .map(|c| (c.summary.clone(), String::new()))
            .unwrap_or_default();
        let branches: Vec<String> = t
            .snap
            .as_ref()
            .map(|s| {
                s.remote_branches
                    .iter()
                    .filter(|b| b.remote == remote)
                    .map(|b| b.branch.clone())
                    .collect()
            })
            .unwrap_or_default();
        self.dialogs.open(Dialog::NewRequest {
            kind: forge.kind,
            request: NewRequest {
                title,
                body,
                source,
                target: target.unwrap_or_else(|| "main".into()),
                draft: false,
            },
            branches,
        });
    }

    pub(super) fn checkout_request(&mut self, r: RequestSummary) {
        let Some(t) = self.tabs.get(self.active) else {
            return;
        };
        let Some((remote, forge)) = t.forge().cloned() else {
            return;
        };
        let same_repo_branch = !r.from_fork
            && t.snap.as_ref().is_some_and(|s| {
                s.remote_branches
                    .iter()
                    .any(|b| b.remote == remote && b.branch == r.source_branch)
            });
        let local_exists = t
            .snap
            .as_ref()
            .is_some_and(|s| s.branches.iter().any(|b| b.name == r.source_branch));
        let op = if same_repo_branch && local_exists {
            Op::Checkout(r.source_branch)
        } else if same_repo_branch {
            Op::CheckoutRemote {
                remote,
                branch: r.source_branch,
            }
        } else {
            let prefix = match forge.kind {
                canopy_core::forge::ForgeKind::GitHub => "pr",
                canopy_core::forge::ForgeKind::GitLab => "mr",
            };
            Op::CheckoutRequest {
                remote,
                remote_ref: canopy_hosting::checkout_ref(forge.kind, r.number),
                branch: format!("{prefix}/{}", r.number),
            }
        };
        if let Some(t) = self.tab() {
            t.op(op);
        }
    }
}
