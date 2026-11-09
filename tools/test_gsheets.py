#!/usr/bin/env python3
"""Test the local half of the Google Sheets bridge without touching Google.

The parts that need Google (OAuth, export, batchUpdate) are thin wrappers
around the Sheets API. The parts that carry our logic - reading a workbook
with iridium-sheets-cli, picking the sheet, turning the used range into an A1
range - are all exercised here against a real workbook.

    python3 tools/test_gsheets.py
"""

from __future__ import annotations

import json
import os
import subprocess
import sys
import tempfile
from pathlib import Path

HERE = Path(__file__).parent
sys.path.insert(0, str(HERE))

import gsheets  # noqa: E402

CLI = Path(os.environ.get("IRIDIUM_CLI", "iridium-sheets-cli")).resolve()


def check(label: str, got, want) -> None:
    if got != want:
        print(f"FAIL {label}\n  got:  {got!r}\n  want: {want!r}")
        raise SystemExit(1)
    print(f"ok   {label}")


def main() -> int:
    if not CLI.exists():
        print(f"FAIL iridium-sheets-cli not found at {CLI}")
        print("  build it: cargo build --release -p iridium-sheets-cli")
        return 1

    gsheets.CLI = CLI

    with tempfile.TemporaryDirectory() as td:
        td = Path(td)
        xlsx = td / "budget.xlsx"

        # Build a workbook with Sheets itself.
        proc = subprocess.run(
            [str(CLI), "run", "--sample", "budget", "--out", str(xlsx), "--quiet"],
            capture_output=True, text=True, timeout=300,
        )
        if proc.returncode != 0:
            print(f"FAIL could not build the fixture workbook:\n{proc.stderr}")
            return 1
        check("workbook was written", xlsx.exists(), True)

        # 1. local_sheets finds the sheet and its used range.
        sheets = gsheets.local_sheets(xlsx)
        check("found one sheet", len(sheets), 1)
        check("sheet is named Budget", sheets[0]["name"], "Budget")
        check("used range is not empty", bool(sheets[0]["usedRange"]), True)

        # 2. The used range converts to an A1 range.
        start, end = gsheets.a1_to_a1(sheets[0]["usedRange"])
        check("range start is a cell", start[0].isalpha() and start[1:].isdigit(), True)
        check("range end is a cell", end[0].isalpha() and end[1:].isdigit(), True)

        # 3. Values come back as rows.
        rows = gsheets.read_values(xlsx, None)
        check("values have rows", len(rows) > 0, True)
        flat = [c for r in rows for c in r]
        check("the title cell is present", any("Monthly Budget" in c for c in flat), True)
        check("a formula result is present", any("1,850" in c for c in flat), True)

        # 4. A missing sheet is an error, not a crash.
        try:
            gsheets.read_values(xlsx, "No Such Sheet")
        except SystemExit:
            print("ok   missing sheet raises a clean error")
        else:
            print("FAIL missing sheet should raise")
            return 1

        # 5. An empty used range must not produce a bogus range.
        check("empty range falls back", gsheets.a1_to_a1(""), ("A1", "A1"))

    print("\nall local checks passed")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
