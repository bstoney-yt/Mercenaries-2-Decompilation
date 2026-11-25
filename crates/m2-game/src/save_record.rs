//! The 360's save file: one save record, 0x3458 bytes, written as the
//! profile object holds it from its +0x18 (where [`RecordHeader`]
//! starts) and read back over it.
//!
//! # Provenance
//!
//! **From the 360 code** unless a line says otherwise. The profile object
//! is the one [`crate::profile`] describes (pointer `0x83134474`). Its
//! offsets below are the profile's; the file's are 0x18 less.
//!
//! - Layout: `0x8239B2F8` makes a new record (version
//!   [`profile::RECORD_VERSION`], size [`profile::RECORD_SIZE`], the
//!   owner's id, the play time, cash and fuel cleared, the time stamp, the
//!   0x20-byte contract name, the bytes +0x60..+0x67, the words
//!   +0x68/+0x6C, then the 0x400-byte options block and the 0x3000-byte
//!   save data block cleared). Saving writes 0x3458 bytes from +0x18
//!   (`0x8226F7A0`, from the save state machine `0x8239D398`, state 0xF);
//!   listing reads 0x3458 bytes of each file (state 5) and keeps those
//!   whose version, size and owner match, the rest put back to a new
//!   record (`0x8239DFD0`); loading copies the chosen file's record over
//!   the profile (`0x8239BE98`, state 0xC) and runs its save data
//!   (`0x8230C880`).
//! - Cash +0x2C and fuel +0x30: `Player.GetCash`/`SetCash`/`AddCash`
//!   (`0x8233F588`, `0x8233F5C0`, `0x8233F668`) and `Player.GetFuel`/
//!   `SetFuel` (`0x8233F728`, `0x8233F760`) read and write them there.
//! - Play time +0x28, contract +0x40, percent +0x65: `Sys.RequestAutosave
//!   (mission_on, contract, seconds, share)` (`0x82345D70`) keeps them in
//!   the script manager (+0xC44, +0xC45, +0xC68, +0xC6C; the share made a
//!   percent: [`percent_from_share`]); each save that is not the retry
//!   (`0x8230C4A8`) stamps them into the record with the time (+0x38,
//!   `time(0)`), packs the scripts' save text into the save block
//!   ([`profile::SaveDataHeader`], zlib 1.2.3 level 9: the program carries
//!   zlib's 1.2.3 banners and calls it as `compress2`/`uncompress`).
//! - The character +0x61 (`profileCharacter`), upgrade +0x62
//!   (`Player.Get/SetProfileUpgrade`, `0x8233FA20`), costume +0x63
//!   (`Player.Get/SetProfileCostume`, `0x8233FAE8`), autosave +0x64 (the
//!   save state's +0x5380, set when the scripts' autosave picks its slot:
//!   `0x8239ACB0`).
//! - The save list (`addSaveGame`, `0x8237E2F0`) shows each file's time
//!   stamp, character, cash, fuel, play time as `%d:%02d:%02d`
//!   ([`played_text`]), contract, autosave and percent: all from here.
//!
//! What the record does **not** hold: where she is, her health, her
//! weapons and ammunition, the vehicles, the wrecks or the dead. Loading
//! (`0x8230C880`) runs the save text (`return ` and it, chunk `LoadGame`),
//! then resets the world (`0x8230C740`: every player put out of her seat,
//! `0x822A1280`, and out of survival mode, `0x8243AED8`; the heli waves
//! stopped, `0x8223B390`), then hands the data to the bootstrap's
//! `LoadSingleton`. Whatever else comes back is the scripts' to put back.
//!
//! Unknown fields are named by offset. The owner's id is the console
//! profile's (`0x83134470`); the port has no console profile and writes
//! [`OWNER`] (**invented**).

use m2_layout::assert_layout;

use crate::profile::{self, SaveDataHeader};

/// The owner's id the port writes and expects (**invented**: the 360's
/// is the signed-in profile's).
pub const OWNER: u32 = 0;
/// The record's length in a file: from its version word to the end of
/// the save data block.
pub const FILE_LEN: usize = profile::RECORD_SIZE as usize;
/// The options block's length in the record.
pub const OPTIONS_LEN: usize = 0x400;
/// How long the contract name is (with its terminator).
pub const CONTRACT_LEN: usize = 0x20;

/// The record's fixed fields: the profile's +0x18 to +0x70 (offsets here
/// from +0x18). Numbers big-endian as stored.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RecordHeader {
    /// [`profile::RECORD_VERSION`].
    pub version: [u8; 4],
    /// [`profile::RECORD_SIZE`].
    pub size: [u8; 4],
    /// The owner's id ([`OWNER`]).
    pub owner: [u8; 4],
    /// +0x24 (left alone by a new record).
    pub field_0c: [u8; 4],
    /// Seconds played, whole (+0x28).
    pub played: [u8; 4],
    /// Her cash (+0x2C, unsigned).
    pub cash: [u8; 4],
    /// Her fuel (+0x30).
    pub fuel: [u8; 4],
    /// +0x34 (left alone by a new record).
    pub field_1c: [u8; 4],
    /// When it was saved: `time(0)` (+0x38).
    pub saved_at: [u8; 8],
    /// The last contract, NUL-terminated (+0x40).
    pub contract: [u8; CONTRACT_LEN],
    /// +0x60 (cleared by a new game, `0x8239C6D8`).
    pub field_48: u8,
    /// `profileCharacter` (+0x61): the save list's character, 1 Mattias,
    /// 2 Chris, 3 Jennifer (the list movie's order).
    pub character: u8,
    /// `Player.GetProfileUpgrade` (+0x62).
    pub upgrade: u8,
    /// `Player.GetProfileCostume` (+0x63).
    pub costume: u8,
    /// The scripts' autosave (+0x64).
    pub autosave: u8,
    /// Percent of the game done, 0 to 100 (+0x65).
    pub percent: u8,
    /// +0x66, +0x67.
    pub field_4e: [u8; 2],
    /// +0x68.
    pub field_50: [u8; 4],
    /// +0x6C.
    pub field_54: [u8; 4],
}

assert_layout!(RecordHeader, size = 0x58, {
    version: 0x00,
    size: 0x04,
    owner: 0x08,
    field_0c: 0x0C,
    played: 0x10,
    cash: 0x14,
    fuel: 0x18,
    field_1c: 0x1C,
    saved_at: 0x20,
    contract: 0x28,
    field_48: 0x48,
    character: 0x49,
    upgrade: 0x4A,
    costume: 0x4B,
    autosave: 0x4C,
    percent: 0x4D,
    field_4e: 0x4E,
    field_50: 0x50,
    field_54: 0x54,
});

/// A save file's record as stored.
#[repr(C)]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SaveRecord {
    pub header: RecordHeader,
    /// The options block (its first 0x260 bytes are
    /// [`profile::ProfileOptions`]).
    pub options: [u8; OPTIONS_LEN],
    /// The save data block: [`SaveDataHeader`] and the packed save text.
    pub save_data: [u8; profile::SAVE_BLOCK_LEN],
}

assert_layout!(SaveRecord, size = FILE_LEN, {
    header: 0x00,
    options: 0x58,
    save_data: 0x458,
});

/// Why a file is not a save record the game would keep.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RecordError {
    /// Shorter or longer than [`FILE_LEN`].
    Length(usize),
    /// Another version, size or owner (`0x8239DFD0` throws such a record
    /// away).
    Header { version: u32, size: u32, owner: u32 },
    /// The save data block is not kind 1, or does not unpack.
    SaveData(String),
    /// The save text is too long for the block (`0x8230C4A8` refuses the
    /// save).
    TooLong(usize),
}

impl std::fmt::Display for RecordError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Length(n) => write!(f, "{n} bytes, not a save record's {FILE_LEN}"),
            Self::Header {
                version,
                size,
                owner,
            } => write!(
                f,
                "record version {version}, size {size:#x}, owner {owner:#x} (this build reads version {}, size {:#x}, owner {OWNER:#x})",
                profile::RECORD_VERSION,
                profile::RECORD_SIZE
            ),
            Self::SaveData(e) => write!(f, "save data: {e}"),
            Self::TooLong(n) => write!(
                f,
                "save data packs to {n} bytes, more than the record's {} hold",
                profile::SAVE_BLOCK_LEN
            ),
        }
    }
}

impl std::error::Error for RecordError {}

/// The save list's play time (`0x8237E2F0`): hours, minutes, seconds as
/// `%d:%02d:%02d`.
pub fn played_text(seconds: u32) -> String {
    format!(
        "{}:{:02}:{:02}",
        seconds / 3600,
        seconds / 60 % 60,
        seconds % 60
    )
}

/// `Sys.RequestAutosave`'s share of the game done made the record's
/// percent (`0x82345D70`): at most 1, below 0 none, times 100 rounded,
/// at most 100.
pub fn percent_from_share(share: f32) -> u8 {
    let s = if share - 1.0 < 0.0 { share } else { 1.0 };
    let s = if s < 0.0 { 0.0 } else { s };
    ((f64::from(s) * 100.0 + 0.5) as u64).min(100) as u8
}

/// What `Sys.RequestAutosave` last told the engine (`0x82345D70`), which
/// each save stamps into its record.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AutosaveInfo {
    /// Its first argument: a mission on (+0xC44).
    pub mission_on: bool,
    /// The last contract, at most 0x1F characters (+0xC45).
    pub contract: String,
    /// Seconds played (+0xC68).
    pub played: f32,
    /// Percent done ([`percent_from_share`], +0xC6C).
    pub percent: u8,
}

impl AutosaveInfo {
    /// From `Sys.RequestAutosave`'s arguments, as `0x82345D70` reads them:
    /// missing ones are false, "", 0 and 0.
    pub fn from_args(
        mission_on: Option<bool>,
        contract: Option<&str>,
        played: Option<f32>,
        share: Option<f32>,
    ) -> Self {
        Self {
            mission_on: mission_on.unwrap_or(false),
            contract: contract
                .unwrap_or("")
                .chars()
                .take(CONTRACT_LEN - 1)
                .collect(),
            played: played.unwrap_or(0.0),
            percent: percent_from_share(share.unwrap_or(0.0)),
        }
    }
}

/// A record's fields in Rust's terms.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RecordInfo {
    pub played: u32,
    pub cash: u32,
    pub fuel: i32,
    pub saved_at: u64,
    pub contract: String,
    pub character: u8,
    pub upgrade: u8,
    pub costume: u8,
    pub autosave: bool,
    pub percent: u8,
}

impl Default for RecordHeader {
    /// A new record's (`0x8239B2F8`).
    fn default() -> Self {
        Self {
            version: profile::RECORD_VERSION.to_be_bytes(),
            size: profile::RECORD_SIZE.to_be_bytes(),
            owner: OWNER.to_be_bytes(),
            field_0c: [0; 4],
            played: [0; 4],
            cash: [0; 4],
            fuel: [0; 4],
            field_1c: [0; 4],
            saved_at: [0; 8],
            contract: [0; CONTRACT_LEN],
            field_48: 0,
            character: 0,
            upgrade: 0,
            costume: 0,
            autosave: 0,
            percent: 0,
            field_4e: [0; 2],
            field_50: [0; 4],
            field_54: [0; 4],
        }
    }
}

impl Default for SaveRecord {
    /// A new record (`0x8239B2F8`): the options at their defaults
    /// (`0x8239B028`), no save data.
    fn default() -> Self {
        let mut options = [0u8; OPTIONS_LEN];
        options[..profile::TITLE_SETTINGS_LEN]
            .copy_from_slice(&profile::ProfileOptions::default().to_bytes());
        Self {
            header: RecordHeader::default(),
            options,
            save_data: [0; profile::SAVE_BLOCK_LEN],
        }
    }
}

fn be32(b: [u8; 4]) -> u32 {
    u32::from_be_bytes(b)
}

impl SaveRecord {
    /// A record of `info` with the scripts' save text `text` packed in.
    pub fn new(info: &RecordInfo, text: &str) -> Result<Box<Self>, RecordError> {
        let mut r = Box::<Self>::default();
        r.set_info(info);
        r.pack(text)?;
        Ok(r)
    }

    pub fn info(&self) -> RecordInfo {
        let h = &self.header;
        let end = h
            .contract
            .iter()
            .position(|&c| c == 0)
            .unwrap_or(CONTRACT_LEN);
        RecordInfo {
            played: be32(h.played),
            cash: be32(h.cash),
            fuel: be32(h.fuel) as i32,
            saved_at: u64::from_be_bytes(h.saved_at),
            contract: String::from_utf8_lossy(&h.contract[..end]).into_owned(),
            character: h.character,
            upgrade: h.upgrade,
            costume: h.costume,
            autosave: h.autosave != 0,
            percent: h.percent,
        }
    }

    pub fn set_info(&mut self, i: &RecordInfo) {
        let h = &mut self.header;
        h.played = i.played.to_be_bytes();
        h.cash = i.cash.to_be_bytes();
        h.fuel = (i.fuel as u32).to_be_bytes();
        h.saved_at = i.saved_at.to_be_bytes();
        h.contract = [0; CONTRACT_LEN];
        // At most 0x1F characters, the last byte the terminator (+0x5F).
        let n = i.contract.len().min(CONTRACT_LEN - 1);
        h.contract[..n].copy_from_slice(&i.contract.as_bytes()[..n]);
        h.character = i.character;
        h.upgrade = i.upgrade;
        h.costume = i.costume;
        h.autosave = u8::from(i.autosave);
        h.percent = i.percent.min(100);
    }

    /// Packs the scripts' save text into the save data block as
    /// `0x8230C4A8` does: zlib level 9 behind a kind-1 header, refused when
    /// it does not fit ([`profile::fits_in_record`]).
    pub fn pack(&mut self, text: &str) -> Result<(), RecordError> {
        if text.len() > profile::SAVE_DATA_CAPACITY {
            return Err(RecordError::TooLong(text.len()));
        }
        let packed = miniz_oxide::deflate::compress_to_vec_zlib(text.as_bytes(), 9);
        if !profile::fits_in_record(packed.len()) {
            return Err(RecordError::TooLong(packed.len()));
        }
        self.save_data = [0; profile::SAVE_BLOCK_LEN];
        let head = SaveDataHeader::new(packed.len() as u32, text.len() as u32);
        self.save_data[0..4].copy_from_slice(&head.kind);
        self.save_data[4..8].copy_from_slice(&head.packed_len);
        self.save_data[8..12].copy_from_slice(&head.text_len);
        self.save_data[12..12 + packed.len()].copy_from_slice(&packed);
        Ok(())
    }

    /// The scripts' save text, unpacked as `0x8230C880` does: `None` for an
    /// empty block (kind 0, a record never saved over).
    pub fn script_text(&self) -> Result<Option<String>, RecordError> {
        let word = |i: usize| be32(self.save_data[i..i + 4].try_into().expect("4 bytes"));
        let (kind, packed, len) = (word(0), word(4) as usize, word(8) as usize);
        if kind == 0 && packed == 0 && len == 0 {
            return Ok(None);
        }
        if kind != SaveDataHeader::KIND {
            return Err(RecordError::SaveData(format!("kind {kind}")));
        }
        let data = self
            .save_data
            .get(12..12 + packed)
            .ok_or_else(|| RecordError::SaveData(format!("{packed} packed bytes")))?;
        let text = miniz_oxide::inflate::decompress_to_vec_zlib_with_limit(
            data,
            profile::SAVE_DATA_CAPACITY,
        )
        .map_err(|e| RecordError::SaveData(format!("{e:?}")))?;
        if text.len() != len {
            return Err(RecordError::SaveData(format!(
                "{} bytes unpacked, the header says {len}",
                text.len()
            )));
        }
        String::from_utf8(text)
            .map(Some)
            .map_err(|e| RecordError::SaveData(e.to_string()))
    }

    /// The options block's first part.
    pub fn options_bytes(&self) -> &[u8] {
        &self.options[..profile::TITLE_SETTINGS_LEN]
    }

    /// The file's bytes.
    pub fn to_bytes(&self) -> Vec<u8> {
        let h = &self.header;
        let mut b = Vec::with_capacity(FILE_LEN);
        for s in [
            &h.version[..],
            &h.size,
            &h.owner,
            &h.field_0c,
            &h.played,
            &h.cash,
            &h.fuel,
            &h.field_1c,
            &h.saved_at,
            &h.contract,
            &[
                h.field_48,
                h.character,
                h.upgrade,
                h.costume,
                h.autosave,
                h.percent,
            ],
            &h.field_4e,
            &h.field_50,
            &h.field_54,
            &self.options,
            &self.save_data,
        ] {
            b.extend_from_slice(s);
        }
        debug_assert_eq!(b.len(), FILE_LEN);
        b
    }

    /// A file's record, kept only as `0x8239DFD0` keeps one: the length,
    /// version, size and owner the game's.
    pub fn from_bytes(b: &[u8]) -> Result<Box<Self>, RecordError> {
        if b.len() != FILE_LEN {
            return Err(RecordError::Length(b.len()));
        }
        let mut at = 0;
        let mut take = |n: usize| {
            let s = &b[at..at + n];
            at += n;
            s
        };
        let mut four = || -> [u8; 4] { take(4).try_into().expect("4 bytes") };
        let (version, size, owner, field_0c, played, cash, fuel, field_1c) = (
            four(),
            four(),
            four(),
            four(),
            four(),
            four(),
            four(),
            four(),
        );
        if (be32(version), be32(size), be32(owner))
            != (profile::RECORD_VERSION, profile::RECORD_SIZE, OWNER)
        {
            return Err(RecordError::Header {
                version: be32(version),
                size: be32(size),
                owner: be32(owner),
            });
        }
        let saved_at = take(8).try_into().expect("8 bytes");
        let contract = take(CONTRACT_LEN).try_into().expect("0x20 bytes");
        let bytes = take(6);
        let field_4e = take(2).try_into().expect("2 bytes");
        let field_50 = take(4).try_into().expect("4 bytes");
        let field_54 = take(4).try_into().expect("4 bytes");
        let options = take(OPTIONS_LEN).try_into().expect("0x400 bytes");
        let save_data = take(profile::SAVE_BLOCK_LEN)
            .try_into()
            .expect("0x3000 bytes");
        Ok(Box::new(Self {
            header: RecordHeader {
                version,
                size,
                owner,
                field_0c,
                played,
                cash,
                fuel,
                field_1c,
                saved_at,
                contract,
                field_48: bytes[0],
                character: bytes[1],
                upgrade: bytes[2],
                costume: bytes[3],
                autosave: bytes[4],
                percent: bytes[5],
                field_4e,
                field_50,
                field_54,
            },
            options,
            save_data,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info() -> RecordInfo {
        RecordInfo {
            played: 3 * 3600 + 7 * 60 + 9,
            cash: 1_250_000,
            fuel: 240,
            saved_at: 1_795_000_000,
            contract: "VzaCon001".into(),
            character: 3,
            upgrade: 1,
            costume: 2,
            autosave: true,
            percent: 12,
        }
    }

    #[test]
    fn a_record_round_trips_byte_for_byte() {
        let text = "{[\"nCash\"]=25000,[\"tList\"]={1,2,3,},}".repeat(40);
        let r = SaveRecord::new(&info(), &text).unwrap();
        let b = r.to_bytes();
        assert_eq!(b.len(), 0x3458);
        let back = SaveRecord::from_bytes(&b).unwrap();
        assert_eq!(back, r);
        assert_eq!(back.info(), info());
        assert_eq!(back.script_text().unwrap().as_deref(), Some(text.as_str()));
        assert_eq!(back.to_bytes(), b);
    }

    /// Fields sit where the 360's profile has them, less 0x18.
    #[test]
    fn fields_are_at_the_360s_offsets() {
        let r = SaveRecord::new(&info(), "{}").unwrap();
        let b = r.to_bytes();
        let at = |profile_off: usize| profile_off - 0x18;
        assert_eq!(&b[at(0x18)..at(0x1C)], &4u32.to_be_bytes());
        assert_eq!(&b[at(0x1C)..at(0x20)], &0x3458u32.to_be_bytes());
        assert_eq!(&b[at(0x28)..at(0x2C)], &11_229u32.to_be_bytes());
        assert_eq!(&b[at(0x2C)..at(0x30)], &1_250_000u32.to_be_bytes());
        assert_eq!(&b[at(0x30)..at(0x34)], &240u32.to_be_bytes());
        assert_eq!(&b[at(0x38)..at(0x40)], &1_795_000_000u64.to_be_bytes());
        assert_eq!(&b[at(0x40)..at(0x49)], b"VzaCon001");
        assert_eq!(b[at(0x49)], 0);
        assert_eq!(
            (
                b[at(0x61)],
                b[at(0x62)],
                b[at(0x63)],
                b[at(0x64)],
                b[at(0x65)]
            ),
            (3, 1, 2, 1, 12)
        );
        // The options block's version word and defaults at +0x70.
        assert_eq!(&b[at(0x70)..at(0x74)], &4u32.to_be_bytes());
        assert_eq!(b[at(0x70) + 0x10], profile::DEFAULT_SENSITIVITY);
        // The save data's kind-1 header at +0x470, zlib behind it.
        assert_eq!(&b[at(0x470)..at(0x474)], &1u32.to_be_bytes());
        assert_eq!(&b[at(0x478)..at(0x47C)], &2u32.to_be_bytes());
        assert_eq!(b[at(0x47C)], 0x78);
    }

    #[test]
    fn records_the_game_would_throw_away_are_refused() {
        let mut b = SaveRecord::new(&info(), "{}").unwrap().to_bytes();
        assert!(matches!(
            SaveRecord::from_bytes(&b[1..]),
            Err(RecordError::Length(_))
        ));
        b[3] = 3;
        assert!(matches!(
            SaveRecord::from_bytes(&b),
            Err(RecordError::Header { version: 3, .. })
        ));
        b[3] = 4;
        b[8 + 3] = 9;
        assert!(matches!(
            SaveRecord::from_bytes(&b),
            Err(RecordError::Header { owner: 9, .. })
        ));
    }

    #[test]
    fn a_new_record_has_no_save_data_and_the_default_options() {
        let r = SaveRecord::default();
        assert_eq!(r.script_text(), Ok(None));
        assert_eq!(
            r.options_bytes(),
            &profile::ProfileOptions::default().to_bytes()[..]
        );
        let mut r = r;
        r.save_data[3] = 2;
        assert!(r.script_text().is_err());
    }

    #[test]
    fn text_too_long_for_the_block_is_refused() {
        // Random-looking text does not pack below 0x3000 bytes.
        let mut x: u32 = 1;
        let text: String = (0..0x8000)
            .map(|_| {
                x = x.wrapping_mul(1_103_515_245).wrapping_add(12_345);
                char::from(b'!' + (x >> 16) as u8 % 90)
            })
            .collect();
        let mut r = SaveRecord::default();
        assert!(matches!(r.pack(&text), Err(RecordError::TooLong(_))));
        assert!(matches!(
            r.pack(&"x".repeat(profile::SAVE_DATA_CAPACITY + 1)),
            Err(RecordError::TooLong(_))
        ));
    }

    #[test]
    fn the_contract_keeps_31_characters() {
        let mut i = info();
        i.contract = "C".repeat(40);
        let mut r = SaveRecord::default();
        r.set_info(&i);
        assert_eq!(r.info().contract.len(), 31);
        assert_eq!(r.header.contract[31], 0);
    }

    #[test]
    fn the_list_shows_play_time_and_percent_the_360s_way() {
        assert_eq!(played_text(0), "0:00:00");
        assert_eq!(played_text(11_229), "3:07:09");
        assert_eq!(played_text(360_000), "100:00:00");
        assert_eq!(percent_from_share(0.0), 0);
        assert_eq!(percent_from_share(0.124), 12);
        assert_eq!(percent_from_share(0.125), 13);
        assert_eq!(percent_from_share(1.0), 100);
        assert_eq!(percent_from_share(7.0), 100);
        assert_eq!(percent_from_share(-0.5), 0);
        let a = AutosaveInfo::from_args(Some(true), Some("VzaCon001"), Some(600.0), Some(0.02));
        assert_eq!(
            a,
            AutosaveInfo {
                mission_on: true,
                contract: "VzaCon001".into(),
                played: 600.0,
                percent: 2,
            }
        );
        assert_eq!(
            AutosaveInfo::from_args(None, None, None, None),
            AutosaveInfo::default()
        );
    }
}
