//! What the Stats page shows of one save: what its 360 record says (play
//! time, percent done, cash, fuel), and what the scripts' save data packed
//! into the record says (the contracts finished, each faction's standing
//! with the player's PMC).
//!
//! # Provenance
//!
//! **From the 360 code** (the record, laid out in `m2_game::save_record`):
//! seconds played +0x10, cash +0x14, fuel +0x18, percent +0x4D; the save
//! data block at +0x458 (0x3000 bytes): a kind-1 header (kind, packed
//! length, text length, big-endian) then the scripts' save text, zlib
//! (`0x8230C4A8` packs it, `0x8230C880` unpacks it).
//!
//! The record's play time is what the scripts last told the engine:
//! `MrxPlayState.GetTotalTimeElapsed()` as the pause screen opened
//! (`_GuiInternal.SetFlashPauseMenu`, `0x82322D50`) or as their autosave
//! asked (`Sys.RequestAutosave`, `0x82345D70`); the engine keeps no clock
//! of its own. A save written without either (the port's quick save and
//! `--save-to`, which the 360 does not have) carries the last stamp, 0
//! if neither ran yet: that is the 0:00:00 seen after 20 s of play.
//!
//! **From the 360 scripts** (their bytecode in the install's `vz.wad`):
//!
//! - The save text is `GenerateSaveData`'s table (script `b4420059`):
//!   `nTimeElapsed` is `MrxPlayState.GetTotalTimeElapsed()` at the save
//!   itself (script `bef35c31`: the seconds of prior sessions plus this
//!   one's, rounded up), so when the record says 0 the page shows this.
//! - Contracts finished: `WifMissionData.GetNumCompletedContracts`
//!   (script `a998325d`) counts the missions `WifMissionFlow.
//!   GetMissionStates` calls `"complete"` (script `01a266d2`: those with
//!   a key in `tFlowData.tMyFlowData`) that are contracts (characters 4-6
//!   of the id `"Con"`, `MrxUtil.ExplodeMissionName`, script `a8395434`).
//!   The launcher has no mission table, so it counts the keys shaped as
//!   contract ids are (three letters, `Con`, three digits: every contract
//!   id in `WifMissionData` is; **from samples**).
//! - Standing: `MrxFactionManager.GetAttitudeLabel(faction, "Pmc")`
//!   (script `b2cec5f5`; the starters ask it so) reads the relation of
//!   the faction to the PMC, -100 to 100, saved as
//!   `tFactionData.tRelations[faction].Pmc`; Hostile below -33, Neutral
//!   to 33, Friendly from 33 (`_tAttitudes`, with its colours).
//!
//! **Invented**: the factions' names as the page writes them and their
//! order.

use std::collections::BTreeMap;

use crate::saves::{RECORD_SIZE, RECORD_VERSION};

/// Where the save data block starts in a record's file.
const SAVE_BLOCK: usize = 0x458;
const SAVE_BLOCK_LEN: usize = 0x3000;
/// The scripts' save text is at most this long (`profile::SAVE_DATA_CAPACITY`).
const TEXT_LIMIT: usize = 0x18000;

/// A value of the scripts' save text: the subset of Lua's table
/// constructor their serialiser writes.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Bool(bool),
    Number(f64),
    Str(String),
    Table(BTreeMap<String, Value>),
    /// A native's call the serialiser writes for a value it cannot write
    /// as a literal, such as `Sys.StringToGuid("0x...")` for a guid.
    Call(String, Vec<Value>),
}

impl Value {
    pub fn get(&self, key: &str) -> Option<&Value> {
        match self {
            Value::Table(t) => t.get(key),
            _ => None,
        }
    }

    pub fn number(&self) -> Option<f64> {
        match self {
            Value::Number(n) => Some(*n),
            _ => None,
        }
    }

    pub fn table(&self) -> Option<&BTreeMap<String, Value>> {
        match self {
            Value::Table(t) => Some(t),
            _ => None,
        }
    }
}

/// Reads a Lua table constructor such as `{["a"]=1,[2]="x",["t"]={},}`
/// (numeric keys are kept as their text). A leading `return` is allowed.
pub fn parse(text: &str) -> Result<Value, String> {
    let mut p = Parser {
        s: text.as_bytes(),
        i: 0,
    };
    p.space();
    if p.s[p.i..].starts_with(b"return") {
        p.i += 6;
    }
    let v = p.value()?;
    p.space();
    if p.i != p.s.len() {
        return Err(format!("text after the table at {}", p.i));
    }
    Ok(v)
}

struct Parser<'a> {
    s: &'a [u8],
    i: usize,
}

impl Parser<'_> {
    fn space(&mut self) {
        while self.i < self.s.len() && self.s[self.i].is_ascii_whitespace() {
            self.i += 1;
        }
    }

    fn peek(&self) -> Option<u8> {
        self.s.get(self.i).copied()
    }

    fn expect(&mut self, c: u8) -> Result<(), String> {
        self.space();
        if self.peek() == Some(c) {
            self.i += 1;
            Ok(())
        } else {
            Err(format!("expected '{}' at {}", c as char, self.i))
        }
    }

    fn value(&mut self) -> Result<Value, String> {
        self.space();
        match self.peek() {
            Some(b'{') => self.table(),
            Some(b'"' | b'\'') => self.string().map(Value::Str),
            Some(b't') if self.s[self.i..].starts_with(b"true") => {
                self.i += 4;
                Ok(Value::Bool(true))
            }
            Some(b'f') if self.s[self.i..].starts_with(b"false") => {
                self.i += 5;
                Ok(Value::Bool(false))
            }
            Some(c) if c == b'_' || c.is_ascii_alphabetic() => self.call(),
            Some(_) => self.number().map(Value::Number),
            None => Err("the text ends early".to_owned()),
        }
    }

    fn table(&mut self) -> Result<Value, String> {
        self.expect(b'{')?;
        let mut out = BTreeMap::new();
        let mut next = 1u32;
        loop {
            self.space();
            match self.peek() {
                Some(b'}') => {
                    self.i += 1;
                    return Ok(Value::Table(out));
                }
                Some(b'[') => {
                    self.i += 1;
                    self.space();
                    let key = match self.peek() {
                        Some(b'"' | b'\'') => self.string()?,
                        _ => number_key(self.number()?),
                    };
                    self.expect(b']')?;
                    self.expect(b'=')?;
                    let v = self.value()?;
                    out.insert(key, v);
                }
                Some(c) if c == b'_' || c.is_ascii_alphabetic() => {
                    // `name = value`, unless it is a bare value.
                    let start = self.i;
                    while self
                        .peek()
                        .is_some_and(|c| c == b'_' || c.is_ascii_alphanumeric())
                    {
                        self.i += 1;
                    }
                    let name = String::from_utf8_lossy(&self.s[start..self.i]).into_owned();
                    self.space();
                    if self.peek() == Some(b'=') {
                        self.i += 1;
                        let v = self.value()?;
                        out.insert(name, v);
                    } else {
                        self.i = start;
                        let v = self.value()?;
                        out.insert(next.to_string(), v);
                        next += 1;
                    }
                }
                Some(_) => {
                    let v = self.value()?;
                    out.insert(next.to_string(), v);
                    next += 1;
                }
                None => return Err("a table is not closed".to_owned()),
            }
            self.space();
            match self.peek() {
                Some(b',' | b';') => self.i += 1,
                Some(b'}') => {}
                _ => return Err(format!("expected ',' or '}}' at {}", self.i)),
            }
        }
    }

    /// `Name.Name(value, ...)`.
    fn call(&mut self) -> Result<Value, String> {
        let start = self.i;
        while self
            .peek()
            .is_some_and(|c| c == b'_' || c == b'.' || c.is_ascii_alphanumeric())
        {
            self.i += 1;
        }
        let name = String::from_utf8_lossy(&self.s[start..self.i]).into_owned();
        self.expect(b'(')
            .map_err(|_| format!("not a value at {start}: {name:?}"))?;
        let mut args = Vec::new();
        self.space();
        if self.peek() == Some(b')') {
            self.i += 1;
            return Ok(Value::Call(name, args));
        }
        loop {
            args.push(self.value()?);
            self.space();
            match self.peek() {
                Some(b',') => self.i += 1,
                Some(b')') => {
                    self.i += 1;
                    return Ok(Value::Call(name, args));
                }
                _ => return Err(format!("expected ',' or ')' at {}", self.i)),
            }
        }
    }

    fn string(&mut self) -> Result<String, String> {
        let quote = self.s[self.i];
        self.i += 1;
        let mut out = Vec::new();
        loop {
            let c = self.peek().ok_or("a string is not closed")?;
            self.i += 1;
            match c {
                c if c == quote => break,
                b'\\' => {
                    let e = self.peek().ok_or("a string is not closed")?;
                    self.i += 1;
                    match e {
                        b'n' => out.push(b'\n'),
                        b't' => out.push(b'\t'),
                        b'r' => out.push(b'\r'),
                        b'a' => out.push(7),
                        b'b' => out.push(8),
                        b'f' => out.push(12),
                        b'v' => out.push(11),
                        b'0'..=b'9' => {
                            let mut n = u32::from(e - b'0');
                            for _ in 0..2 {
                                match self.peek() {
                                    Some(d @ b'0'..=b'9') => {
                                        n = n * 10 + u32::from(d - b'0');
                                        self.i += 1;
                                    }
                                    _ => break,
                                }
                            }
                            out.push(u8::try_from(n).map_err(|_| "a bad escape")?);
                        }
                        other => out.push(other),
                    }
                }
                c => out.push(c),
            }
        }
        Ok(String::from_utf8_lossy(&out).into_owned())
    }

    fn number(&mut self) -> Result<f64, String> {
        let start = self.i;
        while self
            .peek()
            .is_some_and(|c| c.is_ascii_alphanumeric() || matches!(c, b'.' | b'-' | b'+'))
        {
            self.i += 1;
        }
        let t = std::str::from_utf8(&self.s[start..self.i]).unwrap_or("");
        let n = if let Some(h) = t.strip_prefix("0x").or_else(|| t.strip_prefix("0X")) {
            u64::from_str_radix(h, 16).map(|v| v as f64).ok()
        } else {
            t.parse::<f64>().ok()
        };
        n.ok_or_else(|| format!("not a value at {start}: {t:?}"))
    }
}

/// A numeric key as its text: `1`, not `1.0`.
fn number_key(n: f64) -> String {
    if n.fract() == 0.0 && n.abs() < 1e15 {
        format!("{}", n as i64)
    } else {
        n.to_string()
    }
}

/// The factions the page lists, by the scripts' abbreviations, with the
/// names it writes (**invented**).
pub const FACTIONS: [(&str, &str); 5] = [
    ("All", "Allied Nations"),
    ("Chi", "China"),
    ("Oil", "Universal Petroleum"),
    ("Gur", "UPLA"),
    ("Pir", "Pirates"),
];

/// `_tAttitudes` (script `b2cec5f5`): the attitude a relation reads as.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Attitude {
    Hostile,
    Neutral,
    Friendly,
}

impl Attitude {
    /// Hostile `[-100, -33)`, Neutral `[-33, 33)`, Friendly `[33, 100]`.
    pub fn of(relation: f64) -> Self {
        if relation < -33.0 {
            Attitude::Hostile
        } else if relation < 33.0 {
            Attitude::Neutral
        } else {
            Attitude::Friendly
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Attitude::Hostile => "Hostile",
            Attitude::Neutral => "Neutral",
            Attitude::Friendly => "Friendly",
        }
    }

    /// Its `tRgbColor`.
    pub fn rgb(self) -> [u8; 3] {
        match self {
            Attitude::Hostile => [255, 0, 0],
            Attitude::Neutral => [200, 200, 200],
            Attitude::Friendly => [0, 127, 255],
        }
    }
}

/// One save's stats.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SaveStats {
    /// Seconds played: the record's, else the scripts' (see
    /// [`PlayedFrom`]).
    pub played: Option<u32>,
    pub played_from: PlayedFrom,
    pub percent: Option<u8>,
    pub cash: u32,
    pub fuel: i32,
    /// Contracts finished, if the save text could be read.
    pub contracts: Option<u32>,
    /// Each of [`FACTIONS`]' relation to the PMC, where the save has it.
    pub standing: Vec<(&'static str, f64)>,
    /// Why the scripts' save data could not be read, if it could not.
    pub problem: Option<String>,
}

/// Where the play time came from.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PlayedFrom {
    #[default]
    Record,
    /// The record said 0 and the scripts' save text said more.
    Scripts,
}

fn be32(b: &[u8], at: usize) -> u32 {
    u32::from_be_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

/// The scripts' save text in a record's bytes: `Ok(None)` for an empty
/// block.
pub fn script_text(record: &[u8]) -> Result<Option<String>, String> {
    let block = record
        .get(SAVE_BLOCK..SAVE_BLOCK + SAVE_BLOCK_LEN)
        .ok_or("the record is cut short")?;
    let (kind, packed, len) = (
        be32(block, 0),
        be32(block, 4) as usize,
        be32(block, 8) as usize,
    );
    if (kind, packed, len) == (0, 0, 0) {
        return Ok(None);
    }
    if kind != 1 {
        return Err(format!("save data of kind {kind}"));
    }
    let data = block
        .get(12..12 + packed)
        .ok_or_else(|| format!("{packed} packed bytes"))?;
    let text = miniz_oxide::inflate::decompress_to_vec_zlib_with_limit(data, TEXT_LIMIT)
        .map_err(|e| format!("the save data does not unpack: {e:?}"))?;
    if text.len() != len {
        return Err(format!(
            "{} bytes unpacked, the header says {len}",
            text.len()
        ));
    }
    String::from_utf8(text).map(Some).map_err(|e| e.to_string())
}

/// A contract's id: three letters, `Con`, three digits.
pub fn is_contract_id(id: &str) -> bool {
    let b = id.as_bytes();
    b.len() == 9
        && b[..3].iter().all(u8::is_ascii_alphabetic)
        && &b[3..6] == b"Con"
        && b[6..].iter().all(u8::is_ascii_digit)
}

/// What the scripts' save text says, into `out`.
fn read_text(text: &str, out: &mut SaveStats) -> Result<(), String> {
    let save = parse(text)?;
    if out.played == Some(0)
        && let Some(t) = save.get("nTimeElapsed").and_then(Value::number)
        && t > 0.0
    {
        out.played = Some(t.ceil() as u32);
        out.played_from = PlayedFrom::Scripts;
    }
    out.contracts = Some(
        save.get("tFlowData")
            .and_then(|f| f.get("tMyFlowData"))
            .and_then(Value::table)
            .map_or(0, |t| t.keys().filter(|k| is_contract_id(k)).count() as u32),
    );
    let relations = save.get("tFactionData").and_then(|f| f.get("tRelations"));
    out.standing = FACTIONS
        .iter()
        .filter_map(|&(f, _)| {
            relations
                .and_then(|r| r.get(f))
                .and_then(|r| r.get("Pmc"))
                .and_then(Value::number)
                .map(|n| (f, n))
        })
        .collect();
    Ok(())
}

/// A record's stats (its bytes as the file holds them).
pub fn from_record(bytes: &[u8]) -> Result<SaveStats, String> {
    if bytes.len() != RECORD_SIZE as usize
        || be32(bytes, 0) != RECORD_VERSION
        || be32(bytes, 4) != RECORD_SIZE
    {
        return Err("not a save record this launcher reads".to_owned());
    }
    let mut out = SaveStats {
        played: Some(be32(bytes, 0x10)),
        percent: Some(bytes[0x4D].min(100)),
        cash: be32(bytes, 0x14),
        fuel: be32(bytes, 0x18) as i32,
        ..SaveStats::default()
    };
    match script_text(bytes) {
        Ok(Some(text)) => {
            if let Err(e) = read_text(&text, &mut out) {
                out.problem = Some(format!("the scripts' save data: {e}"));
            }
        }
        Ok(None) => out.problem = Some("no scripts' save data in it".to_owned()),
        Err(e) => out.problem = Some(e),
    }
    Ok(out)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A save text shaped as the scripts write theirs.
    pub const TEXT: &str = r#"{["vEquippedSupport"]={},["nTimeElapsed"]=48,["tFlowData"]={["tCulledBindings"]={[1]="Start",},["tActiveMissions"]={["VzaCon001"]={["nState"]=1,["tContractState"]={},},},["tMyFlowData"]={["VzaCon001"]=true,["PmcCon002"]=true,["GurJob012"]=true,["sNote"]="a \"quoted\" \\ line\n",},},["tFactionData"]={["tRelations"]={["Oil"]={["Pmc"]=0,["All"]=100,},["Chi"]={["Pmc"]=-50.5,},["Pmc"]={["Oil"]=-1,["Vza"]=-50,},["All"]={["Pmc"]=40,},["Gur"]={["Pmc"]=-34,},},["tMutableFactions"]={},},["bGrappleEnabled"]=false,["tPlayerData"]={[1]={["tEquipment"]={[1]={[1]=Sys.StringToGuid("0x000000003000009E"),[2]=9,},},},},}"#;

    /// A record with `text` packed in, `played` seconds, 37 %.
    pub fn record(played: u32, text: Option<&str>) -> Vec<u8> {
        let mut b = vec![0u8; RECORD_SIZE as usize];
        b[0..4].copy_from_slice(&RECORD_VERSION.to_be_bytes());
        b[4..8].copy_from_slice(&RECORD_SIZE.to_be_bytes());
        b[0x10..0x14].copy_from_slice(&played.to_be_bytes());
        b[0x14..0x18].copy_from_slice(&250_000u32.to_be_bytes());
        b[0x18..0x1C].copy_from_slice(&(-5i32).to_be_bytes());
        b[0x4D] = 37;
        if let Some(t) = text {
            let packed = miniz_oxide::deflate::compress_to_vec_zlib(t.as_bytes(), 9);
            let at = SAVE_BLOCK;
            b[at..at + 4].copy_from_slice(&1u32.to_be_bytes());
            b[at + 4..at + 8].copy_from_slice(&(packed.len() as u32).to_be_bytes());
            b[at + 8..at + 12].copy_from_slice(&(t.len() as u32).to_be_bytes());
            b[at + 12..at + 12 + packed.len()].copy_from_slice(&packed);
        }
        b
    }

    #[test]
    fn the_scripts_table_constructor_is_read() {
        let v = parse(TEXT).unwrap();
        let flow = v.get("tFlowData").unwrap();
        assert_eq!(
            flow.get("tCulledBindings").unwrap().get("1"),
            Some(&Value::Str("Start".into()))
        );
        assert_eq!(
            flow.get("tMyFlowData").unwrap().get("sNote"),
            Some(&Value::Str("a \"quoted\" \\ line\n".into()))
        );
        assert_eq!(v.get("bGrappleEnabled"), Some(&Value::Bool(false)));
        // A guid is written as the native call that makes it.
        let gun = v
            .get("tPlayerData")
            .and_then(|t| t.get("1"))
            .and_then(|t| t.get("tEquipment"));
        assert_eq!(
            gun.and_then(|t| t.get("1")).and_then(|t| t.get("1")),
            Some(&Value::Call(
                "Sys.StringToGuid".into(),
                vec![Value::Str("0x000000003000009E".into())]
            ))
        );
        assert!(parse("{Sys.Bad(1}").is_err());
        let v = parse("return { 1, 'two', x = -1.5e2, [3.5] = true }").unwrap();
        assert_eq!(v.get("1"), Some(&Value::Number(1.0)));
        assert_eq!(v.get("2"), Some(&Value::Str("two".into())));
        assert_eq!(v.get("x"), Some(&Value::Number(-150.0)));
        assert_eq!(v.get("3.5"), Some(&Value::Bool(true)));
        assert!(parse("{[\"a\"]=1").is_err());
        assert!(parse("{} trailing").is_err());
        assert!(parse("{[\"a\"]=nope}").is_err());
    }

    #[test]
    fn contracts_are_the_finished_keys_shaped_as_contract_ids() {
        assert!(is_contract_id("PmcCon002"));
        assert!(!is_contract_id("GurJob012"));
        assert!(!is_contract_id("PmcCon002_x"));
        assert!(!is_contract_id("sNote"));
    }

    #[test]
    fn a_records_stats_come_from_it_and_its_save_data() {
        let s = from_record(&record(11_229, Some(TEXT))).unwrap();
        assert_eq!(s.problem, None);
        assert_eq!(
            (s.played, s.played_from, s.percent, s.cash, s.fuel),
            (Some(11_229), PlayedFrom::Record, Some(37), 250_000, -5)
        );
        assert_eq!(s.contracts, Some(2));
        // Each faction's relation to the PMC, in the page's order; one the
        // save lacks (Pir) is left out.
        assert_eq!(
            s.standing,
            [("All", 40.0), ("Chi", -50.5), ("Oil", 0.0), ("Gur", -34.0)]
        );
        // A record stamped 0 shows the scripts' own clock.
        let s = from_record(&record(0, Some(TEXT))).unwrap();
        assert_eq!((s.played, s.played_from), (Some(48), PlayedFrom::Scripts));
        // No save data: the record's fields only, and why.
        let s = from_record(&record(0, None)).unwrap();
        assert_eq!((s.played, s.contracts), (Some(0), None));
        assert!(s.problem.is_some());
        // Bad save text says so but keeps the record's fields.
        let s = from_record(&record(5, Some("{oops"))).unwrap();
        assert_eq!((s.played, s.cash), (Some(5), 250_000));
        assert!(s.problem.unwrap().contains("scripts"));
        assert!(from_record(&[0u8; 16]).is_err());
    }

    #[test]
    fn standing_reads_as_the_scripts_attitudes() {
        assert_eq!(Attitude::of(-100.0), Attitude::Hostile);
        assert_eq!(Attitude::of(-33.5), Attitude::Hostile);
        assert_eq!(Attitude::of(-33.0), Attitude::Neutral);
        assert_eq!(Attitude::of(32.9), Attitude::Neutral);
        assert_eq!(Attitude::of(33.0), Attitude::Friendly);
        assert_eq!(Attitude::of(100.0).label(), "Friendly");
    }
}
