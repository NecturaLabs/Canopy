//! What the request list shows: the text, draft, checks, review and author filters and the sort
//! order, chosen per repository and applied to the rows already loaded. Nothing here asks the
//! host for anything, so a change of filter or sort answers at once.

use std::cmp::{Ordering, Reverse};

use canopy_hosting::{CheckState, RequestSummary, ReviewDecision, Rollup};
use serde::{Deserialize, Serialize};

/// Which drafts the list keeps.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DraftChoice {
    /// Drafts only.
    Drafts,
    /// Everything but drafts.
    Ready,
}

impl DraftChoice {
    pub fn label(self) -> &'static str {
        match self {
            Self::Drafts => "Drafts only",
            Self::Ready => "Ready for review",
        }
    }
}

/// Where a request's checks stand.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ChecksChoice {
    Passed,
    Failed,
    Running,
}

impl ChecksChoice {
    pub fn label(self) -> &'static str {
        match self {
            Self::Passed => "Passed",
            Self::Failed => "Failed",
            Self::Running => "Running",
        }
    }
}

/// Where a request's review stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReviewChoice {
    Approved,
    ChangesRequested,
    Required,
}

impl ReviewChoice {
    pub fn label(self) -> &'static str {
        match self {
            Self::Approved => "Approved",
            Self::ChangesRequested => "Changes requested",
            Self::Required => "Required",
        }
    }
}

/// The order of the list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum RequestSort {
    /// The most recently updated first, as the host lists them.
    #[default]
    RecentlyUpdated,
    /// The most recently opened first.
    Newest,
    /// The longest open first.
    Oldest,
    /// The lowest number first.
    Number,
    /// A to Z.
    Title,
    /// A to Z by author, newest first within one.
    Author,
}

impl RequestSort {
    pub const ALL: [Self; 6] = [
        Self::RecentlyUpdated,
        Self::Newest,
        Self::Oldest,
        Self::Number,
        Self::Title,
        Self::Author,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::RecentlyUpdated => "Recently updated",
            Self::Newest => "Newest",
            Self::Oldest => "Oldest",
            Self::Number => "Number",
            Self::Title => "Title",
            Self::Author => "Author",
        }
    }

    /// How `a` and `b` order under this sort; ties fall back to the most recently updated, then
    /// the highest number, so the order is the same on every frame.
    fn compare(self, a: &RequestSummary, b: &RequestSummary) -> Ordering {
        let primary = match self {
            Self::RecentlyUpdated => Reverse(a.updated_at).cmp(&Reverse(b.updated_at)),
            Self::Newest => Reverse(a.created_at).cmp(&Reverse(b.created_at)),
            Self::Oldest => a.created_at.cmp(&b.created_at),
            Self::Number => a.number.cmp(&b.number),
            Self::Title => a.title.to_lowercase().cmp(&b.title.to_lowercase()),
            Self::Author => a.author.to_lowercase().cmp(&b.author.to_lowercase()),
        };
        primary
            .then(Reverse(a.updated_at).cmp(&Reverse(b.updated_at)))
            .then(Reverse(a.number).cmp(&Reverse(b.number)))
    }
}

/// Where a request's checks and review stand, as far as is known (`None`: not known, or nothing
/// to say).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Standing {
    pub checks: Option<ChecksChoice>,
    pub review: Option<ReviewChoice>,
}

impl Standing {
    /// What a list's rollup (the provider's combined checks state and review decision) says.
    pub fn of_rollup(rollup: Rollup) -> Self {
        Self {
            checks: rollup.checks.and_then(|state| match state {
                CheckState::Success => Some(ChecksChoice::Passed),
                CheckState::Failure => Some(ChecksChoice::Failed),
                CheckState::Pending => Some(ChecksChoice::Running),
                CheckState::Neutral => None,
            }),
            review: rollup.review.map(|decision| match decision {
                ReviewDecision::Approved => ReviewChoice::Approved,
                ReviewDecision::ChangesRequested => ReviewChoice::ChangesRequested,
                ReviewDecision::Required => ReviewChoice::Required,
            }),
        }
    }
}

/// The filters and the sort of one repository's request list.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct RequestView {
    /// Words that must all appear in the number, title, author or description.
    pub text: String,
    pub draft: Option<DraftChoice>,
    pub checks: Option<ChecksChoice>,
    pub review: Option<ReviewChoice>,
    /// An author's login.
    pub author: Option<String>,
    pub sort: RequestSort,
}

impl RequestView {
    /// Whether the view is as a repository starts with it (nothing to remember).
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }

    /// How many filters narrow the list.
    pub fn filters(&self) -> usize {
        usize::from(!self.text.trim().is_empty())
            + usize::from(self.draft.is_some())
            + usize::from(self.checks.is_some())
            + usize::from(self.review.is_some())
            + usize::from(self.author.is_some())
    }

    /// How many of the filters other than the text narrow the list (the ones the filter menu
    /// holds).
    pub fn menu_filters(&self) -> usize {
        self.filters() - usize::from(!self.text.trim().is_empty())
    }

    /// Drops every filter and keeps the sort.
    pub fn clear_filters(&mut self) {
        *self = Self {
            sort: self.sort,
            ..Self::default()
        };
    }

    /// Whether request `r`, standing as `standing` says, passes every filter.
    pub fn matches(&self, r: &RequestSummary, standing: Standing) -> bool {
        self.draft
            .is_none_or(|d| (d == DraftChoice::Drafts) == r.draft)
            && self.checks.is_none_or(|c| standing.checks == Some(c))
            && self.review.is_none_or(|v| standing.review == Some(v))
            && self
                .author
                .as_deref()
                .is_none_or(|a| a.eq_ignore_ascii_case(&r.author))
            && text_matches(&self.text, r)
    }

    /// The indices into `rows` of the requests to show, in the order to show them.
    pub fn visible(
        &self,
        rows: &[RequestSummary],
        standing: impl Fn(&RequestSummary) -> Standing,
    ) -> Vec<usize> {
        let mut shown: Vec<usize> = (0..rows.len())
            .filter(|&i| self.matches(&rows[i], standing(&rows[i])))
            .collect();
        shown.sort_by(|&a, &b| self.sort.compare(&rows[a], &rows[b]));
        shown
    }
}

/// The number of the request a list opens on: the first one shown, going by what each row's own
/// rollup says (a list that has just arrived has nothing else).
pub fn first_shown(view: &RequestView, rows: &[RequestSummary]) -> Option<u64> {
    let standing = |r: &RequestSummary| r.rollup.map(Standing::of_rollup).unwrap_or_default();
    view.visible(rows, standing)
        .first()
        .map(|&i| rows[i].number)
}

/// Whether every word of `query` (any case) is in `r`'s number (`154`, `#154` or `!154`), title,
/// author or description.
fn text_matches(query: &str, r: &RequestSummary) -> bool {
    let query = query.to_lowercase();
    let mut words = query.split_whitespace().peekable();
    if words.peek().is_none() {
        return true;
    }
    let number = r.number.to_string();
    let (title, author, description) = (
        r.title.to_lowercase(),
        r.author.to_lowercase(),
        r.description.to_lowercase(),
    );
    words.all(|word| {
        let digits = word.trim_start_matches(['#', '!']);
        (!digits.is_empty()
            && digits.bytes().all(|b| b.is_ascii_digit())
            && number.contains(digits))
            || title.contains(word)
            || author.contains(word)
            || description.contains(word)
    })
}

/// The distinct authors of `rows`, A to Z.
pub fn authors(rows: &[RequestSummary]) -> Vec<String> {
    let mut all: Vec<String> = rows.iter().map(|r| r.author.clone()).collect();
    all.sort_by_key(|a| a.to_lowercase());
    all.dedup_by(|a, b| a.eq_ignore_ascii_case(b));
    all
}

#[cfg(test)]
mod tests {
    use canopy_hosting::RequestState;

    use super::*;

    fn request(
        number: u64,
        title: &str,
        author: &str,
        created: i64,
        updated: i64,
    ) -> RequestSummary {
        RequestSummary {
            number,
            title: title.into(),
            author: author.into(),
            state: RequestState::Open,
            draft: false,
            source_branch: format!("topic-{number}"),
            target_branch: "main".into(),
            from_fork: false,
            created_at: created,
            updated_at: updated,
            web_url: String::new(),
            labels: Vec::new(),
            description: String::new(),
            rollup: None,
        }
    }

    /// Six requests, the way the host lists them (most recently updated first).
    fn rows() -> Vec<RequestSummary> {
        let mut rows = vec![
            request(31, "Stream large diffs", "lucia", 300, 900),
            request(12, "add keyboard help", "Maya", 100, 800),
            request(27, "Fix the graph lanes", "sam", 200, 700),
            request(40, "Bump egui", "lucia", 500, 600),
            request(8, "Draft: new palette", "maya", 50, 500),
            request(19, "Tidy settings", "sam", 400, 400),
        ];
        rows[0].description = "Large diffs no longer allocate a row per line.".into();
        rows[2].description = "Closes #12 and the lane flicker".into();
        rows[4].draft = true;
        rows
    }

    fn numbers(view: &RequestView, rows: &[RequestSummary]) -> Vec<u64> {
        view.visible(rows, |_| Standing::default())
            .into_iter()
            .map(|i| rows[i].number)
            .collect()
    }

    fn standing(r: &RequestSummary) -> Standing {
        match r.number {
            31 => Standing {
                checks: Some(ChecksChoice::Passed),
                review: Some(ReviewChoice::Approved),
            },
            12 => Standing {
                checks: Some(ChecksChoice::Failed),
                review: Some(ReviewChoice::ChangesRequested),
            },
            27 => Standing {
                checks: Some(ChecksChoice::Running),
                review: Some(ReviewChoice::Required),
            },
            _ => Standing::default(),
        }
    }

    #[test]
    fn the_default_view_keeps_every_request_in_the_hosts_order() {
        let rows = rows();
        assert!(RequestView::default().is_default());
        assert_eq!(
            numbers(&RequestView::default(), &rows),
            [31, 12, 27, 40, 8, 19]
        );
    }

    #[test]
    fn every_sort_orders_the_same_rows_its_own_way() {
        let rows = rows();
        let sorted = |sort| {
            numbers(
                &RequestView {
                    sort,
                    ..RequestView::default()
                },
                &rows,
            )
        };
        assert_eq!(
            sorted(RequestSort::RecentlyUpdated),
            [31, 12, 27, 40, 8, 19]
        );
        assert_eq!(sorted(RequestSort::Newest), [40, 19, 31, 27, 12, 8]);
        assert_eq!(sorted(RequestSort::Oldest), [8, 12, 27, 31, 19, 40]);
        assert_eq!(sorted(RequestSort::Number), [8, 12, 19, 27, 31, 40]);
        // Case does not matter ("add" before "Bump"); an author's requests go newest first.
        assert_eq!(sorted(RequestSort::Title), [12, 40, 8, 27, 31, 19]);
        assert_eq!(sorted(RequestSort::Author), [31, 40, 12, 8, 27, 19]);
    }

    #[test]
    fn a_tie_has_one_answer_whatever_order_the_rows_came_in() {
        let mut rows = rows();
        for r in &mut rows {
            r.updated_at = 1;
            r.created_at = 1;
        }
        let view = RequestView::default();
        let forward = numbers(&view, &rows);
        rows.reverse();
        assert_eq!(numbers(&view, &rows), forward);
        assert_eq!(forward, [40, 31, 27, 19, 12, 8], "highest number first");
    }

    #[test]
    fn text_matches_number_title_author_and_description() {
        let rows = rows();
        let find = |text: &str| {
            numbers(
                &RequestView {
                    text: text.into(),
                    ..RequestView::default()
                },
                &rows,
            )
        };
        assert_eq!(find("19"), [19], "a number");
        assert_eq!(find("#27"), [27], "a number with its sigil");
        assert_eq!(find("!8"), [8], "a merge request's sigil");
        assert_eq!(find("GRAPH"), [27], "case does not matter");
        assert_eq!(find("lucia"), [31, 40], "an author");
        assert_eq!(find("allocate row"), [31], "a description, every word");
        assert_eq!(find("lane flicker"), [27]);
        assert_eq!(
            find("lucia graph"),
            Vec::<u64>::new(),
            "every word must match"
        );
        assert_eq!(
            find("   "),
            [31, 12, 27, 40, 8, 19],
            "blank text filters nothing"
        );
        assert_eq!(find("!"), Vec::<u64>::new(), "a bare sigil is not a number");
    }

    #[test]
    fn the_other_filters_narrow_the_list_and_combine() {
        let rows = rows();
        let with = |view: RequestView| {
            view.visible(&rows, standing)
                .into_iter()
                .map(|i| rows[i].number)
                .collect::<Vec<_>>()
        };
        let none = RequestView::default;
        assert_eq!(
            with(RequestView {
                draft: Some(DraftChoice::Drafts),
                ..none()
            }),
            [8]
        );
        assert_eq!(
            with(RequestView {
                draft: Some(DraftChoice::Ready),
                ..none()
            }),
            [31, 12, 27, 40, 19]
        );
        assert_eq!(
            with(RequestView {
                checks: Some(ChecksChoice::Failed),
                ..none()
            }),
            [12]
        );
        assert_eq!(
            with(RequestView {
                review: Some(ReviewChoice::Required),
                ..none()
            }),
            [27]
        );
        assert_eq!(
            with(RequestView {
                author: Some("MAYA".into()),
                ..none()
            }),
            [12, 8],
            "an author's two spellings are one author"
        );
        assert_eq!(
            with(RequestView {
                author: Some("maya".into()),
                checks: Some(ChecksChoice::Failed),
                ..none()
            }),
            [12]
        );
        assert_eq!(
            with(RequestView {
                checks: Some(ChecksChoice::Passed),
                review: Some(ReviewChoice::Required),
                ..none()
            }),
            Vec::<u64>::new()
        );
    }

    #[test]
    fn filters_count_and_clear_but_the_sort_stays() {
        let mut view = RequestView {
            text: "graph".into(),
            draft: Some(DraftChoice::Ready),
            checks: Some(ChecksChoice::Passed),
            review: None,
            author: Some("sam".into()),
            sort: RequestSort::Title,
        };
        assert_eq!(view.filters(), 4);
        assert_eq!(view.menu_filters(), 3);
        assert!(!view.is_default());
        view.clear_filters();
        assert_eq!(view.filters(), 0);
        assert_eq!(view.sort, RequestSort::Title);
        assert!(!view.is_default(), "a sort is a choice to remember");
        view.sort = RequestSort::default();
        assert!(view.is_default());
        let spaces = RequestView {
            text: "  ".into(),
            ..RequestView::default()
        };
        assert_eq!(spaces.filters(), 0);
    }

    #[test]
    fn authors_are_listed_once_a_to_z() {
        assert_eq!(authors(&rows()), ["lucia", "Maya", "sam"]);
        assert_eq!(authors(&[]), Vec::<String>::new());
    }

    #[test]
    fn a_saved_view_reads_back_and_tolerates_what_it_lacks() {
        let view = RequestView {
            text: "graph".into(),
            checks: Some(ChecksChoice::Failed),
            author: Some("sam".into()),
            sort: RequestSort::Oldest,
            ..RequestView::default()
        };
        let json = serde_json::to_string(&view).expect("serialize");
        assert_eq!(
            serde_json::from_str::<RequestView>(&json).expect("read"),
            view
        );
        let partial: RequestView = serde_json::from_str(r#"{"sort":"Title"}"#).expect("partial");
        assert_eq!(partial.sort, RequestSort::Title);
        assert_eq!(partial.filters(), 0);
    }
}
