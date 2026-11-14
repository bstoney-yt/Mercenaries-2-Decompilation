// scratch (not committed)
use std::path::Path;
fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let install = m2_engine::Install::find(Path::new(&a[0])).unwrap();
    let mut source = install.open_world().unwrap();
    let id = u32::from_str_radix(&a[1], 16).unwrap();
    let d = source
        .asset(id, m2_engine::tuning::DAMAGE_TABLE_TYPE)
        .unwrap();
    let w = |o: usize| u32::from_le_bytes(d[o..o + 4].try_into().unwrap());
    let (rows, cols) = (w(0x48) as usize, w(0x4c) as usize);
    let rk: Vec<u32> = (0..rows).map(|i| w(0x50 + 4 * i)).collect();
    let ck: Vec<u32> = (0..cols).map(|i| w(0x50 + 4 * (rows + i))).collect();
    let cells = 0x50 + 4 * (rows + cols);
    println!(
        "rows {rows}: {}",
        rk.iter()
            .map(|k| format!("{k:08x}"))
            .collect::<Vec<_>>()
            .join(",")
    );
    println!(
        "cols {cols}: {}",
        ck.iter()
            .map(|k| format!("{k:08x}"))
            .collect::<Vec<_>>()
            .join(",")
    );
    let filt: Vec<u32> = a[2..]
        .iter()
        .map(|h| u32::from_str_radix(h, 16).unwrap())
        .collect();
    for r in 0..rows {
        for c in 0..cols {
            if !filt.is_empty() && !filt.contains(&rk[r]) && !filt.contains(&ck[c]) {
                continue;
            }
            let o = cells + 8 * (r * cols + c);
            let (x, y) = (w(o), w(o + 4));
            if x != 0 || y != 0 {
                println!("{:08x} x {:08x}: {x:08x} {y:08x}", rk[r], ck[c]);
            }
        }
    }
}
