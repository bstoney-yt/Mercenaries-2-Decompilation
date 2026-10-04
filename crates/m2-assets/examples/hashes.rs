//! Looks for the hash functions behind two sets of numbers in an archive
//! from your own installation: the keys of its string tables (hashes of
//! text ids such as `VzaCon001.Objectives.001`) and its assets' name
//! hashes.
//!
//! ```text
//! cargo run --release -p m2-assets --example hashes -- <archive.wad>
//! ```
//!
//! Every string the scripts hold is a candidate name, as are the parts of
//! it (the ids in `[...]`, what follows each `.`, `\` or `/`) and the names
//! assets give themselves. Each candidate, as is, in lower and in upper
//! case, as bytes and as UTF-16, goes through a battery of well-known
//! 32-bit string hashes. A function that is the right one finds hundreds
//! of names; chance finds a handful at most. Prints, per set, the
//! combinations that found the most, with a few of the names found.

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::fs::File;
use std::io::BufReader;
use std::process::ExitCode;

use m2_assets::lua::{Chunk, SCRIPT_TYPE};
use m2_assets::strings::STRINGS_TYPE;
use m2_assets::{BlockContents, Ucfx, WadArchive};

fn main() -> ExitCode {
    let Some(path) = std::env::args().nth(1) else {
        eprintln!("usage: hashes <archive.wad>");
        return ExitCode::FAILURE;
    };
    match run(&path) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run(path: &str) -> Result<(), Box<dyn std::error::Error>> {
    let mut wad = WadArchive::open(BufReader::new(File::open(path)?))?;
    let assets: HashSet<u32> = wad.index().assets().iter().map(|a| a.name_hash).collect();
    let mut keys = HashSet::new();
    let mut names = BTreeSet::new();
    for block in 0..wad.index().blocks().len() {
        names.insert(wad.index().blocks()[block].path.clone());
        let Ok(decoded) = wad.read_block(block) else {
            continue;
        };
        let Ok(contents) = BlockContents::parse(&decoded) else {
            continue;
        };
        for (entry, data) in contents.iter() {
            if let Some(n) = Ucfx::parse(data).ok().as_ref().and_then(|u| u.name()) {
                names.insert(n.to_owned());
            }
            if entry.type_hash == STRINGS_TYPE
                && let Some(k) = Ucfx::parse(data).ok().and_then(|u| u.top_level(*b"KEYS"))
            {
                for c in k.get(4..).unwrap_or_default().chunks_exact(8) {
                    keys.insert(u32::from_le_bytes([c[0], c[1], c[2], c[3]]));
                }
            }
            if entry.type_hash == SCRIPT_TYPE
                && let Ok(chunk) = Chunk::from_asset(data)
            {
                names.extend(chunk.main.strings().into_iter().map(str::to_owned));
            }
        }
    }
    let candidates = candidates(&names);
    println!(
        "{} string keys, {} asset names, {} candidate names",
        keys.len(),
        assets.len(),
        candidates.len()
    );
    report("string keys", &keys, &candidates);
    report("asset names", &assets, &candidates);
    Ok(())
}

/// Each name and its parts.
fn candidates(names: &BTreeSet<String>) -> Vec<String> {
    let mut out = BTreeSet::new();
    for n in names {
        if n.is_empty() || n.len() > 200 {
            continue;
        }
        out.insert(n.clone());
        let mut rest = n.as_str();
        while let Some(open) = rest.find('[') {
            let Some(close) = rest[open..].find(']') else {
                break;
            };
            out.insert(rest[open + 1..open + close].to_owned());
            out.insert(rest[open..=open + close].to_owned());
            rest = &rest[open + close + 1..];
        }
        for (i, c) in n.char_indices() {
            if matches!(c, '.' | '\\' | '/') && i + 1 < n.len() {
                out.insert(n[i + 1..].to_owned());
                out.insert(n[..i].to_owned());
            }
        }
    }
    out.into_iter().filter(|s| !s.is_empty()).collect()
}

fn report(what: &str, targets: &HashSet<u32>, candidates: &[String]) {
    type Hash = fn(&[u8]) -> u32;
    let functions: [(&str, Hash); 19] = [
        ("fnv1", fnv1),
        ("fnv1a", fnv1a),
        ("djb2", djb2),
        ("djb2 xor", djb2x),
        ("sdbm", sdbm),
        ("x31", x31),
        ("x65599", x65599),
        ("one-at-a-time", oat),
        ("elf", elf),
        ("crc32", crc32),
        ("crc32 no final xor", |b| !crc32(b)),
        ("crc32 init 0", crc32_init0),
        ("crc32 msb-first", crc32_msb),
        ("crc32 msb-first no final xor", |b| !crc32_msb(b)),
        ("lookup2", lookup2),
        ("lookup3", lookup3),
        ("murmur2", murmur2),
        ("murmur3", murmur3),
        ("fnv1a 64 low", fnv1a64_low),
    ];
    let mut found: BTreeMap<(usize, &str, &str), Vec<&str>> = BTreeMap::new();
    for name in candidates {
        let cases = [
            ("as is", name.clone()),
            ("lower", name.to_lowercase()),
            ("upper", name.to_uppercase()),
        ];
        for (case, text) in &cases {
            if *case != "as is" && text == name {
                continue;
            }
            let wide: Vec<u8> = text.encode_utf16().flat_map(u16::to_le_bytes).collect();
            for (enc, bytes) in [("bytes", text.as_bytes()), ("utf-16", &wide[..])] {
                for (fi, (f, hash)) in functions.iter().enumerate() {
                    if targets.contains(&hash(bytes)) {
                        let label: &'static str = match (*case, enc) {
                            ("as is", "bytes") => "as is",
                            ("lower", "bytes") => "lower",
                            ("upper", "bytes") => "upper",
                            ("as is", _) => "as is, utf-16",
                            ("lower", _) => "lower, utf-16",
                            _ => "upper, utf-16",
                        };
                        found.entry((fi, f, label)).or_default().push(name);
                    }
                }
            }
        }
    }
    let mut rows: Vec<_> = found.into_iter().collect();
    rows.sort_by_key(|(_, v)| std::cmp::Reverse(v.len()));
    println!();
    println!("{what}: most names found");
    for ((_, f, case), names) in rows.iter().take(12) {
        let some: Vec<String> = names
            .iter()
            .take(4)
            .map(|n| format!("{:?}", n.chars().take(40).collect::<String>()))
            .collect();
        println!("  {:5}  {f}, {case}: {}", names.len(), some.join(", "));
    }
    if rows.is_empty() {
        println!("  none");
    }
}

fn fnv1(b: &[u8]) -> u32 {
    b.iter().fold(0x811c_9dc5u32, |h, &c| {
        h.wrapping_mul(0x0100_0193) ^ u32::from(c)
    })
}

fn fnv1a(b: &[u8]) -> u32 {
    b.iter().fold(0x811c_9dc5u32, |h, &c| {
        (h ^ u32::from(c)).wrapping_mul(0x0100_0193)
    })
}

fn fnv1a64_low(b: &[u8]) -> u32 {
    b.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, &c| {
        (h ^ u64::from(c)).wrapping_mul(0x0000_0100_0000_01b3)
    }) as u32
}

fn djb2(b: &[u8]) -> u32 {
    b.iter().fold(5381u32, |h, &c| {
        h.wrapping_mul(33).wrapping_add(u32::from(c))
    })
}

fn djb2x(b: &[u8]) -> u32 {
    b.iter()
        .fold(5381u32, |h, &c| h.wrapping_mul(33) ^ u32::from(c))
}

fn sdbm(b: &[u8]) -> u32 {
    b.iter().fold(0u32, |h, &c| {
        u32::from(c)
            .wrapping_add(h << 6)
            .wrapping_add(h << 16)
            .wrapping_sub(h)
    })
}

fn x31(b: &[u8]) -> u32 {
    b.iter()
        .fold(0u32, |h, &c| h.wrapping_mul(31).wrapping_add(u32::from(c)))
}

fn x65599(b: &[u8]) -> u32 {
    b.iter().fold(0u32, |h, &c| {
        h.wrapping_mul(65599).wrapping_add(u32::from(c))
    })
}

fn oat(b: &[u8]) -> u32 {
    let mut h = b.iter().fold(0u32, |mut h, &c| {
        h = h.wrapping_add(u32::from(c));
        h = h.wrapping_add(h << 10);
        h ^ (h >> 6)
    });
    h = h.wrapping_add(h << 3);
    h ^= h >> 11;
    h.wrapping_add(h << 15)
}

fn elf(b: &[u8]) -> u32 {
    b.iter().fold(0u32, |h, &c| {
        let mut h = (h << 4).wrapping_add(u32::from(c));
        let g = h & 0xf000_0000;
        if g != 0 {
            h ^= g >> 24;
        }
        h & !g
    })
}

fn crc32_with(b: &[u8], init: u32) -> u32 {
    let mut h = init;
    for &c in b {
        h ^= u32::from(c);
        for _ in 0..8 {
            h = if h & 1 != 0 {
                (h >> 1) ^ 0xedb8_8320
            } else {
                h >> 1
            };
        }
    }
    !h
}

fn crc32(b: &[u8]) -> u32 {
    crc32_with(b, !0)
}

fn crc32_init0(b: &[u8]) -> u32 {
    crc32_with(b, 0)
}

fn crc32_msb(b: &[u8]) -> u32 {
    let mut h = !0u32;
    for &c in b {
        h ^= u32::from(c) << 24;
        for _ in 0..8 {
            h = if h & 0x8000_0000 != 0 {
                (h << 1) ^ 0x04c1_1db7
            } else {
                h << 1
            };
        }
    }
    !h
}

fn word(b: &[u8], i: usize) -> u32 {
    u32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]])
}

fn lookup2(k: &[u8]) -> u32 {
    fn mix(a: &mut u32, b: &mut u32, c: &mut u32) {
        *a = a.wrapping_sub(*b).wrapping_sub(*c) ^ (*c >> 13);
        *b = b.wrapping_sub(*c).wrapping_sub(*a) ^ (*a << 8);
        *c = c.wrapping_sub(*a).wrapping_sub(*b) ^ (*b >> 13);
        *a = a.wrapping_sub(*b).wrapping_sub(*c) ^ (*c >> 12);
        *b = b.wrapping_sub(*c).wrapping_sub(*a) ^ (*a << 16);
        *c = c.wrapping_sub(*a).wrapping_sub(*b) ^ (*b >> 5);
        *a = a.wrapping_sub(*b).wrapping_sub(*c) ^ (*c >> 3);
        *b = b.wrapping_sub(*c).wrapping_sub(*a) ^ (*a << 10);
        *c = c.wrapping_sub(*a).wrapping_sub(*b) ^ (*b >> 15);
    }
    let (mut a, mut b, mut c) = (0x9e37_79b9u32, 0x9e37_79b9u32, 0u32);
    let mut i = 0;
    while k.len() - i >= 12 {
        a = a.wrapping_add(word(k, i));
        b = b.wrapping_add(word(k, i + 4));
        c = c.wrapping_add(word(k, i + 8));
        mix(&mut a, &mut b, &mut c);
        i += 12;
    }
    c = c.wrapping_add(k.len() as u32);
    let mut t = [0u8; 12];
    t[..k.len() - i].copy_from_slice(&k[i..]);
    a = a.wrapping_add(word(&t, 0));
    b = b.wrapping_add(word(&t, 4));
    // The low byte of c is the length's.
    c = c.wrapping_add(word(&t, 8) << 8);
    mix(&mut a, &mut b, &mut c);
    c
}

fn lookup3(k: &[u8]) -> u32 {
    let rot = |x: u32, n: u32| x.rotate_left(n);
    let init = 0xdead_beefu32.wrapping_add(k.len() as u32);
    let (mut a, mut b, mut c) = (init, init, init);
    let mut i = 0;
    while k.len() - i > 12 {
        a = a.wrapping_add(word(k, i));
        b = b.wrapping_add(word(k, i + 4));
        c = c.wrapping_add(word(k, i + 8));
        a = a.wrapping_sub(c) ^ rot(c, 4);
        c = c.wrapping_add(b);
        b = b.wrapping_sub(a) ^ rot(a, 6);
        a = a.wrapping_add(c);
        c = c.wrapping_sub(b) ^ rot(b, 8);
        b = b.wrapping_add(a);
        a = a.wrapping_sub(c) ^ rot(c, 16);
        c = c.wrapping_add(b);
        b = b.wrapping_sub(a) ^ rot(a, 19);
        a = a.wrapping_add(c);
        c = c.wrapping_sub(b) ^ rot(b, 4);
        b = b.wrapping_add(a);
        i += 12;
    }
    if k.len() == i {
        return c;
    }
    let mut t = [0u8; 12];
    t[..k.len() - i].copy_from_slice(&k[i..]);
    a = a.wrapping_add(word(&t, 0));
    b = b.wrapping_add(word(&t, 4));
    c = c.wrapping_add(word(&t, 8));
    c ^= b;
    c = c.wrapping_sub(rot(b, 14));
    a ^= c;
    a = a.wrapping_sub(rot(c, 11));
    b ^= a;
    b = b.wrapping_sub(rot(a, 25));
    c ^= b;
    c = c.wrapping_sub(rot(b, 16));
    a ^= c;
    a = a.wrapping_sub(rot(c, 4));
    b ^= a;
    b = b.wrapping_sub(rot(a, 14));
    c ^= b;
    c.wrapping_sub(rot(b, 24))
}

fn murmur2(k: &[u8]) -> u32 {
    const M: u32 = 0x5bd1_e995;
    let mut h = k.len() as u32;
    let mut chunks = k.chunks_exact(4);
    for c in &mut chunks {
        let mut w = word(c, 0).wrapping_mul(M);
        w ^= w >> 24;
        h = h.wrapping_mul(M) ^ w.wrapping_mul(M);
    }
    let r = chunks.remainder();
    if !r.is_empty() {
        for (i, &c) in r.iter().enumerate() {
            h ^= u32::from(c) << (8 * i);
        }
        h = h.wrapping_mul(M);
    }
    h ^= h >> 13;
    h = h.wrapping_mul(M);
    h ^ (h >> 15)
}

fn murmur3(k: &[u8]) -> u32 {
    const C1: u32 = 0xcc9e_2d51;
    const C2: u32 = 0x1b87_3593;
    let mut h = 0u32;
    let mut chunks = k.chunks_exact(4);
    for c in &mut chunks {
        let w = word(c, 0).wrapping_mul(C1).rotate_left(15).wrapping_mul(C2);
        h = (h ^ w)
            .rotate_left(13)
            .wrapping_mul(5)
            .wrapping_add(0xe654_6b64);
    }
    let r = chunks.remainder();
    if !r.is_empty() {
        let mut w = 0u32;
        for (i, &c) in r.iter().enumerate() {
            w |= u32::from(c) << (8 * i);
        }
        h ^= w.wrapping_mul(C1).rotate_left(15).wrapping_mul(C2);
    }
    h ^= k.len() as u32;
    h ^= h >> 16;
    h = h.wrapping_mul(0x85eb_ca6b);
    h ^= h >> 13;
    h = h.wrapping_mul(0xc2b2_ae35);
    h ^ (h >> 16)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Published check values.
    #[test]
    fn hashes_match_their_references() {
        assert_eq!(crc32(b"123456789"), 0xcbf4_3926);
        assert_eq!(crc32_msb(b"123456789"), 0xfc89_1918);
        assert_eq!(fnv1a(b"a"), 0xe40c_292c);
        assert_eq!(fnv1(b"a"), 0x050c_5d7e);
        assert_eq!(murmur3(b"hello"), 0x248b_fa47);
        assert_eq!(lookup3(b"Four score and seven years ago"), 0x1777_0551);
        assert_eq!(oat(b"a"), 0xca2e_9442);
    }
}
