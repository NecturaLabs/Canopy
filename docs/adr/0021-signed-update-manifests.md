# ADR 0021: Update manifests are signed; Canopy trusts only signed ones

**Status:** accepted
**Date:** 2026-11-18

## Context

Canopy's updater and the one-line installers read `canopy/latest.json` (or `beta.json`) in the
public `NecturaLabs/app-updates` repository and check each archive against the SHA-256 in it.
The archive and the manifest come from the same place, so whoever can write that repository (a
leaked feed token, a compromised account) could ship any build to every installed copy (#227).
The owner asked for this to be closed and approved the `minisign-verify` dependency
(2026-11-18).

## Decision

- The release workflow signs every manifest it writes with an Ed25519 key in minisign's
  prehashed format (`tools/sign-manifest.py`, openssl only) and commits `<manifest>.minisig`
  beside it. The key is the secret `MANIFEST_SIGNING_KEY` of the `release` environment, which
  only `main` can use; it is not in `app-updates` or in the feed token's reach. A copy is kept
  offline by the owner.
- Canopy fetches the signature with the manifest from the same host and parses the manifest only
  when the signature verifies against a key built into it (`canopy_hosting::updates::MANIFEST_KEYS`,
  matching `packaging/update-signing.pub`), with the `minisign-verify` crate (MIT, no
  dependencies). A missing, malformed, legacy (not prehashed) or wrong signature is an
  update-check error; nothing is offered or downloaded. The signature's trusted comment names the
  channel and the build (`Canopy update manifest: channel=latest build=1684`) and Canopy checks
  both, so a stable manifest cannot be served as the beta one or the other way round.
- The manifest also lists the SHA-256 of `install.sh` and `install.ps1` (`installers`). Where
  Update runs the installer (macOS, Windows, any install not in its own folder; ADR 0006), Canopy
  downloads the script from the build's own release and runs it only when it matches that signed
  checksum; refreshing `uninstall.sh` in an install folder applies the same check. Canopy runs it
  with `--sha256` (`-Sha256`) and the signed archive checksum, so the script reads nothing from
  the feed to decide what it installs. Update on such an install therefore needs the build to
  list this platform in its signed manifest.
- A build made with `CANOPY_UPDATE_KEY` set trusts that key instead of the built-in ones, for an
  end-to-end update test against a local feed (`docs/playbook.md`). Release builds never set it
  (`tools/test-release-gate.py`).
- The one-line install commands do not check the signature: they download the installer from
  `app-updates` itself, so an attacker who can change the manifest can change the installer, and
  a check inside it would prove nothing. They rely on HTTPS and the repository's protection
  (rulesets on `main`, #226). Release files carry build provenance and immutable GitHub releases for
  people who verify by hand (SECURITY.md).

Related: ADR 0006 (Canopy updates itself in place), ADR 0003 (network services).

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
- A replayed older signed manifest of the same channel only offers an older build, which Canopy
  never installs; one of the other channel fails the trusted-comment check.
- If the key leaks: remove `MANIFEST_SIGNING_KEY`, generate a new key, add its public key to
  `MANIFEST_KEYS` beside the old one and release a build signed with the old key, then drop the
  old key from `MANIFEST_KEYS` in the next build and sign only with the new one. Copies that never
  update past the first build keep trusting the leaked key; there is no revocation list, because
  Canopy fetches nothing else to learn about one.
- Copies from before this change still trust the unsigned checksum alone until they update.
- The manifests published before this change carry the trusted comment `Canopy update manifest`
  without a channel or build, which new builds refuse. They were re-signed with the new comment
  on 2026-11-18 (their content first checked against the old signature; the shipped builds do not
  read signatures, so this changed nothing for them); the release workflow writes the new form.
