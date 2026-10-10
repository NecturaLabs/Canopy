//! The dialog stack: what is on screen and what waits behind it.

use super::*;

impl Dialog {
    /// How strongly the dialog holds the screen. A dialog never hides one that ranks higher: it
    /// waits behind it instead.
    ///
    /// - 2: git's and ssh's credential questions. Git is blocked until they are answered, so
    ///   they go in front of everything, one at a time in the order they arrived.
    /// - 1: the start-up consent, which must be answered, and the legal documents.
    /// - 0: everything else; the newest shows first.
    pub(super) fn rank(&self) -> u8 {
        match self {
            Self::Credential { .. } => 2,
            Self::Consent { .. } | Self::Legal(_) => 1,
            _ => 0,
        }
    }
}

/// The dialog on screen and those waiting behind it. Opening never throws a dialog away, and a
/// credential prompt that leaves the stack, however it leaves, always gets an answer.
#[derive(Debug, Default)]
pub struct DialogStack {
    /// Bottom first; the last one is on screen.
    dialogs: Vec<Dialog>,
}

impl DialogStack {
    /// Shows `dialog`, or queues it behind the dialogs that outrank it (see [`Dialog::rank`]).
    pub fn open(&mut self, dialog: Dialog) {
        let rank = dialog.rank();
        // Credential prompts queue in arrival order behind the one on screen; other dialogs go
        // on top of their equals.
        let waits_behind = |d: &Dialog| d.rank() > rank || (rank == 2 && d.rank() == 2);
        let mut at = self.dialogs.len();
        while at > 0 && waits_behind(&self.dialogs[at - 1]) {
            at -= 1;
        }
        self.dialogs.insert(at, dialog);
    }

    /// The dialog on screen.
    pub fn current(&self) -> Option<&Dialog> {
        self.dialogs.last()
    }

    pub fn current_mut(&mut self) -> Option<&mut Dialog> {
        self.dialogs.last_mut()
    }

    /// Whether any open or waiting dialog matches.
    pub fn any(&self, f: impl Fn(&Dialog) -> bool) -> bool {
        self.dialogs.iter().any(f)
    }

    /// Closes the dialog on screen; the next one shows. A credential prompt not answered yet is
    /// cancelled.
    pub fn close(&mut self) {
        if let Some(d) = self.dialogs.pop() {
            cancel(&d);
        }
    }

    /// Escape: closes the dialog on screen, except the consent, which must be answered (a
    /// document read inside it goes back to the summary). A credential prompt is declined (`no`
    /// to an unknown host key).
    pub fn dismiss(&mut self) {
        match self.dialogs.last_mut() {
            Some(Dialog::Consent { reading, .. }) => *reading = None,
            None => {}
            Some(Dialog::Credential { prompt, .. }) => {
                prompt.decline();
                self.close();
            }
            Some(_) => self.close(),
        }
    }

    /// App-level Escape also returns the closed dialog's keyboard focus.
    pub fn dismiss_with_focus(&mut self, ctx: &egui::Context) {
        let closing = self
            .current()
            .filter(|dialog| !matches!(dialog, Dialog::Consent { .. }))
            .map(salt);
        self.dismiss();
        if let Some(salt) = closing {
            KitDialog::restore_focus(ctx, salt);
        }
    }

    /// Closes every dialog that matches, wherever it is (credential prompts are cancelled).
    pub fn close_where(&mut self, f: impl Fn(&Dialog) -> bool) {
        self.dialogs.retain(|d| {
            let close = f(d);
            if close {
                cancel(d);
            }
            !close
        });
    }

    /// Keeps a failed clone's input and shows why it failed (git's message) in the form that
    /// started it, or closes the submitted form after success. Whether such a form was waiting
    /// (else the caller reports it).
    pub fn finish_clone_with(&mut self, result: Result<(), &str>) -> bool {
        let mut found = false;
        self.dialogs.retain_mut(|dialog| {
            if let Dialog::Clone { pending, error, .. } = dialog
                && *pending
            {
                found = true;
                *pending = false;
                if let Err(message) = result
                    && !message.is_empty()
                {
                    *error = Some(message.to_owned());
                }
                return result.is_err();
            }
            true
        });
        found
    }

    /// Hands the opener's answer for `path` to the open (or, when `create`, new-repository)
    /// dialog that asked for it: it closes once the repository opened, and shows why otherwise.
    /// Whether such a dialog was waiting (else the caller reports the failure itself).
    pub fn finish_open(
        &mut self,
        path: &std::path::Path,
        create: bool,
        result: Result<(), &OpenFailure>,
    ) -> bool {
        let asked = |d: &Dialog| match d {
            Dialog::Open { pending, .. } if !create => pending.as_deref() == Some(path),
            Dialog::NewRepository { pending, .. } if create => pending.as_deref() == Some(path),
            _ => false,
        };
        let Some(at) = self.dialogs.iter().position(asked) else {
            return false;
        };
        match result {
            Ok(()) => {
                self.dialogs.remove(at);
            }
            Err(failure) => match &mut self.dialogs[at] {
                Dialog::Open { pending, error, .. } => {
                    *pending = None;
                    *error = Some(failure.clone());
                }
                Dialog::NewRepository { pending, error, .. } => {
                    *pending = None;
                    *error = Some(failure.message.clone());
                }
                _ => {}
            },
        }
        true
    }

    /// Closes the credential prompts git or ssh stopped waiting for.
    pub fn drop_withdrawn(&mut self) {
        self.close_where(
            |d| matches!(d, Dialog::Credential { prompt, .. } if !prompt.is_waiting()),
        );
    }
}

/// Cancels `dialog`'s credential prompt if it has one; a prompt already answered keeps its
/// answer.
pub(super) fn cancel(dialog: &Dialog) {
    if let Dialog::Credential { prompt, .. } = dialog {
        prompt.answer(None);
    }
}
