#!/usr/bin/env python3
"""Release identity and GHCR checks for the container-images workflow."""

import argparse
import base64
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import urllib.error
import urllib.parse
import urllib.request


IMAGE = "ghcr.io/givtaj/sleepapi"
REPOSITORY = "givtaj/sleepapi"
ACCEPT = ", ".join([
    "application/vnd.oci.image.index.v1+json",
    "application/vnd.docker.distribution.manifest.list.v2+json",
    "application/vnd.oci.image.manifest.v1+json",
    "application/vnd.docker.distribution.manifest.v2+json",
])
CI_JOBS = {"Linux / Rust stable", "Linux / Rust 1.94.0", "Docker / linux/amd64", "Docker / linux/arm64"}


def require(condition, message):
    if not condition:
        raise RuntimeError(message)


def github(path):
    request = urllib.request.Request(
        "https://api.github.com/repos/" + REPOSITORY + path,
        headers={"Authorization": "Bearer " + os.environ["GH_TOKEN"], "Accept": "application/vnd.github+json"},
    )
    with urllib.request.urlopen(request, timeout=30) as response:
        return json.load(response)


def version_from_tag(tag):
    require(re.fullmatch(r"v[0-9]+\.[0-9]+\.[0-9]+(?:-[0-9A-Za-z]+(?:[.-][0-9A-Za-z]+)*)?", tag),
            "Use a versioned release tag such as v0.1.1-alpha.4")
    return tag[1:]


def release_identity(tag):
    version = version_from_tag(tag)
    release = github("/releases/tags/" + urllib.parse.quote(tag, safe=""))
    require(not release["draft"] and release["published_at"] and release["tag_name"] == tag,
            "The tag must have a published GitHub release (prereleases are supported)")
    obj = github("/git/ref/tags/" + urllib.parse.quote(tag, safe=""))["object"]
    for _ in range(8):
        if obj["type"] == "commit":
            break
        require(obj["type"] == "tag", "Release tag does not resolve to a commit")
        obj = github("/git/tags/" + obj["sha"])["object"]
    require(obj["type"] == "commit" and re.fullmatch(r"[0-9a-f]{40}", obj["sha"]), "Invalid release commit")
    sha = obj["sha"]
    runs = github("/actions/workflows/ci.yml/runs?" + urllib.parse.urlencode({
        "head_sha": sha, "event": "push", "branch": "main", "status": "success", "per_page": 100,
    }))["workflow_runs"]
    for run in runs:
        if run["head_sha"] != sha or run["conclusion"] != "success":
            continue
        jobs = github(f"/actions/runs/{run['id']}/jobs?per_page=100")["jobs"]
        passed = {job["name"] for job in jobs if job["conclusion"] == "success"}
        if CI_JOBS <= passed:
            return version, sha
    raise RuntimeError("No successful main CI run with all four required jobs for the release commit")


def output(**values):
    with open(os.environ["GITHUB_OUTPUT"], "a", encoding="utf-8") as stream:
        for key, value in values.items():
            stream.write(f"{key}={value}\n")


def prepare():
    import tomllib

    if os.environ["GITHUB_EVENT_NAME"] == "pull_request":
        version = tomllib.loads(Path("Cargo.toml").read_text())["package"]["version"]
        sha = os.environ["GITHUB_SHA"]
        publish = "false"
    else:
        require(os.environ["GITHUB_REPOSITORY"] == REPOSITORY, "Publication is restricted to the owning repository")
        event = json.loads(Path(os.environ["GITHUB_EVENT_PATH"]).read_text())
        if os.environ["GITHUB_EVENT_NAME"] == "workflow_dispatch":
            require(os.environ["GITHUB_REF"] == "refs/heads/main", "Dispatch publication from main")
            tag = event["inputs"]["tag"]
        else:
            require(os.environ["GITHUB_EVENT_NAME"] == "release", "Unsupported publication event")
            tag = event["release"]["tag_name"]
        version, sha = release_identity(tag)
        publish = "true"
    output(version=version, source_sha=sha, publish=publish)
    print(json.dumps({"version": version, "source_sha": sha, "publish": publish}))


def registry_manifest(reference, authenticated=False):
    query = urllib.parse.urlencode({"service": "ghcr.io", "scope": "repository:givtaj/sleepapi:pull"})
    headers = {}
    if authenticated:
        credentials = (os.environ["GITHUB_ACTOR"] + ":" + os.environ["GH_TOKEN"]).encode()
        headers["Authorization"] = "Basic " + base64.b64encode(credentials).decode()
    with urllib.request.urlopen(urllib.request.Request("https://ghcr.io/token?" + query, headers=headers), timeout=30) as response:
        token = json.load(response)["token"]
    request = urllib.request.Request(
        "https://ghcr.io/v2/givtaj/sleepapi/manifests/" + urllib.parse.quote(reference, safe=":"),
        headers={"Authorization": "Bearer " + token, "Accept": ACCEPT},
    )
    try:
        with urllib.request.urlopen(request, timeout=30) as response:
            raw = response.read()
            digest = "sha256:" + hashlib.sha256(raw).hexdigest()
            require(response.headers["Docker-Content-Digest"] == digest, "Registry manifest digest mismatch")
            return digest, json.loads(raw)
    except urllib.error.HTTPError as error:
        # Authentication, network, and server failures must never mean "absent".
        if error.code == 404:
            body = json.loads(error.read())
            codes = {entry["code"] for entry in body.get("errors", [])}
            if codes and codes <= {"MANIFEST_UNKNOWN", "NAME_UNKNOWN"}:
                return None
        raise


def inspect_image(reference, version, sha, arch):
    details = json.loads(subprocess.check_output(["docker", "image", "inspect", reference], text=True))[0]
    require(details["Os"] == "linux" and details["Architecture"] == arch, "Image platform mismatch")
    labels = details["Config"]["Labels"]
    require(labels.get("org.opencontainers.image.version") == version, "Image version mismatch")
    require(labels.get("org.opencontainers.image.revision") == sha, "Image source revision mismatch")
    require(labels.get("org.opencontainers.image.source") == "https://github.com/" + REPOSITORY, "Image source URL mismatch")
    return details


def record(args):
    details = inspect_image(args.reference, args.version, args.sha, args.arch)
    digests = [item.removeprefix(IMAGE + "@") for item in details["RepoDigests"] if item.startswith(IMAGE + "@")]
    require(len(digests) == 1 and re.fullmatch(r"sha256:[0-9a-f]{64}", digests[0]), "Expected one pushed image digest")
    Path(args.arch + ".json").write_text(json.dumps({
        "arch": args.arch, "version": args.version, "source_sha": args.sha, "digest": digests[0],
    }) + "\n")


def check_index(manifest, expected):
    actual = {}
    for entry in manifest["manifests"]:
        platform = entry["platform"]
        key = (platform["os"], platform["architecture"])
        require(key not in actual, "Duplicate image platform")
        actual[key] = entry["digest"]
    require(actual == {("linux", arch): digest for arch, digest in expected.items()}, "Unexpected manifest platforms or digests")


def publish_index(version, expected):
    existing = registry_manifest(version, authenticated=True)
    if existing is not None:
        # Recover a push that completed before its receipt/output was saved.
        # Reuse only the exact tested images; never rewrite the existing tag.
        check_index(existing[1], expected)
    else:
        command = ["docker", "buildx", "imagetools", "create", "--tag", IMAGE + ":" + version,
                   *[IMAGE + "@" + expected[arch] for arch in ("amd64", "arm64")]]
        preview = json.loads(subprocess.check_output([*command, "--dry-run"], text=True))
        check_index(preview, expected)
        subprocess.run(command, check=True)
    published = registry_manifest(version)
    require(published is not None, "Published version is not anonymously readable")
    digest, manifest = published
    check_index(manifest, expected)
    return digest


def promote(args):
    expected = {}
    for arch in ("amd64", "arm64"):
        receipt = json.loads(Path("digests", arch + ".json").read_text())
        require(receipt["arch"] == arch and receipt["version"] == args.version and receipt["source_sha"] == args.sha,
                "Platform receipt does not match this release")
        digest = receipt["digest"]
        require(re.fullmatch(r"sha256:[0-9a-f]{64}", digest), "Invalid platform digest")
        # A first GHCR package may need a one-time public visibility change.
        # Fail before publishing the version if the staging images are private.
        public = registry_manifest(digest)
        require(public is not None and public[0] == digest, "Staging image must be anonymously readable")
        require("config" in public[1] and "manifests" not in public[1], "Each staging digest must be one image, not an index")
        expected[arch] = digest
    require(release_identity("v" + args.version) == (args.version, args.sha), "Release tag moved during publication")
    digest = publish_index(args.version, expected)
    output(digest=digest)
    receipt = {"image": IMAGE, "version": args.version, "source_sha": args.sha,
               "manifest_digest": digest, "platform_digests": expected}
    Path("container-release.json").write_text(json.dumps(receipt, indent=2) + "\n")
    with open(os.environ["GITHUB_STEP_SUMMARY"], "a", encoding="utf-8") as stream:
        stream.write("## Published container\n\n```json\n" + json.dumps(receipt, indent=2) + "\n```\n")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=["prepare", "record", "promote", "verify"])
    parser.add_argument("--version")
    parser.add_argument("--sha")
    parser.add_argument("--arch", choices=["amd64", "arm64"])
    parser.add_argument("--reference")
    parser.add_argument("--digest")
    args = parser.parse_args()
    if args.command == "prepare":
        prepare()
    elif args.command == "record":
        record(args)
    elif args.command == "promote":
        promote(args)
    else:
        manifest = registry_manifest(args.version)
        require(manifest is not None and manifest[0] == args.digest, "Published version digest changed")
        details = inspect_image(args.reference, args.version, args.sha, args.arch)
        require(IMAGE + "@" + args.digest in details["RepoDigests"], "Pulled image does not match published digest")
        print(json.dumps({"image": args.reference, "digest": args.digest, "architecture": args.arch, "source_sha": args.sha}))


if __name__ == "__main__":
    main()
