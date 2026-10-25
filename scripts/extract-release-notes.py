#!/usr/bin/env python3
"""Build the Tauri updater notes payload for one stable version.

Reads the matching ``## vX.Y.Z - YYYY-MM-DD`` section from the English and
Simplified Chinese changelogs. Stable packaging fails when either section is
missing, empty, differently dated, or too large to publish in ``latest.json``.

The notes string is plain text an already-installed client can show without a
parser: Simplified Chinese first, then English. Newer clients split on the
same headings and show the active language.
"""

from __future__ import annotations

import json
import re
import sys
from pathlib import Path

MAX_CHARS = 20000
ZH_HEADING = "简体中文"
EN_HEADING = "English"
EN_SEPARATOR = f"\n\n{EN_HEADING}\n\n"


def extract(path: Path, version: str) -> tuple[str, str]:
    text = path.read_text(encoding="utf-8")
    pattern = re.compile(
        rf"^## v{re.escape(version)} - (\d{{4}}-\d{{2}}-\d{{2}})[ \t]*$",
        re.MULTILINE,
    )
    matches = list(pattern.finditer(text))
    if len(matches) != 1:
        raise SystemExit(
            f"{path}: expected exactly one ## v{version} section, found {len(matches)}"
        )
    match = matches[0]
    start = match.end()
    nxt = re.search(r"^## ", text[start:], re.MULTILINE)
    body = text[start : start + nxt.start()] if nxt else text[start:]
    body = re.sub(r"\n---\s*$", "", body.strip()).strip()
    if not body:
        raise SystemExit(f"{path}: empty changelog section for v{version}")
    if len(body) > MAX_CHARS:
        raise SystemExit(
            f"{path}: changelog section exceeds {MAX_CHARS} characters"
        )
    if re.search(r"[\x00-\x08\x0b\x0c\x0e-\x1f]", body):
        raise SystemExit(f"{path}: changelog section contains control characters")
    return match.group(1), body


def main() -> None:
    if len(sys.argv) != 4:
        raise SystemExit(
            "usage: extract-release-notes.py VERSION CHANGELOG_EN CHANGELOG_ZH"
        )
    version, en_path, zh_path = sys.argv[1:]
    if re.fullmatch(r"\d+\.\d+\.\d+", version) is None:
        raise SystemExit("version must be stable SemVer without a v prefix")
    en_date, en_body = extract(Path(en_path), version)
    zh_date, zh_body = extract(Path(zh_path), version)
    if en_date != zh_date:
        raise SystemExit(
            f"changelog dates differ: en {en_date}, zh-CN {zh_date}"
        )
    if EN_SEPARATOR in f"\n\n{zh_body}\n\n" or zh_body.startswith(f"{EN_HEADING}\n"):
        raise SystemExit(
            "Chinese changelog contains the English notes separator"
        )
    notes = f"{ZH_HEADING}\n\n{zh_body}{EN_SEPARATOR}{en_body}\n"
    json.dump(
        {"notes": notes, "pub_date": f"{en_date}T00:00:00Z"},
        sys.stdout,
        ensure_ascii=False,
    )
    sys.stdout.write("\n")


if __name__ == "__main__":
    main()
