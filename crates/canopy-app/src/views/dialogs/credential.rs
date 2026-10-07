//! Git's and ssh's credential prompts.

use super::*;

/// The tallest git's question grows before it scrolls.
const QUESTION_HEIGHT: f32 = 180.0;

/// The credential prompt: the question as git or ssh asked it, and the answer. It is answered
/// with its buttons (or Esc, which declines through the dialog stack), never by a stray click on
/// the scrim.
pub(super) fn credential(
    ctx: &egui::Context,
    salt: impl std::hash::Hash + std::fmt::Debug,
    prompt: &Prompt,
    answer: &mut String,
    serial: u64,
) -> Outcome {
    let p = palette(ctx);
    let m = theme::metrics();
    let lower = prompt.text.to_lowercase();
    let passphrase = lower.contains("passphrase");
    let (glyph, heading) = match prompt.kind {
        PromptKind::Secret if passphrase => (icon::KEY, "Unlock SSH key"),
        PromptKind::Secret => (icon::KEY, "Password required"),
        PromptKind::Text => (icon::USER, "Sign in"),
        PromptKind::YesNo => (icon::SHIELD_WARNING, "Trust this host?"),
        PromptKind::Approve | PromptKind::Notice => (icon::SHIELD_CHECK, "Confirm"),
    };
    let asker = if lower.contains("passphrase for key")
        || lower.contains("authenticity of host")
        || lower.contains("host key")
    {
        "SSH asks:"
    } else {
        "Git asks:"
    };
    let mut extra: Option<Outcome> = None;
    let dialog = KitDialog::new(salt)
        .title(heading)
        .icon(glyph)
        .width(DialogWidth::Form)
        .blocking();
    let reply = |value: Option<String>| {
        prompt.answer(value);
        Outcome::Close
    };
    match prompt.kind {
        PromptKind::Secret | PromptKind::Text => {
            let secret = prompt.kind == PromptKind::Secret;
            let ok = !answer.is_empty() || secret;
            let result = dialog
                .initial_focus(InitialFocus::Body)
                .body(|ui| {
                    note(ui, asker, p.text_dim);
                    question(ui, &prompt.text);
                    let r = TextField::new(answer)
                        .label(match (secret, passphrase) {
                            (true, true) => "Passphrase",
                            (true, false) => "Password",
                            (false, _) => "Username",
                        })
                        .password(secret)
                        .id_salt(("credential-answer", serial))
                        .show(ui);
                    if !r.lost_focus() && ui.memory(|mem| mem.focused().is_none()) {
                        r.request_focus();
                    }
                    note(
                        ui,
                        "Canopy hands this to git and does not keep it. Your git credential helper may save it.",
                        p.text_faint,
                    );
                })
                .footer(|f| {
                    confirm(
                        f,
                        if passphrase { "Unlock key" } else { "Continue" },
                        ok,
                        "Enter a username",
                    );
                })
                .show(ctx);
            match result {
                DialogResult::Cancel => reply(None),
                DialogResult::Confirm if !answer.is_empty() || secret => {
                    reply(Some(std::mem::take(answer)))
                }
                DialogResult::Open | DialogResult::Confirm => Outcome::Keep,
            }
        }
        PromptKind::YesNo => {
            let host = if lower.contains("authenticity of host") {
                prompt
                    .text
                    .split('\'')
                    .nth(1)
                    .and_then(|host| host.split_whitespace().next())
            } else {
                None
            };
            let fingerprint = prompt
                .text
                .split_whitespace()
                .find(|word| word.starts_with("SHA256:") || word.starts_with("MD5:"))
                .map(|word| word.trim_end_matches('.'));
            let published = match host {
                Some("github.com") => Some((
                    "Compare with GitHub's fingerprints",
                    "https://docs.github.com/en/authentication/keeping-your-account-and-data-secure/githubs-ssh-key-fingerprints",
                )),
                Some("gitlab.com") => Some((
                    "Compare with GitLab.com's fingerprints",
                    "https://docs.gitlab.com/user/gitlab_com/#ssh-host-keys-fingerprints",
                )),
                _ => None,
            };
            let details_id = egui::Id::new(("credential-ssh-details", &prompt.text));
            let mut details = ctx
                .data(|d| d.get_temp::<bool>(details_id))
                .unwrap_or(fingerprint.is_none());
            let result = dialog
                .initial_focus(InitialFocus::Cancel)
                .body(|ui| {
                    if let Some(host) = host {
                        ui.label(
                            RichText::new(host)
                                .font(theme::heading(m.subtitle_size))
                                .color(p.text),
                        );
                    }
                    if let Some(fingerprint) = fingerprint {
                        note(ui, "SSH host fingerprint", p.text_dim);
                        InsetBox::new().show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.add(
                                egui::Label::new(
                                    RichText::new(fingerprint)
                                        .font(FontId::monospace(m.mono_size))
                                        .color(p.text),
                                )
                                .wrap(),
                            );
                        });
                        if Button::new("Copy fingerprint")
                            .secondary()
                            .size(Size::Sm)
                            .icon(icon::COPY)
                            .show(ui)
                            .clicked()
                        {
                            ui.ctx().copy_text(fingerprint.to_owned());
                        }
                    }
                    if let Some((label, url)) = published
                        && LinkButton::new(label)
                            .url(url)
                            .text_size(m.meta_size)
                            .show(ui)
                            .clicked()
                    {
                        extra = Some(Outcome::RunKeep(vec![Action::OpenUrl(url.to_owned())]));
                    }
                    Disclosure::new("ssh-details", "SSH details", &mut details).show(ui, |ui| {
                        ui.add(
                            egui::Label::new(
                                RichText::new(&prompt.text)
                                    .font(FontId::monospace(m.small_size))
                                    .color(p.text_dim),
                            )
                            .wrap(),
                        );
                    });
                    note(
                        ui,
                        "Trust the host only if its fingerprint matches the one its owner publishes.",
                        p.text_dim,
                    );
                })
                .footer(|f| {
                    f.choice("Trust host", "Don't trust");
                })
                .show(ctx);
            ctx.data_mut(|d| d.insert_temp(details_id, details));
            match result {
                DialogResult::Open => extra.unwrap_or(Outcome::Keep),
                DialogResult::Confirm => reply(Some("yes".to_owned())),
                DialogResult::Cancel => {
                    prompt.decline();
                    Outcome::Close
                }
            }
        }
        PromptKind::Approve | PromptKind::Notice => {
            swallow_enter(ctx);
            let result = dialog
                .initial_focus(InitialFocus::Body)
                .body(|ui| {
                    note(ui, asker, p.text_dim);
                    question(ui, &prompt.text);
                })
                .footer(|f| {
                    f.cancel_label("Deny").confirm("Allow");
                })
                .show(ctx);
            match result {
                DialogResult::Open => Outcome::Keep,
                DialogResult::Confirm => reply(Some(String::new())),
                DialogResult::Cancel => {
                    prompt.decline();
                    Outcome::Close
                }
            }
        }
    }
}

/// The question as git or ssh wrote it, in a recessed box that scrolls when long.
fn question(ui: &mut Ui, text: &str) {
    let p = palette(ui.ctx());
    let m = theme::metrics();
    inset_scroll(ui, "credential-question", QUESTION_HEIGHT, |ui| {
        ui.add(
            egui::Label::new(
                RichText::new(text)
                    .font(FontId::monospace(m.mono_size))
                    .color(p.text),
            )
            .wrap(),
        );
    });
}
