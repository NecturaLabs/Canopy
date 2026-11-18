#!/usr/bin/env python3
"""Signs Canopy's update manifests in minisign's format, with openssl and no other tool.

    sign-manifest.py sign PUBLIC_KEY_FILE MANIFEST...
    sign-manifest.py selftest
    sign-manifest.py emit DIR    (a throwaway key and signed manifest, for Canopy's own tests)

`sign` reads the Ed25519 secret key as PEM from the environment variable CANOPY_MANIFEST_KEY,
checks that it belongs to PUBLIC_KEY_FILE (a minisign public key, `packaging/update-signing.pub`),
and writes `MANIFEST.minisig` beside each manifest. Canopy refuses a manifest whose signature does
not verify against a key built into it (`canopy_hosting::updates`), so whoever can write the update
feed cannot ship an archive without this key.

The signature is minisign's prehashed form: an Ed25519 signature of the manifest's BLAKE2b-512
digest, then a global signature over that signature and the trusted comment. The trusted comment
names the channel (the manifest's file name: `latest.json` or `beta.json`) and the build, and
Canopy checks both, so a signed manifest cannot be served as another channel's. Each signature is
verified again before `sign` returns. The release workflow runs `sign` in the `release`
environment, which alone holds the key. Needs OpenSSL 3 (`pkeyutl -rawin`).
"""

import base64
import hashlib
import json
import os
import re
import subprocess
import sys
import tempfile
from pathlib import Path

ALGORITHM = b"ED"
PUBLIC_ALGORITHM = b"Ed"
KEY_ID_LEN = 8
PUBLIC_KEY_LEN = 32
DER_PREFIX_LEN = 12
TRUSTED_COMMENT = "Canopy update manifest: channel={channel} build={build}"
CHANNELS = ("latest", "beta")


def public_key(path):
    """The key id and the raw public key from a minisign public key file."""
    lines = [line for line in Path(path).read_text().splitlines() if line.strip()]
    raw = base64.b64decode(lines[-1])
    if len(raw) != len(PUBLIC_ALGORITHM) + KEY_ID_LEN + PUBLIC_KEY_LEN or not raw.startswith(PUBLIC_ALGORITHM):
        raise SystemExit(f"{path}: not a minisign Ed25519 public key")
    start = len(PUBLIC_ALGORITHM)
    return raw[start : start + KEY_ID_LEN], raw[start + KEY_ID_LEN :]


def openssl(*args, data=None):
    return subprocess.run(["openssl", *args], input=data, capture_output=True, check=True).stdout


def ed25519_sign(key_file, message, work):
    message_file = Path(work) / "message"
    message_file.write_bytes(message)
    return openssl("pkeyutl", "-sign", "-rawin", "-inkey", str(key_file), "-in", str(message_file))


def trusted_comment(path, manifest):
    """What the signature vouches for besides the bytes: the channel and the build."""
    channel = Path(path).name.removesuffix(".json")
    if channel not in CHANNELS:
        raise SystemExit(f"{path}: a manifest is named latest.json or beta.json")
    build = str(json.loads(manifest).get("version", ""))
    if not re.fullmatch(r"[0-9]+", build):
        raise SystemExit(f"{path}: the manifest names no build number")
    return TRUSTED_COMMENT.format(channel=channel, build=build)


def signature_text(key_file, key_id, manifest, comment, work):
    signature = ed25519_sign(key_file, hashlib.blake2b(manifest, digest_size=64).digest(), work)
    global_signature = ed25519_sign(key_file, signature + comment.encode(), work)
    line = base64.b64encode(ALGORITHM + key_id + signature).decode()
    return (
        f"untrusted comment: signature from the Canopy release workflow\n{line}\n"
        f"trusted comment: {comment}\n{base64.b64encode(global_signature).decode()}\n"
    )


def sign(public_key_file, manifests, pem):
    key_id, expected = public_key(public_key_file)
    with tempfile.TemporaryDirectory() as work:
        key_file = Path(work) / "key.pem"
        key_file.touch(mode=0o600)
        key_file.write_text(pem)
        actual = openssl("pkey", "-in", str(key_file), "-pubout", "-outform", "DER")[DER_PREFIX_LEN:]
        if actual != expected:
            raise SystemExit("CANOPY_MANIFEST_KEY does not belong to " + str(public_key_file))
        for manifest in manifests:
            path = Path(manifest)
            signature = path.with_name(path.name + ".minisig")
            # The feed repository is not trusted: a link there must not redirect a write.
            if path.is_symlink() or signature.is_symlink():
                raise SystemExit(f"{path}: a manifest or its signature is a symbolic link")
            data = path.read_bytes()
            signature.write_text(signature_text(key_file, key_id, data, trusted_comment(path, data), work))
            if not verify(public_key_file, path, signature, work):
                signature.unlink()
                raise SystemExit(f"{path}: the new signature does not verify")
            print(f"signed {path}")


def verify(public_key_file, manifest, signature_file, work):
    """Checks a signature with openssl, the way minisign does (for the self-test)."""
    key_id, raw = public_key(public_key_file)
    pub = Path(work) / "pub.der"
    pub.write_bytes(bytes.fromhex("302a300506032b6570032100") + raw)
    lines = Path(signature_file).read_text().splitlines()
    blob = base64.b64decode(lines[1])
    if blob[:2] != ALGORITHM or blob[2:10] != key_id:
        return False
    trusted = lines[2].removeprefix("trusted comment: ")
    checks = (
        (hashlib.blake2b(Path(manifest).read_bytes(), digest_size=64).digest(), blob[10:]),
        (blob[10:] + trusted.encode(), base64.b64decode(lines[3])),
    )
    for message, signature in checks:
        (Path(work) / "m").write_bytes(message)
        (Path(work) / "s").write_bytes(signature)
        ok = subprocess.run(
            ["openssl", "pkeyutl", "-verify", "-rawin", "-pubin", "-keyform", "DER", "-inkey", str(pub),
             "-in", str(Path(work) / "m"), "-sigfile", str(Path(work) / "s")],
            capture_output=True,
            check=False,
        )
        if ok.returncode != 0:
            return False
    return True


def selftest():
    with tempfile.TemporaryDirectory() as work:
        work = Path(work)
        pem = openssl("genpkey", "-algorithm", "ed25519").decode()
        (work / "k.pem").write_text(pem)
        raw = openssl("pkey", "-in", str(work / "k.pem"), "-pubout", "-outform", "DER")[DER_PREFIX_LEN:]
        key_id = bytes(range(1, KEY_ID_LEN + 1))
        pub = work / "test.pub"
        pub.write_text("untrusted comment: test\n" + base64.b64encode(PUBLIC_ALGORITHM + key_id + raw).decode() + "\n")
        manifest = work / "latest.json"
        manifest.write_text('{"version": "1"}\n')
        sign(pub, [manifest], pem)
        signature = work / "latest.json.minisig"
        assert verify(pub, manifest, signature, work), "a fresh signature does not verify"
        assert "trusted comment: Canopy update manifest: channel=latest build=1\n" in signature.read_text()
        manifest.write_text('{"version": "2"}\n')
        assert not verify(pub, manifest, signature, work), "a changed manifest still verifies"
        link = work / "beta.json"
        link.symlink_to(manifest)
        try:
            sign(pub, [link], pem)
        except SystemExit:
            pass
        else:
            raise AssertionError("a symbolic link was signed")
        link.unlink()
        for name, body in (("other.json", '{"version": "1"}'), ("beta.json", '{"version": "0.2.1"}'), ("beta.json", '{"version": "١٢"}')):
            (work / name).write_text(body)
            try:
                sign(pub, [work / name], pem)
            except SystemExit:
                pass
            else:
                raise AssertionError(f"{name} {body} was signed")
        other = openssl("genpkey", "-algorithm", "ed25519").decode()
        try:
            sign(pub, [manifest], other)
        except SystemExit:
            pass
        else:
            raise AssertionError("a key that does not match the public key signed")
    print("sign-manifest selftest: ok")


def emit(directory):
    """Writes a throwaway public key, a signed latest.json and its signature into DIRECTORY, for
    the Rust test that checks Canopy accepts what this script signs."""
    out = Path(directory)
    with tempfile.TemporaryDirectory() as work:
        pem = openssl("genpkey", "-algorithm", "ed25519").decode()
        (Path(work) / "k.pem").write_text(pem)
        raw = openssl("pkey", "-in", str(Path(work) / "k.pem"), "-pubout", "-outform", "DER")[DER_PREFIX_LEN:]
    key_id = bytes(range(8, 0, -1))
    pub = out / "test.pub"
    pub.write_text("untrusted comment: test\n" + base64.b64encode(PUBLIC_ALGORITHM + key_id + raw).decode() + "\n")
    (out / "latest.json").write_text('{"version": "1700", "platforms": {}}\n')
    sign(pub, [out / "latest.json"], pem)


def main():
    args = sys.argv[1:]
    if args == ["selftest"]:
        selftest()
    elif len(args) == 2 and args[0] == "emit":
        emit(args[1])
    elif len(args) >= 3 and args[0] == "sign":
        pem = os.environ.get("CANOPY_MANIFEST_KEY", "")
        if "PRIVATE KEY" not in pem:
            raise SystemExit("CANOPY_MANIFEST_KEY holds no PEM private key")
        sign(args[1], args[2:], pem)
    else:
        raise SystemExit(__doc__)


if __name__ == "__main__":
    main()
