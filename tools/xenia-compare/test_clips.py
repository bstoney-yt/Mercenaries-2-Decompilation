"""Tests for clips.py: parsers, the case format, the GPU lock, and compose
smoke tests on synthetic clips made with ffmpeg's lavfi sources (no game
footage, no GPU).

    python -I -m unittest discover -s tools/xenia-compare -p "test_*.py"
"""

from __future__ import annotations

import json
import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import clips

HAVE_FFMPEG = Path(clips.FFMPEG).exists()


def case(**kw) -> dict:
    c = {
        "name": "t",
        "caption": "A test: caption's \"quotes\", commas, 50%",
        "duration": 2,
        "fps": 30,
        "size": "160x120",
        "ours": {"start": "t:0"},
        "xenia": {"start": "t:0"},
    }
    c.update(kw)
    return clips.check_case(json.loads(json.dumps(c)))


ROUTE_LOG = """    0.00 t0 epoch 1795654800.250
    0.01 xenia pid 1: --gpu=d3d12
    1.02 window 0x56023c screenshot menu item (57148129, 3)
   27.09 shot tap-004.png 1280x720 mean=6.2 rgb=(8, 7, 3)
   80.71 mark OilCon020 start
   90.00 mark crash
   91.00 shot after-000.png 1280x720 mean=40.0 rgb=(1, 2, 3)
   92.00 shot after-001 FAILED
"""


class Parsers(unittest.TestCase):
    def test_route_log(self):
        r = clips.parse_route_log(ROUTE_LOG)
        self.assertEqual(r["t0"], 1795654800.25)
        self.assertEqual(r["window"], 0x56023C)
        self.assertEqual(r["marks"], [(80.71, "OilCon020 start"), (90.0, "crash")])
        self.assertEqual(r["shots"], [(27.09, "tap-004.png"), (91.0, "after-000.png")])

    def test_event_times(self):
        r = clips.parse_route_log(ROUTE_LOG)
        self.assertEqual(clips.event_time("mark:crash", "xenia", r), 90.0)
        self.assertEqual(clips.event_time("mark:OilCon020", "xenia", r), 80.71)
        self.assertEqual(clips.event_time("shot:after-000", "xenia", r), 91.0)
        self.assertEqual(clips.event_time("t:4.5", "xenia", r), 4.5)
        err = 'autoplay 3.0 s: at [1, 2, 3]\nautoplay 31.5 s: objectives ["Wait for weapon delivery"]\n'
        spec = r'log:autoplay ([\d.]+) s: objectives \["Wait for weapon delivery"\]'
        self.assertEqual(clips.event_time(spec, "ours", m2_err=err), 31.5)
        with self.assertRaises(clips.CaseError):
            clips.event_time("mark:nope", "xenia", r)
        with self.assertRaises(clips.CaseError):
            clips.event_time(r"log:never ([\d.]+)", "ours", m2_err=err)

    def test_m2_shots_and_ffmpeg_start(self):
        err = "mission-shot: 25 s: C:\\x\\m2-t025.0.ppm\nother\nmission-shot: 25.1 s: C:\\x\\m2-t025.1.ppm\n"
        self.assertEqual(clips.parse_m2_shots(err), [(25.0, "C:\\x\\m2-t025.0.ppm"), (25.1, "C:\\x\\m2-t025.1.ppm")])
        st = "Input #0, gdigrab, from 'title=Xenia':\n  Duration: N/A, start: 1795654812.345000, bitrate: N/A\n"
        self.assertEqual(clips.ffmpeg_start_epoch(st), 1795654812.345)
        self.assertIsNone(clips.ffmpeg_start_epoch("start: 0.000000"))

    def test_mission_shots_list(self):
        self.assertEqual(clips.mission_shots({"from": 25, "to": 25.5, "fps": 10}), "25,25.1,25.2,25.3,25.4,25.5")
        self.assertEqual(clips.mission_shots({"from": 0, "to": 1, "fps": 5}), "0,0.2,0.4,0.6,0.8,1")


class CaseFormat(unittest.TestCase):
    def test_defaults(self):
        c = case()
        self.assertEqual(c["labels"], ["Ours", "Xenia"])
        self.assertEqual(c["xenia"]["capture"], "video")
        self.assertEqual(c["slowmo"], 1.0)

    def test_bad_cases(self):
        bad = [
            {"name": "a b"},
            {"duration": 0},
            {"size": "161x120"},
            {"slowmo": 50},
            {"ours": {"start": "mark:x"}},
            {"xenia": {"start": "log:(x)"}},
            {"ours": {"start": "log:no group"}},
            {"ours": {"shots": {"from": 0, "to": 5, "fps": 30}}},
            {"xenia": {"capture": "film"}},
            {"xenia": {"routes": ["no-such-route.txt"]}},
        ]
        for b in bad:
            with self.subTest(b=b), self.assertRaises(clips.CaseError):
                case(**b)

    def test_example_cases_are_valid(self):
        d = Path(__file__).resolve().parent / "cases"
        files = sorted(d.glob("*.json"))
        self.assertGreaterEqual(len(files), 3)
        for f in files:
            c = clips.load_case(f)
            self.assertTrue(c.get("fix", "").startswith("#"), f)
            text = f.read_text(encoding="utf-8")
            self.assertNotIn(":\\\\", text, f"{f.name} holds a path on the VM")

    def test_notes(self):
        n = clips.notes(case(fix="#776", summary="One line."))
        self.assertIn("### A test", n)
        self.assertIn("Fixed in #776.", n)
        self.assertIn("(CLIP_LINK)", n)
        self.assertIn("Ours (left)", n)
        n = clips.notes(case(swap=True))
        self.assertIn("Xenia (left)", n)


class Lock(unittest.TestCase):
    def test_stale(self):
        o = {"pid": 9, "started_unix": 1000, "purpose": "x"}
        self.assertFalse(clips.lock_stale(o, [("a.exe", 9)], 2000))
        self.assertTrue(clips.lock_stale(o, [("a.exe", 9)], 1000 + 9 * 3600))
        self.assertTrue(clips.lock_stale(o, [], 2000))

    def test_busy(self):
        procs = [("m2-alpha.exe", 1), ("m2-alpha-xenia.exe", 2), ("cargo.exe", 3), ("xenia_canary.exe", 4)]
        self.assertEqual([p for _, p in clips.busy(procs)], [1, 2, 4])

    def test_take_and_refuse(self):
        with tempfile.TemporaryDirectory() as d:
            p = Path(d) / "gpu.lock"
            lock, owner = clips.take_lock(p, "test")
            self.assertEqual(lock, p)
            if os.name == "nt":  # the pid check reads tasklist
                again, owner = clips.take_lock(p, "second")
                self.assertIsNone(again)
                self.assertEqual(owner["purpose"], "test")
            p.unlink()
            # A lock left by a dead pid is taken over.
            p.write_text('{"pid": 4000000000, "started_unix": 1, "purpose": "old"}', encoding="utf-8")
            lock, _ = clips.take_lock(p, "new")
            self.assertEqual(json.loads(p.read_text(encoding="utf-8"))["purpose"], "new")


def lavfi_flash(path: Path, flash_at: float, seconds: float = 6, size: str = "320x240", rate: int = 30):
    """Black, turning white at `flash_at`: the shared start event."""
    subprocess.run(
        [clips.FFMPEG, "-hide_banner", "-loglevel", "error", "-y", "-f", "lavfi",
         "-i", f"color=black:s={size}:r={rate}:d={seconds}",
         "-vf", f"drawbox=x=0:y=0:w=iw:h=ih:color=white:t=fill:enable='gte(t,{flash_at})',format=yuv420p",
         "-c:v", "libx264", "-preset", "ultrafast", str(path)],
        check=True)


def gray_frame(path: Path, at: float = 0.0) -> tuple[int, int, bytes]:
    """A frame of `path` as 8-bit gray: (width, height, pixels)."""
    probe = subprocess.run(
        [clips.FFMPEG, "-hide_banner", "-i", str(path)], capture_output=True, text=True, check=False).stderr
    import re
    w, h = (int(v) for v in re.search(r", (\d{2,5})x(\d{2,5})", probe).groups())
    raw = subprocess.run(
        [clips.FFMPEG, "-hide_banner", "-loglevel", "error", "-ss", f"{at}", "-i", str(path),
         "-frames:v", "1", "-f", "rawvideo", "-pix_fmt", "gray", "-"],
        capture_output=True, check=True).stdout
    return w, h, raw


def luma(w: int, raw: bytes, x: int, y: int) -> int:
    return raw[y * w + x]


@unittest.skipUnless(HAVE_FFMPEG, "needs the local ffmpeg (M2_FFMPEG)")
class ComposeSmoke(unittest.TestCase):
    def test_compose_aligns_on_the_start_events(self):
        with tempfile.TemporaryDirectory() as d:
            d = Path(d)
            ours, xen = d / "ours.mp4", d / "xenia.mp4"
            lavfi_flash(ours, 2.0)
            lavfi_flash(xen, 3.5, size="640x360")
            c = case(duration=2)
            meta = clips.compose(c, ours, xen, 2.0, 3.5, d / "out")
            mp4 = d / "out" / "side-by-side.mp4"
            self.assertTrue(mp4.exists())
            for f in ("side-by-side.gif", "thumb.png", "notes.md", "compose.json"):
                self.assertTrue((d / "out" / f).exists(), f)
            self.assertEqual(meta["ours_start"], 2.0)
            w, h, raw = gray_frame(mp4, 0.1)
            self.assertEqual((w, h), (320, 120 + 24))  # two 160x120 halves + caption bar
            # Both halves white from the first frame: aligned on the flash.
            self.assertGreater(luma(w, raw, 80, 90), 200)
            self.assertGreater(luma(w, raw, 240, 90), 200)

    def test_misaligned_starts_show(self):
        with tempfile.TemporaryDirectory() as d:
            d = Path(d)
            ours, xen = d / "ours.mp4", d / "xenia.mp4"
            lavfi_flash(ours, 2.0)
            lavfi_flash(xen, 3.5)
            # Xenia started at its 2.0 s: still black for 1.5 s.
            clips.compose(case(), ours, xen, 2.0, 2.0, d / "out")
            w, _, raw = gray_frame(d / "out" / "side-by-side.mp4", 0.5)
            self.assertGreater(luma(w, raw, 80, 90), 200)
            self.assertLess(luma(w, raw, 240, 90), 40)
            # The xenia offset fixes it (frame sync offset).
            c = case(xenia={"start": "t:0", "offset": 1.5})
            clips.compose(c, ours, xen, 2.0, 2.0, d / "out2")
            w, _, raw = gray_frame(d / "out2" / "side-by-side.mp4", 0.5)
            self.assertGreater(luma(w, raw, 240, 90), 200)

    def test_swap_and_slowmo(self):
        with tempfile.TemporaryDirectory() as d:
            d = Path(d)
            ours, xen = d / "ours.mp4", d / "xenia.mp4"
            lavfi_flash(ours, 0.0)  # white
            lavfi_flash(xen, 99.0)  # black
            clips.compose(case(swap=True, slowmo=2.0, duration=1), ours, xen, 0, 0, d / "out")
            mp4 = d / "out" / "side-by-side.mp4"
            w, _, raw = gray_frame(mp4, 0.2)
            self.assertLess(luma(w, raw, 80, 90), 40)  # Xenia on the left
            self.assertGreater(luma(w, raw, 240, 90), 200)
            # 1 s at half speed lasts 2 s.
            probe = subprocess.run([clips.FFMPEG, "-hide_banner", "-i", str(mp4)], capture_output=True, text=True, check=False).stderr
            self.assertIn("Duration: 00:00:02.0", probe)

    def test_negative_start_holds_the_first_frame(self):
        with tempfile.TemporaryDirectory() as d:
            d = Path(d)
            ours, xen = d / "ours.mp4", d / "xenia.mp4"
            lavfi_flash(ours, 0.0)
            lavfi_flash(xen, 0.0)
            clips.compose(case(duration=1), ours, xen, -0.5, 0, d / "out")
            self.assertTrue((d / "out" / "side-by-side.mp4").exists())

    def test_frames_to_clip_from_timed_stills(self):
        with tempfile.TemporaryDirectory() as d:
            d = Path(d)
            frames = []
            for i, t in enumerate([10.0, 10.1, 10.2, 10.5, 11.0]):
                p = d / f"f{i}.ppm"
                v = 255 if t >= 10.5 else 0
                p.write_bytes(b"P6\n16 16\n255\n" + bytes([v]) * (16 * 16 * 3))
                frames.append((t, p))
            clip = d / "frames.mp4"
            first = clips.frames_to_clip(frames, clip, 30)
            self.assertEqual(first, 10.0)
            _, _, raw = gray_frame(clip, 0.2)
            self.assertLess(raw[0], 40)
            _, _, raw = gray_frame(clip, 0.6)
            self.assertGreater(raw[0], 200)

    def test_compose_cli_from_a_frames_folder(self):
        with tempfile.TemporaryDirectory() as d:
            d = Path(d)
            fr = d / "frames"
            fr.mkdir()
            for i in range(10):
                (fr / f"m2-t{i:03d}.ppm").write_bytes(b"P6\n16 16\n255\n" + bytes([200]) * 768)
            xen = d / "xenia.mp4"
            lavfi_flash(xen, 0.0)
            cf = d / "case.json"
            cf.write_text(json.dumps({"name": "cli", "caption": "c", "duration": 0.8, "size": "160x120",
                                      "ours": {}, "xenia": {}}), encoding="utf-8")
            rc = clips.main(["compose", str(cf), "--ours", str(fr), "--xenia", str(xen), "--out", str(d / "o")])
            self.assertEqual(rc, 0)
            self.assertTrue((d / "o" / "side-by-side.mp4").exists())


def lavfi_noise(path: Path, seconds: float = 6, size: str = "320x240"):
    """Moving noise: a GIF of it is large, to make the cap work."""
    subprocess.run(
        [clips.FFMPEG, "-hide_banner", "-loglevel", "error", "-y", "-f", "lavfi",
         "-i", f"testsrc2=s={size}:r=30:d={seconds}", "-vf", "noise=alls=60:allf=t,format=yuv420p",
         "-c:v", "libx264", "-preset", "ultrafast", "-crf", "10", str(path)],
        check=True)


@unittest.skipUnless(HAVE_FFMPEG, "needs the local ffmpeg (M2_FFMPEG)")
class GifCapAndSources(unittest.TestCase):
    def test_the_gif_stays_under_its_cap(self):
        with tempfile.TemporaryDirectory() as d:
            d = Path(d)
            a, b = d / "a.mp4", d / "b.mp4"
            lavfi_noise(a)
            lavfi_noise(b)
            c = case(duration=5, size="320x240", gif={"max_mb": 0.4})
            meta = clips.compose(c, a, b, 0, 0, d / "out")
            gif = d / "out" / "side-by-side.gif"
            self.assertLessEqual(gif.stat().st_size, 0.4 * 1024 * 1024)
            g = meta["gif"]
            self.assertEqual(g["bytes"], gif.stat().st_size)
            # It had to step down from the first try.
            self.assertTrue((g["fps"], g["width"]) != (12, 640) or g["excerpt"], g)
            # The MP4 is whole either way.
            self.assertGreater(clips.clip_seconds(d / "out" / "side-by-side.mp4"), 4.5)

    def test_a_tight_cap_cuts_an_excerpt(self):
        with tempfile.TemporaryDirectory() as d:
            d = Path(d)
            a = d / "a.mp4"
            lavfi_noise(a)
            c = case(duration=5, size="320x240", gif={"max_mb": 0.05})
            meta = clips.compose(c, a, a, 0, 0, d / "out")
            g = meta["gif"]
            self.assertIsNotNone(g["excerpt"], g)
            self.assertLess(g["excerpt"][1], 5.0)
            self.assertEqual((g["fps"], g["width"]), clips.GIF_LADDER[-1])

    def test_sources_say_screenshots_and_their_rate(self):
        with tempfile.TemporaryDirectory() as d:
            d = Path(d)
            frames = []
            for i in range(10):  # one a second, as Xenia's route shots
                p = d / f"s{i}.ppm"
                p.write_bytes(b"P6\n16 16\n255\n" + bytes([i * 20]) * 768)
                frames.append((100.0 + i, p))
            shots = d / "xenia-frames.mp4"
            clips.frames_to_clip(frames, shots, 30)
            live = d / "ours.mp4"
            lavfi_flash(live, 0.0, seconds=10)
            s = clips.describe_source(shots, 2.0, 5.0)
            self.assertEqual(s, {"source": "screenshots", "fps": 1.0})
            v = clips.describe_source(live, 0.0, 5.0)
            self.assertEqual(v["source"], "video")
            self.assertEqual(v["fps"], 30.0)
            meta = clips.compose(case(duration=5), live, shots, 0, 2.0, d / "out")
            self.assertEqual(meta["sources"]["xenia"]["source"], "screenshots")
            n = (d / "out" / "notes.md").read_text(encoding="utf-8")
            self.assertIn("Xenia: still screenshots, not live video, about 1 a second", n)
            self.assertIn("Ours: live video, about 30 a second", n)

    def test_notes_without_sources_say_nothing_of_capture(self):
        self.assertNotIn("Capture", clips.notes(case()))

    def test_bad_gif_cap(self):
        with self.assertRaises(clips.CaseError):
            case(gif={"max_mb": 0})


if __name__ == "__main__":
    unittest.main()
