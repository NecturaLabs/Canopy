//! The Terms of Service and Privacy Policy as shipped, the versions the user accepts, and the
//! documents laid out for reading.
//!
//! The texts live in `docs/legal/`. When either changes in a way users must see, update its
//! "Last updated" date; the version below must match it (a test checks), and Canopy then asks
//! again on the next start.

use std::sync::{Arc, OnceLock};

use egui::text::{LayoutJob, TextFormat};
use egui::{
    Color32, CornerRadius, FontId, Galley, Rect, Response, Sense, Ui, WidgetInfo, WidgetType, pos2,
    vec2,
};

use egui_phosphor::regular as icon;

use crate::settings::Settings;
use crate::ui::kit::menu;
use crate::ui::kit::overlay::TooltipExt;
use crate::ui::kit::paint;
use crate::ui::theme::{self, palette};

pub const TERMS: &str = include_str!("../../../../docs/legal/terms-of-service.md");
pub const PRIVACY: &str = include_str!("../../../../docs/legal/privacy-policy.md");

/// "Last updated" of the Terms of Service, as `YYYY-MM-DD`.
pub const TERMS_VERSION: &str = "2026-10-03";
/// "Last updated" of the Privacy Policy, as `YYYY-MM-DD`.
pub const PRIVACY_VERSION: &str = "2026-10-10";

/// A list item's dot, and how far below the line's middle its x-height sits.
const BULLET_RADIUS: f32 = 2.5;
const BULLET_DROP: f32 = 1.0;

/// Which document to show.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Document {
    Terms,
    Privacy,
}

impl Document {
    pub fn title(self) -> &'static str {
        match self {
            Self::Terms => "Terms of Service",
            Self::Privacy => "Privacy Policy",
        }
    }

    /// The glyph that stands for the document.
    pub fn glyph(self) -> &'static str {
        match self {
            Self::Terms => egui_phosphor::regular::SCROLL,
            Self::Privacy => egui_phosphor::regular::SHIELD_CHECK,
        }
    }

    fn text(self) -> &'static str {
        match self {
            Self::Terms => TERMS,
            Self::Privacy => PRIVACY,
        }
    }

    /// The text without its first-level title (the dialog shows that).
    pub fn body(self) -> &'static str {
        let text = self.text();
        text.split_once('\n').map_or(text, |(_, rest)| rest).trim()
    }

    /// The "Last updated" date as the document writes it ("3 October 2026").
    pub fn updated(self) -> &'static str {
        self.text()
            .lines()
            .find_map(|l| l.strip_prefix("Last updated: "))
            .map_or("", str::trim)
    }

    /// The document's blocks below its title and date, parsed once.
    fn blocks(self) -> &'static [Block] {
        static TERMS_BLOCKS: OnceLock<Vec<Block>> = OnceLock::new();
        static PRIVACY_BLOCKS: OnceLock<Vec<Block>> = OnceLock::new();
        let cell = match self {
            Self::Terms => &TERMS_BLOCKS,
            Self::Privacy => &PRIVACY_BLOCKS,
        };
        cell.get_or_init(|| parse(self.body()))
    }
}

/// Where a link in a document leads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Link {
    /// A web page, opened in the browser.
    Web(String),
    /// An email address, copied (Canopy opens web addresses only).
    Email(String),
}

/// A stretch of a paragraph: plain text, or a link and its text.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Run {
    text: String,
    link: Option<Link>,
}

/// One block of a legal document.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Block {
    Heading(String),
    Paragraph(Vec<Run>),
    Item(Vec<Run>),
}

/// The blocks of a document body: `## ` headings, `- ` list items and paragraphs, with wrapped
/// lines joined to the block they continue. The "Last updated" line is left out: the reader
/// shows it under the title.
fn parse(body: &str) -> Vec<Block> {
    #[derive(PartialEq)]
    enum Kind {
        Heading,
        Paragraph,
        Item,
    }
    let mut raw: Vec<(Kind, String)> = Vec::new();
    let mut open = false;
    for line in body.lines() {
        let t = line.trim();
        if t.is_empty() {
            open = false;
            continue;
        }
        if t.starts_with("Last updated: ") {
            continue;
        }
        let (kind, text) = if let Some(h) = t.strip_prefix("## ").or_else(|| t.strip_prefix("# ")) {
            (Kind::Heading, h)
        } else if let Some(i) = t.strip_prefix("- ").or_else(|| t.strip_prefix("* ")) {
            (Kind::Item, i)
        } else {
            (Kind::Paragraph, t)
        };
        match raw.last_mut() {
            Some((_, last)) if open && kind == Kind::Paragraph => {
                last.push(' ');
                last.push_str(text);
            }
            _ => {
                open = kind != Kind::Heading;
                raw.push((kind, text.to_owned()));
            }
        }
    }
    raw.into_iter()
        .map(|(kind, text)| match kind {
            Kind::Heading => Block::Heading(text),
            Kind::Item => Block::Item(runs(&text)),
            Kind::Paragraph => Block::Paragraph(runs(&text)),
        })
        .collect()
}

/// Splits `text` into plain runs and links: web addresses (`https://…`) and email addresses, with
/// the sentence punctuation after them left in the text.
fn runs(text: &str) -> Vec<Run> {
    let mut out: Vec<Run> = Vec::new();
    let mut plain = String::new();
    for (i, word) in text.split(' ').enumerate() {
        if i > 0 {
            plain.push(' ');
        }
        let core = word.trim_end_matches(['.', ',', ';', ':', ')']);
        let tail = word.get(core.len()..).unwrap_or_default();
        let link = if core.starts_with("https://") || core.starts_with("http://") {
            Some(Link::Web(core.to_owned()))
        } else if is_email(core) {
            Some(Link::Email(core.to_owned()))
        } else {
            None
        };
        if let Some(link) = link {
            if !plain.is_empty() {
                out.push(Run {
                    text: std::mem::take(&mut plain),
                    link: None,
                });
            }
            out.push(Run {
                text: core.to_owned(),
                link: Some(link),
            });
            plain.push_str(tail);
        } else {
            plain.push_str(word);
        }
    }
    if !plain.is_empty() {
        out.push(Run {
            text: plain,
            link: None,
        });
    }
    out
}

fn is_email(word: &str) -> bool {
    let Some((user, host)) = word.split_once('@') else {
        return false;
    };
    !user.is_empty()
        && host.contains('.')
        && !host.starts_with('.')
        && !host.ends_with('.')
        && user
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "._+-".contains(c))
        && host
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || ".-".contains(c))
}

/// A document's body text: its font and line height (1.6 times the size).
fn body_font() -> (FontId, f32) {
    let size = theme::metrics().subtitle_size;
    (FontId::proportional(size), (size * 1.6).round())
}

/// Lays `doc` out at the width of `ui`: its title and date, then its headings, paragraphs and
/// lists with document spacing; links read and behave as kit links. Returns the link clicked
/// this frame.
pub fn show(ui: &mut Ui, doc: Document) -> Option<Link> {
    let p = palette(ui.ctx());
    let m = theme::metrics();
    let width = ui.available_width();
    let mut clicked = None;
    ui.spacing_mut().item_spacing.y = 0.0;
    ui.add(
        egui::Label::new(
            egui::RichText::new(doc.title())
                .font(theme::heading(m.display_size))
                .color(p.text),
        )
        .wrap(),
    );
    ui.add_space(theme::XS);
    ui.add(
        egui::Label::new(
            egui::RichText::new(format!("Last updated {}", doc.updated()))
                .size(m.meta_size)
                .color(p.text_faint),
        )
        .wrap(),
    );
    ui.add_space(theme::LG);
    let (font, line) = body_font();
    let indent = theme::LG + theme::XS;
    let mut previous: Option<&Block> = None;
    for (i, block) in doc.blocks().iter().enumerate() {
        match block {
            Block::Heading(text) => {
                if previous.is_some() {
                    ui.add_space(theme::XL);
                }
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(text)
                            .font(theme::heading(m.headline_size))
                            .color(p.text),
                    )
                    .wrap(),
                );
                ui.add_space(theme::SM);
            }
            Block::Paragraph(runs) => {
                if matches!(previous, Some(Block::Paragraph(_) | Block::Item(_))) {
                    ui.add_space(theme::MD);
                }
                clicked = paragraph(ui, (doc, i), runs, &font, line, width).or(clicked);
            }
            Block::Item(runs) => {
                ui.add_space(match previous {
                    Some(Block::Item(_)) => theme::XS,
                    Some(Block::Paragraph(_)) => theme::SM,
                    _ => 0.0,
                });
                let top = ui.cursor().top();
                let left = ui.cursor().left();
                let item = ui
                    .horizontal_top(|ui| {
                        ui.spacing_mut().item_spacing.x = 0.0;
                        ui.add_space(indent);
                        paragraph(ui, (doc, i), runs, &font, line, width - indent)
                    })
                    .inner;
                clicked = item.or(clicked);
                // A dot at the first line's x-height.
                ui.painter().circle_filled(
                    pos2(left + indent / 2.0, top + line / 2.0 + BULLET_DROP),
                    BULLET_RADIUS,
                    p.text_faint,
                );
            }
        }
        previous = Some(block);
    }
    clicked
}

/// One paragraph laid out as a single galley `width` wide, so all its lines share one height,
/// with its links painted and made clickable, focusable and announced as links.
fn paragraph(
    ui: &mut Ui,
    salt: impl std::hash::Hash + std::fmt::Debug,
    runs: &[Run],
    font: &FontId,
    line: f32,
    width: f32,
) -> Option<Link> {
    let p = palette(ui.ctx());
    let m = theme::metrics();
    let mut job = LayoutJob::default();
    job.wrap.max_width = width;
    let mut links: Vec<(std::ops::Range<usize>, &Link, &str)> = Vec::new();
    let mut chars = 0;
    for run in runs {
        let format = TextFormat {
            font_id: font.clone(),
            line_height: Some(line),
            color: if run.link.is_some() {
                p.accent2
            } else {
                p.text
            },
            ..TextFormat::default()
        };
        job.append(&run.text, 0.0, format.clone());
        let mut n = run.text.chars().count();
        if let Some(link) = &run.link {
            // A link that leaves Canopy ends in the arrow the kit's links end in.
            if matches!(link, Link::Web(_)) {
                let marker = TextFormat {
                    font_id: FontId::proportional(font.size * 0.85),
                    ..format
                };
                let tail = format!("\u{a0}{}", icon::ARROW_UP_RIGHT);
                n += tail.chars().count();
                job.append(&tail, 0.0, marker);
            }
            links.push((chars..chars + n, link, &run.text));
        }
        chars += n;
    }
    let galley: Arc<Galley> = ui.fonts_mut(|f| f.layout_job(job));
    let (rect, response) = ui.allocate_exact_size(galley.size(), Sense::hover());
    let text: String = runs.iter().map(|r| r.text.as_str()).collect();
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Label, true, &text));
    ui.painter().galley(rect.min, Arc::clone(&galley), p.text);
    let id = ui.id().with(("legal-paragraph", salt));
    let mut clicked = None;
    for (i, (range, link, label)) in links.into_iter().enumerate() {
        // One area per line the link runs over; the first is the focus stop and the one spoken.
        let areas = link_areas(&galley, &range, rect.min.to_vec2());
        let responses: Vec<_> = areas
            .iter()
            .enumerate()
            .map(|(k, area)| {
                let sense = if k == 0 { Sense::click() } else { Sense::CLICK };
                let r = ui.interact(*area, id.with((i, k)), sense);
                if k == 0 {
                    r.widget_info(|| WidgetInfo::labeled(WidgetType::Link, true, label));
                }
                r.on_hover_cursor(egui::CursorIcon::PointingHand)
            })
            .collect();
        let lit = responses.iter().any(|r| r.hovered() || r.has_focus());
        let focused = responses.iter().any(Response::has_focus);
        for area in &areas {
            // Underlined at rest, as a link in a block of text must be (WCAG 1.4.1); heavier
            // under the pointer and at focus. Under the glyphs, as the kit link draws it: the
            // row's text sits centred in the taller line.
            let baseline = area.center().y + font.size * 0.5;
            paint::seam_h(ui.painter(), area.x_range(), baseline, p.accent2);
            if lit {
                paint::seam_h(ui.painter(), area.x_range(), baseline + 1.0, p.accent2);
            }
            if focused {
                paint::focus_ring(
                    ui.painter(),
                    area.expand2(vec2(2.0, 0.0)),
                    CornerRadius::same(m.radius_sm),
                    p.focus,
                    Color32::TRANSPARENT,
                );
            }
        }
        let (tip, copy, destination) = match link {
            Link::Web(url) => (url.clone(), "Copy link", url),
            Link::Email(address) => ("Copy address".to_owned(), "Copy address", address),
        };
        for r in responses {
            let r = r.kit_tooltip(tip.clone()).show();
            menu::link_menu(&r, destination, copy);
            if r.clicked() {
                clicked = Some(link.clone());
            }
        }
    }
    clicked
}

/// Where the characters `range` of `galley` are, one rectangle for each line they run over,
/// moved by `by` (the galley's place on screen).
fn link_areas(galley: &Galley, range: &std::ops::Range<usize>, by: egui::Vec2) -> Vec<Rect> {
    let mut areas = Vec::new();
    let mut start = 0;
    for row in &galley.rows {
        let glyphs = row.glyphs.len();
        let (from, to) = (range.start.max(start), range.end.min(start + glyphs));
        if let (true, Some(first), Some(last)) = (
            from < to,
            row.glyphs.get(from.saturating_sub(start)),
            row.glyphs.get(to.saturating_sub(start + 1)),
        ) {
            let x = row.pos.x + first.pos.x..=row.pos.x + last.pos.x + last.advance_width;
            areas.push(Rect::from_x_y_ranges(x, row.rect().y_range()).translate(by));
        }
        start += glyphs + usize::from(row.ends_with_newline);
    }
    areas
}

/// What the start-up consent dialog still has to ask.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pending {
    /// The current terms and policy were not accepted yet.
    Terms,
    /// Only the crash-report question is open.
    CrashReports,
}

/// What Canopy must ask before it does anything else, if anything.
pub fn pending(s: &Settings) -> Option<Pending> {
    if s.accepted_terms != TERMS_VERSION || s.accepted_privacy != PRIVACY_VERSION {
        Some(Pending::Terms)
    } else if crate::crash_reports::available()
        && (s.crash_reports.is_none()
            // Agreed to send less than reports now carry: ask again.
            || (s.crash_reports == Some(true)
                && s.crash_reports_version < crate::crash_reports::DATA_VERSION))
    {
        Some(Pending::CrashReports)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn last_updated(text: &str) -> String {
        let line = text
            .lines()
            .find_map(|l| l.strip_prefix("Last updated: "))
            .expect("Last updated line");
        chrono::NaiveDate::parse_from_str(line.trim(), "%-d %B %Y")
            .expect("date")
            .format("%Y-%m-%d")
            .to_string()
    }

    #[test]
    fn versions_match_the_documents() {
        assert_eq!(last_updated(TERMS), TERMS_VERSION);
        assert_eq!(last_updated(PRIVACY), PRIVACY_VERSION);
        assert!(Document::Terms.body().starts_with("Last updated"));
        assert_eq!(Document::Terms.updated(), "3 October 2026");
    }

    #[test]
    fn documents_parse_into_blocks_without_markdown() {
        for doc in [Document::Terms, Document::Privacy] {
            let blocks = doc.blocks();
            assert!(blocks.iter().any(|b| matches!(b, Block::Heading(_))));
            assert!(blocks.iter().any(|b| matches!(b, Block::Item(_))));
            assert!(matches!(blocks.first(), Some(Block::Paragraph(_))));
            for block in blocks {
                let text: String = match block {
                    Block::Heading(t) => t.clone(),
                    Block::Paragraph(r) | Block::Item(r) => {
                        r.iter().map(|r| r.text.as_str()).collect()
                    }
                };
                assert!(
                    !text.starts_with('#') && !text.starts_with("- "),
                    "markdown left in {text}"
                );
                assert!(!text.starts_with("Last updated"), "{text}");
            }
        }
        let items = parse("- one\n  continued\n- two\n\nPara\nline");
        assert_eq!(items.len(), 3);
        assert!(
            matches!(&items[0], Block::Item(r) if r[0].text == "one continued"),
            "{items:?}"
        );
        assert!(matches!(&items[2], Block::Paragraph(r) if r[0].text == "Para line"));
    }

    #[test]
    fn links_are_found_without_their_punctuation() {
        let r = runs("Email support@necturalabs.com. See https://sentry.io/legal/subprocessors/.");
        let links: Vec<&Link> = r.iter().filter_map(|r| r.link.as_ref()).collect();
        assert_eq!(
            links,
            [
                &Link::Email("support@necturalabs.com".into()),
                &Link::Web("https://sentry.io/legal/subprocessors/".into()),
            ]
        );
        let text: String = r.iter().map(|r| r.text.as_str()).collect();
        assert_eq!(
            text,
            "Email support@necturalabs.com. See https://sentry.io/legal/subprocessors/."
        );
        assert!(!is_email("a@b"));
        assert!(!is_email("@example.com"));
        assert!(!is_email("user@host."));
        let privacy: Vec<Link> = Document::Privacy
            .blocks()
            .iter()
            .filter_map(|b| match b {
                Block::Paragraph(r) | Block::Item(r) => Some(r),
                Block::Heading(_) => None,
            })
            .flatten()
            .filter_map(|r| r.link.clone())
            .collect();
        assert!(privacy.contains(&Link::Web("https://sentry.io/legal/subprocessors/".into())));
        assert!(privacy.contains(&Link::Email("support@necturalabs.com".into())));
    }

    #[test]
    fn a_document_keeps_to_its_column() {
        let ctx = egui::Context::default();
        crate::ui::fonts::install(&ctx);
        for doc in [Document::Terms, Document::Privacy] {
            let mut width = 0.0;
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(500.0, 900.0))),
                    ..Default::default()
                },
                |ui| {
                    ui.set_width(420.0);
                    show(ui, doc);
                    width = ui.min_rect().width();
                },
            );
            output.textures_delta.clear();
            assert!(width <= 420.5, "{} grew to {width}", doc.title());
        }
        assert!(
            crate::ui::kit::overlay::reading_measure(&ctx) > 400.0,
            "a reading line holds about 70 characters"
        );
    }

    #[test]
    fn asks_until_everything_is_answered() {
        let mut s = Settings::default();
        assert_eq!(pending(&s), Some(Pending::Terms));
        s.accepted_terms = TERMS_VERSION.into();
        s.accepted_privacy = PRIVACY_VERSION.into();
        assert_eq!(pending(&s), Some(Pending::CrashReports));
        s.crash_reports = Some(false);
        assert_eq!(pending(&s), None);
        s.accepted_terms = "2020-01-01".into();
        assert_eq!(pending(&s), Some(Pending::Terms));
    }
    /// The Privacy Policy drawn in a tall window for a frame or more with `events`.
    fn privacy_frame(
        ctx: &egui::Context,
        events: Vec<egui::Event>,
        clicked: &mut Option<Link>,
    ) -> egui::FullOutput {
        let mut out = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(700.0, 6000.0))),
                events,
                ..Default::default()
            },
            |ui| {
                ui.set_width(600.0);
                *clicked = show(ui, Document::Privacy).or(clicked.take());
            },
        );
        out.textures_delta.clear();
        out
    }

    fn privacy_links() -> usize {
        Document::Privacy
            .blocks()
            .iter()
            .filter_map(|b| match b {
                Block::Paragraph(r) | Block::Item(r) => Some(r),
                Block::Heading(_) => None,
            })
            .flatten()
            .filter(|r| r.link.is_some())
            .count()
    }

    /// `(horizontal accent lines, shapes of text holding the external arrow)` in `shape`.
    fn count(shape: &egui::Shape, accent: Color32) -> (usize, usize) {
        match shape {
            egui::Shape::LineSegment { points, stroke } => (
                usize::from(stroke.color == accent && (points[0].y - points[1].y).abs() < 0.01),
                0,
            ),
            egui::Shape::Rect(r) if r.fill == accent && r.rect.height() <= 1.5 => (1, 0),
            egui::Shape::Text(t) => (
                0,
                usize::from(t.galley.text().contains(icon::ARROW_UP_RIGHT)),
            ),
            egui::Shape::Vec(v) => v
                .iter()
                .map(|s| count(s, accent))
                .fold((0, 0), |(a, b), (c, d)| (a + c, b + d)),
            _ => (0, 0),
        }
    }

    /// WCAG 1.4.1: a link in a block of text is underlined at rest in every theme, not only
    /// under the pointer, and a web address ends in the arrow that marks leaving Canopy.
    #[test]
    fn inline_links_are_underlined_at_rest_and_web_ones_end_in_an_arrow() {
        for id in theme::ThemeId::ALL {
            let ctx = crate::ui::kit::testkit::ctx_with(
                id,
                theme::Style::Standard,
                theme::Density::Comfortable,
            );
            let accent = palette(&ctx).accent2;
            let mut clicked = None;
            for _ in 0..2 {
                privacy_frame(&ctx, Vec::new(), &mut clicked);
            }
            let out = privacy_frame(&ctx, Vec::new(), &mut clicked);
            let (underlines, arrows) = out
                .shapes
                .iter()
                .map(|c| count(&c.shape, accent))
                .fold((0, 0), |(a, b), (c, d)| (a + c, b + d));
            assert!(
                underlines >= privacy_links(),
                "{id:?}: {underlines} underlines for {} links",
                privacy_links()
            );
            assert!(arrows >= 1, "{id:?}: a web link ends in an arrow");
        }
    }

    /// A link takes Tab focus, Enter follows it, and the secondary click offers to copy it.
    #[test]
    fn an_inline_link_follows_on_enter_and_offers_to_copy_it() {
        use crate::ui::kit::testkit;
        let ctx = testkit::ctx();
        let mut clicked = None;
        for _ in 0..2 {
            privacy_frame(&ctx, Vec::new(), &mut clicked);
        }
        // The first link is the first Tab stop.
        privacy_frame(&ctx, testkit::key(egui::Key::Tab), &mut clicked);
        privacy_frame(&ctx, Vec::new(), &mut clicked);
        assert!(clicked.is_none());
        privacy_frame(&ctx, testkit::key(egui::Key::Enter), &mut clicked);
        privacy_frame(&ctx, Vec::new(), &mut clicked);
        assert!(clicked.is_some(), "Enter follows the focused link");

        let ctx = testkit::ctx();
        ctx.enable_accesskit();
        let at = |out: &egui::FullOutput| {
            out.platform_output
                .accesskit_update
                .as_ref()
                .and_then(|u| {
                    u.nodes
                        .iter()
                        .find(|(_, n)| n.role() == egui::accesskit::Role::Link)
                })
                .and_then(|(_, n)| n.bounds())
                .map(|b| {
                    pos2(
                        crate::ui::to_f32(f64::midpoint(b.x0, b.x1)),
                        crate::ui::to_f32(f64::midpoint(b.y0, b.y1)),
                    )
                })
                .expect("a link")
        };
        privacy_frame(&ctx, Vec::new(), &mut clicked);
        let out = privacy_frame(&ctx, Vec::new(), &mut clicked);
        let link = at(&out);
        privacy_frame(&ctx, testkit::secondary_click(link), &mut clicked);
        let out = privacy_frame(&ctx, Vec::new(), &mut clicked);
        let labels: Vec<String> = out
            .platform_output
            .accesskit_update
            .map(|u| {
                u.nodes
                    .iter()
                    .filter_map(|(_, n)| n.label().map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default();
        assert!(
            labels
                .iter()
                .any(|l| l == "Copy link" || l == "Copy address"),
            "{labels:?}"
        );
    }
    /// A link spanning wrapped rows exposes a hit area for every row, not a single
    /// rectangle covering the intervening whitespace or a misplaced preceding row.
    #[test]
    fn a_multiline_link_has_one_hit_area_per_row() {
        let ctx = egui::Context::default();
        crate::ui::fonts::install(&ctx);
        let prefix = "See ";
        let label = "alpha beta gamma delta epsilon zeta eta theta";
        let text = format!("{prefix}{label}");
        let mut galley = None;
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            let job = LayoutJob::simple(
                text.clone(),
                FontId::proportional(16.0),
                Color32::WHITE,
                115.0,
            );
            galley = Some(ui.fonts_mut(|f| f.layout_job(job)));
        });
        output.textures_delta.clear();
        let galley = galley.expect("laid out");
        let start = prefix.chars().count();
        let range = start..start + label.chars().count();
        let areas = link_areas(&galley, &range, egui::Vec2::ZERO);
        assert!(areas.len() > 1, "expected wrapping: {areas:?}");
        assert_eq!(areas.len(), galley.rows.len(), "the link covers every row");
        for (area, row) in areas.iter().zip(&galley.rows) {
            let bounds = row.rect();
            assert_eq!(area.y_range(), bounds.y_range());
            assert!(area.left() >= bounds.left());
            assert!(area.right() <= bounds.right() + 0.5);
        }
    }

    /// A link that wraps gets an area on each line it runs over, and one that starts a line is
    /// not drawn at the end of the line before it.
    #[test]
    fn a_link_that_wraps_is_found_on_each_of_its_lines() {
        let ctx = egui::Context::default();
        crate::ui::fonts::install(&ctx);
        let text = "For questions about your data, email support@necturalabs.com today";
        let font = FontId::proportional(16.0);
        let galley_of = |width: f32| {
            let mut out = None;
            let mut run = ctx.run_ui(egui::RawInput::default(), |ui| {
                let mut job =
                    LayoutJob::simple(text.to_owned(), font.clone(), Color32::WHITE, width);
                job.wrap.max_width = width;
                out = Some(ui.fonts_mut(|f| f.layout_job(job)));
            });
            run.textures_delta.clear();
            out.expect("laid out")
        };
        let before = text.split_once("support").expect("link").0.chars().count();
        let range = before..before + "support@necturalabs.com".chars().count();
        let whole = galley_of(10_000.0);
        let one = link_areas(&whole, &range, egui::Vec2::ZERO);
        assert_eq!(one.len(), 1);
        // Narrow enough that the address starts a line of its own.
        let narrow = galley_of(whole.size().x * 0.62);
        assert!(narrow.rows.len() >= 2, "{}", narrow.rows.len());
        let areas = link_areas(&narrow, &range, egui::Vec2::ZERO);
        assert_eq!(areas.len(), 1, "{areas:?}");
        let last = narrow.rows.last().expect("a row").rect();
        assert!(
            areas[0].top() >= last.top() - 0.5,
            "the link is on the line it was wrapped to: {:?} not in {last:?}",
            areas[0]
        );
        assert!(
            (areas[0].width() - one[0].width()).abs() < 1.0,
            "all of it: {:?} vs {:?}",
            areas[0],
            one[0]
        );
    }
}
