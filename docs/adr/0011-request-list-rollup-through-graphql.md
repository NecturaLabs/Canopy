# ADR 0011: Request list checks and reviews through one GraphQL query

**Status:** accepted
**Date:** 2026-10-06

## Context

Reviewers scan the pull or merge request list for each request's CI and review state. The REST
list endpoints carry neither: GitLab's list has no head pipeline, and GitHub's list has no checks
or reviews at all. So rows showed them only for requests opened in the session. Asking each
request's REST endpoints would cost up to 100 calls (GitHub) or 200 (GitLab) per refresh. ADR 0003
allows only the REST API of the repository's own host.

## Decision

On each list refresh, when the hosting thread has a token for the host (saved in Canopy, the
host's environment variable or git's credential helper: the same token the list already sends to
the same host), send one GraphQL POST to the same host's GraphQL endpoint:
`https://api.github.com/graphql`, or `https://<host>[:port]/api/graphql` for GitLab. The endpoint is
derived only from the API base, never from response data. The request carries the same token,
`User-Agent`, timeouts (10 s to connect, 30 s in total), 10 MiB body limit and log fields as the
REST calls, and follows no redirect.

- GitHub: per listed number, `statusCheckRollup.state`, `reviewDecision`, and
  `latestOpinionatedReviews` when there is no decision.
- GitLab: `headPipeline.status` and `approvedBy` for the listed iids, only fields every edition
  has.

At most the 100 listed requests are asked for. Results ride on the list (`RequestSummary::rollup`)
and are refreshed only with it. The query waits a quarter of a second after the list is shown, and
for every job queued by then, so opening the selected request or an action the user starts is not
held up behind it. A rate-limit answer pauses the query until its reset, 60 s when none is given,
an hour at most. Any other failure leaves the rows as before. Without a token nothing extra is
sent, and `Client::rollup` itself refuses to send without one. Turning off Pull and merge
requests, changing the token or switching remotes stops the query, even one already queued.

This amends ADR 0003's network table: its hosting API row now covers REST and this one GraphQL
query on the same host.

## Alternatives and costs

- Per-request REST calls: too many requests against the rate limit, and slow.
- Opening each request to see its state (the behaviour before): no way to scan the list.
- GitLab's `approvalsLeft` or `approved`: `approvalsLeft` is Premium-only, and an unknown field
  fails the whole query on a Community Edition server; `approved` means "has any approval" there.
  So GitLab rows show approvals but never "review required".

One extra request per refresh counts against the provider's GraphQL limit. GitHub keeps a
separate budget of 5,000 points an hour for GraphQL, and the query costs a few points.

## Searching on the host (amendment)

When the host holds more requests than the first page, the request list's text, draft choice,
author and newest/oldest sort are put to the host (`Client::search_page`), so a search reaches
requests that are not loaded. GitHub's REST search does not return the branches a row needs, so
GitHub uses the same GraphQL endpoint and token as the rollup: `search(type: ISSUE)` on
`repo:<owner>/<name> is:pr`, paged by cursor, each typed word quoted without quotes, colons or
backslashes so it cannot become a qualifier, at most eight words, the author only when it can be a
login. Without a token GitHub is not searched (GraphQL refuses anonymous calls); the loaded rows
are filtered locally as before. GitLab's list endpoint takes `search`, `in`, `author_username`,
`draft` (`wip` before GitLab 19.0), `order_by` and `sort` directly.

Cost: one request per search page, none per keystroke (the list waits 400 ms after the last key),
and a result for an older text is dropped by comparing the list's key. GitHub's search has its own
limit of 30 requests a minute and 1,000 results per query; a limit answer pauses the list the way
the timed refresh already pauses. The typed text now reaches the repository's own host, which the
privacy policy says.

## References

- [GitHub GraphQL `PullRequest`](https://docs.github.com/en/graphql/reference/objects#pullrequest)
  (`statusCheckRollup`, `reviewDecision`, `latestOpinionatedReviews`)
- [GitHub GraphQL rate limits](https://docs.github.com/en/graphql/overview/rate-limits-and-query-limits-for-the-graphql-api)
- [GitLab GraphQL `MergeRequest`](https://docs.gitlab.com/api/graphql/reference/#mergerequest)
- [ADR 0003](0003-network-services-and-consent.md), amended here for the hosting API row.
- [ADR 0004](0004-hosting-api-tokens.md), where tokens come from and which hosts receive them.
