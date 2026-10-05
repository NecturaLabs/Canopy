//! Git's and ssh's credential prompts.

use super::*;

/// The credential prompt: the question as git or ssh asked it, and the answer.
pub(super) fn credential(
    ui: &mut Ui,
    prompt: &Prompt,
    answer: &mut String,
    reveal: &mut bool,
    enter: bool,
) -> Outcome {
    let p = palette(ui.ctx());
    let lower = prompt.text.to_lowercase();
    let passphrase = lower.contains("passphrase");
    let (glyph, color, heading) = match prompt.kind {
        PromptKind::Secret if passphrase => (icon::KEY, p.accent, "Unlock SSH key"),
        PromptKind::Secret => (icon::KEY, p.accent, "Password required"),
        PromptKind::Text => (icon::USER, p.accent, "Sign in"),
        PromptKind::YesNo => (icon::SHIELD_WARNING, p.warning, "Trust this host?"),
        PromptKind::Approve | PromptKind::Notice => (icon::SHIELD_CHECK, p.accent, "Confirm"),
    };
    ui.horizontal(|ui| {
        ui.label(RichText::new(glyph).size(theme::ICON_TITLE).color(color));
        ui.label(RichText::new(heading).font(FontId::new(theme::TITLE_SIZE, theme::semibold())));
    });
    ui.add_space(10.0);
    let asker = if lower.contains("passphrase for key")
        || lower.contains("authenticity of host")
        || lower.contains("host key")
    {
        "SSH asks:"
    } else {
        "Git asks:"
    };
    if prompt.kind == PromptKind::YesNo {
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
        if let Some(host) = host {
            ui.label(
                RichText::new(host).font(FontId::new(theme::SUBTITLE_SIZE, theme::semibold())),
            );
        }
        if let Some(fingerprint) = fingerprint {
            ui.label(
                RichText::new("SSH host fingerprint")
                    .size(theme::META_SIZE)
                    .color(p.text_dim),
            );
            ui.add(
                egui::Label::new(
                    RichText::new(fingerprint).font(FontId::monospace(theme::MONO_SIZE)),
                )
                .wrap(),
            );
            if ui
                .button(format!("{}  Copy fingerprint", icon::COPY))
                .clicked()
            {
                ui.ctx().copy_text(fingerprint.to_owned());
            }
        }
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
        if let Some((label, url)) = published {
            ui.hyperlink_to(label, url);
        }
        egui::CollapsingHeader::new("SSH details")
            .default_open(fingerprint.is_none())
            .show(ui, |ui| {
                ui.add(
                    egui::Label::new(
                        RichText::new(&prompt.text)
                            .font(FontId::monospace(theme::MONO_SMALL_SIZE))
                            .color(p.text_dim),
                    )
                    .wrap(),
                );
            });
    } else {
        ui.label(
            RichText::new(asker)
                .size(theme::META_SIZE)
                .color(p.text_dim),
        );
        ui.add_space(4.0);
        egui::Frame::new()
            .fill(p.panel())
            .stroke(egui::Stroke::new(1.0, p.border()))
            .corner_radius(egui::CornerRadius::same(theme::RADIUS))
            .inner_margin(egui::Margin::same(10))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                egui::ScrollArea::vertical()
                    .max_height(180.0)
                    .show(ui, |ui| {
                        ui.add(
                            egui::Label::new(
                                RichText::new(&prompt.text)
                                    .font(FontId::monospace(theme::MONO_SIZE))
                                    .color(p.text),
                            )
                            .wrap(),
                        );
                    });
            });
    }
    ui.add_space(12.0);
    let escape = ui.input(|i| i.key_pressed(Key::Escape));
    let mut outcome = Outcome::Keep;
    let reply = |value: Option<String>| {
        prompt.answer(value);
        Outcome::Close
    };
    match prompt.kind {
        PromptKind::Secret | PromptKind::Text => {
            let secret = prompt.kind == PromptKind::Secret;
            ui.label(
                RichText::new(match (secret, passphrase) {
                    (true, true) => "Passphrase",
                    (true, false) => "Password",
                    (false, _) => "Username",
                })
                .size(theme::META_SIZE)
                .color(p.text_dim),
            );
            ui.horizontal(|ui| {
                let width = ui.available_width() - if secret { 40.0 } else { 0.0 };
                let r = ui.add(
                    egui::TextEdit::singleline(answer)
                        .password(secret && !*reveal)
                        .desired_width(width)
                        .margin(theme::FIELD_MARGIN),
                );
                if ui.memory(|m| m.focused().is_none()) {
                    r.request_focus();
                }
                if secret {
                    let (eye, tip) = if *reveal {
                        (icon::EYE_SLASH, "Hide")
                    } else {
                        (icon::EYE, "Show")
                    };
                    if ui
                        .add_sized([32.0, 30.0], egui::Button::new(eye))
                        .on_hover_text(tip)
                        .clicked()
                    {
                        *reveal = !*reveal;
                    }
                }
            });
            ui.add_space(4.0);
            ui.label(RichText::new("Canopy hands this to git and does not keep it. Your git credential helper may save it.").size(theme::META_SIZE).color(p.text_faint));
            ui.add_space(14.0);
            ui.horizontal(|ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let ok = !answer.is_empty() || secret;
                    if widgets::primary_button(
                        ui,
                        if passphrase { "Unlock key" } else { "Continue" },
                        ok,
                        130.0,
                    )
                    .clicked()
                        || (enter && ok)
                    {
                        outcome = reply(Some(std::mem::take(answer)));
                    }
                    if ui
                        .add_sized([90.0, 34.0], egui::Button::new("Cancel"))
                        .clicked()
                        || escape
                    {
                        outcome = reply(None);
                    }
                });
            });
        }
        PromptKind::YesNo | PromptKind::Approve | PromptKind::Notice => {
            if prompt.kind == PromptKind::YesNo {
                ui.label(RichText::new("Only trust the host if its fingerprint matches the one its owner publishes (GitHub and GitLab list theirs in their documentation).").size(theme::META_SIZE).color(p.text_dim));
                ui.add_space(14.0);
            }
            let (yes, no, yes_answer) = if prompt.kind == PromptKind::YesNo {
                ("Trust host", "Don't trust", "yes")
            } else {
                ("Allow", "Deny", "")
            };
            ui.horizontal(|ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let accept = if prompt.kind == PromptKind::YesNo {
                        ui.add_sized([130.0, 34.0], egui::Button::new(yes))
                    } else {
                        widgets::accent_button(ui, yes, egui::vec2(130.0, 34.0))
                    };
                    if accept.clicked() {
                        outcome = reply(Some(yes_answer.to_owned()));
                    }
                    let reject = ui.add_sized([130.0, 34.0], egui::Button::new(no));
                    if prompt.kind == PromptKind::YesNo
                        && ui.memory(|memory| memory.focused().is_none())
                    {
                        reject.request_focus();
                    }
                    if reject.clicked() || escape {
                        prompt.decline();
                        outcome = Outcome::Close;
                    }
                });
            });
        }
    }
    outcome
}
