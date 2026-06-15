#!/usr/bin/env python3
"""Generate a binary size comparison report in Markdown format.

Usage: binary-size-report.py <sizes-base.txt> <sizes-pr.txt> [base-sha] [pr-sha]

Each size file contains lines of the form: <filename> <bytes>
"""
import sys


def read_sizes(path):
    sizes = {}
    with open(path) as f:
        for line in f:
            parts = line.strip().split()
            if len(parts) == 2:
                sizes[parts[0]] = int(parts[1])
    return sizes


def fmt_size(n):
    if n < 1024:
        return f"{n:,} B"
    elif n < 1024 * 1024:
        return f"{n / 1024:.1f} KiB"
    else:
        return f"{n / 1024 / 1024:.2f} MiB"


def fmt_delta(delta, base):
    if delta == 0:
        return "—"
    sign = "+" if delta > 0 else ""
    pct = delta / base * 100 if base else 0
    return f"{sign}{fmt_size(delta)} ({sign}{pct:.2f}%)"


def main():
    if len(sys.argv) < 3:
        print(f"Usage: {sys.argv[0]} <sizes-base.txt> <sizes-pr.txt> [base-sha] [pr-sha]", file=sys.stderr)
        sys.exit(1)

    base_file, pr_file = sys.argv[1], sys.argv[2]
    base_sha = sys.argv[3][:8] if len(sys.argv) > 3 else "base"
    pr_sha = sys.argv[4][:8] if len(sys.argv) > 4 else "pr"

    base = read_sizes(base_file)
    pr = read_sizes(pr_file)
    names = sorted(set(base) | set(pr))

    lines = [
        "<!-- binary-size-report -->",
        "## Binary Size Report",
        "",
        f"Comparing `{base_sha}` (base) → `{pr_sha}` (PR)",
        "",
        "| Binary | Base | PR | Change |",
        "|--------|------|----|--------|",
    ]

    total_base = total_pr = 0
    for name in names:
        b = base.get(name, 0)
        p = pr.get(name, 0)
        total_base += b
        total_pr += p
        lines.append(f"| `{name}` | {fmt_size(b)} | {fmt_size(p)} | {fmt_delta(p - b, b)} |")

    total_delta = total_pr - total_base
    lines += [
        "",
        f"**Total: {fmt_size(total_base)} → {fmt_size(total_pr)} ({fmt_delta(total_delta, total_base)})**",
    ]

    print("\n".join(lines))


main()
