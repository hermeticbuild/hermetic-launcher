#!/usr/bin/env bash
# Update launcher/private/extensions.bzl with the latest prebuilt binaries from GitHub.
# Usage: bazel run //tools:update-binaries [-- --tag binaries-YYYYMMDD]
set -euo pipefail
python3 - "$@" <<'PYEOF'
"""Update launcher/private/extensions.bzl with the latest prebuilt binaries from GitHub."""
import json, os, re, sys, urllib.request

REPO = "hermeticbuild/hermetic-launcher"
TAG_PREFIX = "binaries-"
EXTENSIONS_BZL = "launcher/private/extensions.bzl"


def fetch(url, token=None):
    headers = {"User-Agent": "update-binaries/1.0"}
    if token:
        headers["Authorization"] = f"Bearer {token}"
    req = urllib.request.Request(url, headers=headers)
    with urllib.request.urlopen(req) as r:
        return r.read()


def main():
    import argparse
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--tag", help="Use this specific release tag instead of the latest")
    args = parser.parse_args()

    token = os.environ.get("GITHUB_TOKEN")
    workspace = os.environ.get("BUILD_WORKSPACE_DIRECTORY", os.getcwd())
    extensions_path = os.path.join(workspace, EXTENSIONS_BZL)

    if args.tag:
        tag = args.tag
        print(f"Using tag: {tag}", flush=True)
        release = json.loads(fetch(
            f"https://api.github.com/repos/{REPO}/releases/tags/{tag}", token))
    else:
        print("Fetching releases...", flush=True)
        releases = json.loads(fetch(
            f"https://api.github.com/repos/{REPO}/releases?per_page=30", token))
        candidates = sorted(
            [r for r in releases if r["tag_name"].startswith(TAG_PREFIX)],
            key=lambda r: r["tag_name"],
            reverse=True,
        )
        if not candidates:
            sys.exit("error: no binaries-* releases found")
        release = candidates[0]
        tag = release["tag_name"]
        print(f"Latest release: {tag}", flush=True)

    asset = next((a for a in release["assets"] if a["name"] == "SHA256SUMS.txt"), None)
    if not asset:
        sys.exit(f"error: SHA256SUMS.txt not found in release {tag}")

    print("Downloading SHA256SUMS.txt...", flush=True)
    sums = {}
    for line in fetch(asset["browser_download_url"], token).decode().splitlines():
        parts = line.split(None, 1)
        if len(parts) == 2:
            sums[parts[1].strip()] = parts[0]

    with open(extensions_path) as f:
        content = f.read()

    # Replace the release tag in all download URLs
    content = re.sub(
        r"(releases/download/)binaries-\d{8}/",
        rf"\1{tag}/",
        content,
    )

    # Replace each SHA256 hash, matching via the filename on the preceding url line
    for filename, sha256 in sums.items():
        content, n = re.subn(
            rf'("url":\s*"[^"]+/{re.escape(filename)}",\n\s*"sha256":\s*")[a-f0-9]{{64}}(")',
            rf"\g<1>{sha256}\g<2>",
            content,
        )
        if n == 0:
            print(f"  warning: {filename} not found in extensions.bzl", file=sys.stderr)

    with open(extensions_path, "w") as f:
        f.write(content)

    print(f"Done — updated {EXTENSIONS_BZL} to {tag}")


main()
PYEOF
