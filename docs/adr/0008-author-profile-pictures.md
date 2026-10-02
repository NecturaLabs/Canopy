# ADR 0008: Author profile pictures with a bounded local cache

**Status:** accepted
**Date:** 2026-10-03

## Context

The owner requested GitHub/GitLab profile pictures instead of initials, then approved direct Gravatar and Libravatar image links if they need no API keys. This replaces ADR 0003's exclusion of avatar downloads; its other network and consent rules remain.

## Decision

Use GitHub's commit API account mapping and GitLab's public-email Avatar API. Download only the returned direct image URL, restricted to the repository host, GitHub's avatar CDN, and exact Gravatar/Libravatar HTTPS image hosts returned by GitLab. Never use a profile API key, send credentials with image requests, follow image redirects, or discover arbitrary federated servers. Keep initials for unsupported or unavailable pictures.

The owner required a local twelve-hour cache, one lookup when due, and retention of the current picture on failure. Cache bounded 64×64 RGBA thumbnails and failed lookups on the hosting worker, scoped by host and email. A global cache lock prevents duplicate lookups across tabs. Files have opaque hashed names and no stored email or source URL. A failed refresh advances the due time while retaining the old thumbnail. Show cached pictures across restarts and retry only when the author is visible after expiry. Disabling downloads shows initials and leaves the disk cache available for re-enabling; deleting the cache folder removes it.

The hosting consent gate, a Privacy switch, Author avatars and the hosting API switch all apply. Bump the legal document versions so existing users see and accept the new network behavior. Limit downloads to 2 MiB and PNG/JPEG decoding to 2048×2048 with a 32 MiB allocation budget before reducing to thumbnails.

## Alternatives and costs

Keeping initials would not satisfy the request. Downloading arbitrary API-provided URLs or following redirects would widen the network boundary without review. Profile APIs add keys and unrelated data while image URLs suffice. Memory-only caching would re-fetch after every restart. Caching forever would miss profile updates. A disk cache retains publicly available pictures until users delete it and can temporarily show an old picture after failure. External image services receive the image URL and IP address; email hashes can identify people and are not anonymous.

## References

- [GitHub commit API](https://docs.github.com/en/rest/commits/commits)
- [GitLab Avatar API](https://docs.gitlab.com/api/avatar/)
- [Gravatar image URLs](https://docs.gravatar.com/sdk/images/)
- [Libravatar image API](https://wiki.libravatar.org/api/)
- [ADR 0003](0003-network-services-and-consent.md), superseded here only for author pictures.
