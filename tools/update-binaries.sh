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

    # Regenerate the manifest, including newly published stub variants.
    expected = {
        f"{kind}-{arch}-{os_name}{suffix}"
        for kind in ["runfiles-stub", "runfiles-stub-large", "finalize-stub"]
        for arch, os_name, suffix in [
            ("aarch64", "linux", ""), ("s390x", "linux", ""),
            ("x86_64", "linux", ""), ("aarch64", "macos", ""),
            ("x86_64", "macos", ""), ("aarch64", "windows", ".exe"),
            ("x86_64", "windows", ".exe"),
        ]
    }
    missing = expected - sums.keys()
    if missing:
        sys.exit("error: release is missing binaries: " + ", ".join(sorted(missing)))
    manifest = {}
    for filename in sorted(expected):
        if not re.fullmatch(r"[a-f0-9]{64}", sums[filename]):
            sys.exit(f"error: invalid SHA256 for {filename}")
        manifest[filename] = {
            "name": filename.removesuffix(".exe").replace("-", "_"),
            "url": f"https://github.com/{REPO}/releases/download/{tag}/{filename}",
            "sha256": sums[filename],
        }
    content = re.sub(r"_download_attrs = \{.*?\n\}",
                     "_download_attrs = " + json.dumps(manifest, indent=4), content, flags=re.S)
    module_path = os.path.join(workspace, "MODULE.bazel")
    with open(module_path) as f:
        module = f.read()
    repos = ", ".join(json.dumps(attrs["name"]) for attrs in manifest.values())
    module = re.sub(r"use_repo\(non_module_dependencies,.*?\)",
                    "use_repo(non_module_dependencies, " + repos + ")", module, flags=re.S)
    with open(module_path, "w") as f:
        f.write(module)

    with open(extensions_path, "w") as f:
        f.write(content)

    print(f"Done — updated {EXTENSIONS_BZL} to {tag}")


main()
PYEOF
