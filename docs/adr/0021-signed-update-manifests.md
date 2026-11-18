# ADR 0021: Update manifests are signed; Canopy trusts only signed ones

**Status:** accepted
**Date:** 2026-11-18

## Context

Canopy's updater and the one-line installers read `canopy/latest.json` (or `beta.json`) in the
public `NecturaLabs/app-updates` repository and check each archive against the SHA-256 in it.
The archive and the manifest come from the same place, so whoever can write that repository (a
leaked feed token, a compromised account) could ship any build to every installed copy (#227).
The owner asked for this to be closed (2026-11-18).

## Decision

- The release workflow signs every manifest it writes with an Ed25519 key in minisign's
  prehashed format (`tools/sign-manifest.py`, openssl only) and commits `<manifest>.minisig`
  beside it. The key is the secret `MANIFEST_SIGNING_KEY` of the `release` environment, which
  only `main` can use; it is not in `app-updates` or in the feed token's reach. A copy is kept
  offline by the owner.
- Canopy fetches the signature with the manifest from the same host and parses the manifest only
  when the signature verifies against a key built into it (`canopy_hosting::updates::MANIFEST_KEYS`,
  matching `packaging/update-signing.pub`), with the `minisign-verify` crate (MIT, no
  dependencies). A missing, malformed or wrong signature is an update-check error; nothing is
  offered or downloaded.
- The installers do not check the signature: they are themselves downloaded from `app-updates`,
  so an attacker who can change the manifest can change the installer, and a check inside it
  would prove nothing. Release files carry build provenance and immutable GitHub releases for
  people who verify by hand (SECURITY.md).

## Alternatives

- Verifying the Sigstore build-provenance attestation in the app: needs a Sigstore client and
  trust-root updates in Canopy and requests to further hosts (a change to the product's network
  rules); kept for manual verification instead.
- Trusting Canopy's own immutable GitHub release's `SHA256SUMS`: another request to another
  release, and the release workflow's token can still create a new release.

## Consequences

- Losing the key leaves running copies unable to update until they are reinstalled; the owner
  keeps an offline copy. Rotating it: add the new public key to `MANIFEST_KEYS`, release, then
  sign with the new key.
- A replayed older signed manifest only offers an older build, which Canopy never installs.
- Copies from before this change still trust the unsigned checksum alone until they update.
