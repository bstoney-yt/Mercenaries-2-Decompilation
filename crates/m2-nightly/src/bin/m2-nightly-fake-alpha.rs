//! A stand-in for m2-alpha in the runner's tests: it writes a synthetic
//! autoplay log to stderr and behaves by the `--skip-to` contract id
//! (`FakePass`, `FakePanic`, `FakeHang`, `FakeStuck`, `FakeCrash`). It
//! reads no game data and draws nothing.

use std::io::Write as _;
use std::time::Duration;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let id = args
        .iter()
        .position(|a| a == "--skip-to")
        .and_then(|i| args.get(i + 1))
        .cloned()
        .unwrap_or_else(|| "FakePass".into());
    let mut e = std::io::stderr();
    let _ = writeln!(e, "autoplay 1.0 s: objectives [\"Go\"]");
    let _ = writeln!(e, "autoplay 2.0 s: at [0.0, -30.0, 0.0], on foot");
    match id.as_str() {
        "FakePass" => {
            let _ = writeln!(e, "autoplay 30.0 s: objectives []");
            let _ = writeln!(e, "autoplay: contract 1 completed after 30.0 s of play");
            let _ = writeln!(e, "autoplay: 31.0 s played, 0 hops;");
        }
        "FakePanic" => {
            let _ = writeln!(
                e,
                "thread 'main' panicked at crates/m2-alpha/src/fake.rs:10:5:"
            );
            let _ = writeln!(e, "boom");
            std::process::exit(101);
        }
        "FakeCrash" => std::process::exit(0xC000_0005_u32 as i32),
        "FakeHang" => std::thread::sleep(Duration::from_secs(600)),
        "FakeStuck" => {
            for t in (10..2000).step_by(10) {
                let _ = writeln!(e, "autoplay {t}.0 s: at [5.0, -30.0, 5.0], on foot");
                let _ = e.flush();
                std::thread::sleep(Duration::from_millis(20));
            }
            std::thread::sleep(Duration::from_secs(600));
        }
        _ => {}
    }
}
