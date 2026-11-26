"""Tests for changelog.py on made-up pull requests (no network).

    python -I -m unittest discover -s tools/changelog -p "test_*.py"
"""

import sys
import tomllib
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import changelog

BODY = """<!-- claude-projects-attribution: {"github_login":"someone"} -->
_Requested by **Someone** · [project thread](https://example.invalid/x)_

## Pass condition (written before any check run)

1. CI green.

The launcher's **Stats** page shows each save's `play time`. More detail follows here.

- a list item
"""


class Summary(unittest.TestCase):
    def test_skips_attribution_headings_and_lists(self):
        self.assertEqual(changelog.summary(BODY), "The launcher's Stats page shows each save's play time.")

    def test_drops_paths_addresses_and_mentions(self):
        self.assertEqual(changelog.summary("Logs go to C:\\Claude\\scratch now."), "")
        self.assertEqual(changelog.summary("See https://example.invalid for more."), "")
        self.assertEqual(changelog.summary("Thanks @someone for this."), "")
        self.assertEqual(changelog.summary("Logs go to C:\\x.\n\nA plain change."), "A plain change.")

    def test_private_words_and_lead_ins_are_skipped(self):
        body = "Alice saw it break.\n\nThe crate now opens."
        self.assertEqual(changelog.summary(body, ("alice",)), "The crate now opens.")
        self.assertEqual(changelog.summary("New tool:\n\nIt runs nightly."), "It runs nightly.")

    def test_long_sentences_are_cut(self):
        s = changelog.summary("word " * 80)
        self.assertLessEqual(len(s), changelog.SUMMARY_MAX)
        self.assertTrue(s.endswith("..."))

    def test_empty(self):
        self.assertEqual(changelog.summary(None), "")
        self.assertEqual(changelog.summary("## Only a heading"), "")


class Entries(unittest.TestCase):
    PRS = (
        {"number": 5, "title": "Old fix", "body": "Fixes a thing.", "mergedAt": "2026-11-20T23:59:59Z"},
        {"number": 9, "title": "New `tab`", "body": None, "mergedAt": "2026-11-25T10:00:00Z"},
        {"number": 7, "title": "Same day", "body": "Also.", "mergedAt": "2026-11-25T01:00:00Z"},
        {"number": 8, "title": "Not merged", "body": "", "mergedAt": None},
    )

    def test_newest_first_and_plain_titles(self):
        es = changelog.entries(list(self.PRS))
        self.assertEqual([e["pr"] for e in es], [9, 7, 5])
        self.assertEqual(es[0]["title"], "New tab")
        self.assertEqual(es[0]["date"], "2026-11-25")

    def test_toml_reads_back(self):
        text = changelog.to_toml(changelog.entries(list(self.PRS)), "2026-11-26")
        data = tomllib.loads(text)
        self.assertEqual(data["generated"], "2026-11-26")
        self.assertEqual(len(data["entry"]), 3)
        self.assertNotIn("summary", data["entry"][0])
        self.assertEqual(data["entry"][2]["summary"], "Fixes a thing.")
        # Quotes and backslashes survive.
        t = changelog.to_toml([{"pr": 1, "date": "2026-01-01", "title": 'a "b" \\ c', "summary": ""}], "x")
        self.assertEqual(tomllib.loads(t)["entry"][0]["title"], 'a "b" \\ c')

    def test_the_checked_in_file_reads(self):
        p = Path(__file__).resolve().parents[2] / "docs" / "changelog.toml"
        data = tomllib.loads(p.read_text(encoding="utf-8"))
        self.assertGreater(len(data["entry"]), 0)
        dates = [e["date"] for e in data["entry"]]
        self.assertEqual(dates, sorted(dates, reverse=True))


if __name__ == "__main__":
    unittest.main()
