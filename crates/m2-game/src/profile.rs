//! The player profile's options block: the game's per-player settings
//! (look sensitivity, inverted Y, rumble, autosave, tutorials, subtitles,
//! the volumes...), the loading-hint counter, the fuel capacity and the
//! character and costume choices, with their defaults.
//!
//! **From the 360 code.** The game keeps one profile object (pointer
//! `0x83134474`, static at `0x8348F5A8`). Its save record runs from +0x10
//! for 0x3460 bytes: flags, then a header at +0x18 (`0x8239B2F8`: version
//! [`RECORD_VERSION`], size [`RECORD_SIZE`], the owner's id), the character
//! and cash fields, this options block at +0x70 (0x400 bytes, defaults
//! `0x8239B028`) and a 0x3000-byte block at +0x470 (cleared; its contents
//! not traced). A loaded record whose version, size or owner differ is
//! thrown away for the defaults (`0x8239DFD0`). The record goes to a save
//! file (`0x8226F7A0` from `0x8239D398`); the first [`TITLE_SETTINGS_LEN`]
//! bytes of the options block also go to the console's per-title settings,
//! with a CRC ([`settings_crc`]) to see whether they changed
//! (`0x8239C850`, `0x8239CCF8`).
//!
//! [`ProfileOptions`] is that first part, as the 360 lays it out (offsets
//! from +0x70; numbers big-endian as stored). Its fields' meanings come
//! from the code that reads or writes them:
//!
//! - The pause menu's options go through one setter keyed by the option's
//!   name hash (`0x8239B3C0`): `gameInvert` +0x0D, `gameAutosave` +0x0F,
//!   `gameSensitivity` +0x10, `gameRumble` +0x11, `gameTutorials` +0x12,
//!   `videoSubtitles` +0x14, `profileCharacter` (the record's +0x61). Each
//!   marks the record dirty and has the options applied (`0x8239E338`).
//! - Applying: the inverted Y and the sensitivity (times 0.01) go to the
//!   pad's settings, +0x13 (times 0.01) to a float of the renderer's, the
//!   three bytes at +0x16 (times 0.01) to the audio mixer's three volumes.
//! - The scripts' natives read the same bytes: `Sys.YAxisInverted`
//!   (+0x0D), `Sys.IsAutosaveEnabled`/`SetAutosaveEnabled` (+0x0F),
//!   `Sys.RumbleEnabled` (+0x11), `Player.Get/SetFuelCapacity` (+0x25C),
//!   `Player.Get/SetAvailableCostumes` (+0x1B2). The loading screen's hint
//!   counter is +0x0C (`0x8236E248`).
//! - A new profile with no saves takes the console's own Y inversion,
//!   vibration and sensitivity (setting ids `0x10040002`, `0x10040003`,
//!   `0x10040018`; low 25, high 75, else 50: [`console_sensitivity`]).
//!
//! - The volumes, by their option names' hashes: `audioSFXVolume`
//!   (`0xE65739B8`) +0x16, `audioMusicVolume` (`0xD6E00BD8`) +0x17,
//!   `audioDialogVolume` (`0x940B6B17`) +0x18, in the mixer's order.
//!   `gameDifficulty` (`0x8FEF2A84`) is +0x0E (default 2);
//!   `joingameMap` and `joingameMission` are the strings at +0xF2, +0x132.
//! - +0x13 is the brightness: `videoBrightness` (`0xBBBE452B`, setter
//!   `0x82373710`) writes the same renderer float the apply step fills
//!   from it.
//! - Subtitles start on only in the Russian game: the default is "the
//!   language is 8" (`0x8239B028`), and `Sys.GetLanguage` (`0x82345FC8`)
//!   names 8 `Russian` ([`Language`], [`default_subtitles`]).
//!
//! The save record's 0x3000-byte block at +0x470 holds the game's save
//! data ([`SaveDataHeader`]): the scripts' save text (as
//! [`crate::savedata`] writes it, at most [`SAVE_DATA_CAPACITY`] bytes)
//! compressed at level 9 (`0x828141A8`), behind a 12-byte header, when it
//! fits ([`fits_in_record`]; `0x8230C4A8`). Loading uncompresses it
//! (`0x82814260`) and runs `return ` and the text (`0x8230C880`), as
//! ours does. Saving also stamps the record: the time at +0x38, a count
//! from the scripts at +0x28, a 0x20-character name at +0x40 and a byte
//! at +0x65 (what each shows wasn't traced). Unknown fields are named by
//! offset.

use m2_layout::assert_layout;

/// The save record's version (`0x8239B2F8`).
pub const RECORD_VERSION: u32 = 4;
/// The save record's size after its version and size words.
pub const RECORD_SIZE: u32 = 0x3458;
/// How much of the profile object the save record covers (from +0x10).
pub const SAVED_LEN: usize = 0x3460;
/// How much of the options block goes to the console's title settings.
pub const TITLE_SETTINGS_LEN: usize = 0x260;
/// The options block's own version word.
pub const OPTIONS_VERSION: u32 = 4;
/// The volumes' default, 0 to 100 (`0x8239B028`).
pub const DEFAULT_VOLUME: u8 = 75;
/// The look sensitivity's default, 0 to 100 (`0x82B767B9`).
pub const DEFAULT_SENSITIVITY: u8 = 50;
/// The brightness's default (applied times 0.01 to the renderer).
pub const DEFAULT_BRIGHTNESS: u8 = 50;
/// The difficulty's default (`gameDifficulty`).
pub const DEFAULT_DIFFICULTY: u8 = 2;
/// The volumes' order in [`ProfileOptions::volumes`] and the mixer.
pub const VOLUME_SFX: usize = 0;
pub const VOLUME_MUSIC: usize = 1;
pub const VOLUME_DIALOG: usize = 2;
/// How much save text the game gathers at most (`0x8230C4A8`).
pub const SAVE_DATA_CAPACITY: usize = 0x18000;
/// The save record's block for the compressed save data.
pub const SAVE_BLOCK_LEN: usize = 0x3000;

/// The game's languages, as `Sys.GetLanguage` names them (`0x82345FC8`;
/// any other value is `English`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Language {
    English = 0,
    Spanish = 1,
    Italian = 2,
    French = 3,
    German = 4,
    Japanese = 5,
    Russian = 8,
}

/// Whether a new profile starts with subtitles on: only in Russian
/// (`0x8239B028`).
pub fn default_subtitles(language: Language) -> bool {
    language == Language::Russian
}

/// The header of the save data in the record's +0x470 block
/// (`0x8230C4A8`, `0x8230C880`), big-endian as stored.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SaveDataHeader {
    /// 1; anything else isn't loaded.
    pub kind: [u8; 4],
    /// The compressed data's length, after the header.
    pub packed_len: [u8; 4],
    /// The save text's length.
    pub text_len: [u8; 4],
}

assert_layout!(SaveDataHeader, size = 12, {
    kind: 0,
    packed_len: 4,
    text_len: 8,
});

impl SaveDataHeader {
    /// The kind the game writes and loads.
    pub const KIND: u32 = 1;

    pub fn new(packed_len: u32, text_len: u32) -> Self {
        Self {
            kind: Self::KIND.to_be_bytes(),
            packed_len: packed_len.to_be_bytes(),
            text_len: text_len.to_be_bytes(),
        }
    }
}

/// Whether save data compressed to `packed_len` bytes goes in the save
/// record: header and data under [`SAVE_BLOCK_LEN`] (`0x8230C4A8`).
pub fn fits_in_record(packed_len: usize) -> bool {
    packed_len + std::mem::size_of::<SaveDataHeader>() < SAVE_BLOCK_LEN
}
/// The fuel capacity's default (`0x8239B028`, `0x8239C6D8`).
pub const DEFAULT_FUEL_CAPACITY: u32 = 300;
/// The profile name's default format (`0x8239C7A8`): `auto_` and the
/// current time as eight hex digits.
pub const DEFAULT_NAME_FORMAT: &str = "auto_%08X";

/// The options block as the 360 stores it (offsets from the profile's
/// +0x70). Numbers are big-endian bytes as stored; see the accessors.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProfileOptions {
    /// [`OPTIONS_VERSION`], big-endian.
    pub version: [u8; 4],
    /// The online agreement accepted (`serverAccept`); default 2.
    pub server_accept: u8,
    pub field_05: u8,
    /// `serverEAagreement`.
    pub ea_agreement: u8,
    pub field_07: u8,
    pub field_08: [u8; 4],
    /// The loading screen's hint counter (`0x8236E248`).
    pub hint_counter: u8,
    /// `gameInvert`: the look's Y axis inverted.
    pub invert_y: u8,
    /// `gameDifficulty`; default 2.
    pub difficulty: u8,
    /// `gameAutosave`.
    pub autosave: u8,
    /// `gameSensitivity`, 0 to 100.
    pub sensitivity: u8,
    /// `gameRumble`.
    pub rumble: u8,
    /// `gameTutorials`.
    pub tutorials: u8,
    /// The brightness (`videoBrightness`), applied times 0.01 to the
    /// renderer; default 50.
    pub brightness: u8,
    /// `videoSubtitles`.
    pub subtitles: u8,
    pub field_15: u8,
    /// The SFX, music and dialog volumes, 0 to 100 ([`VOLUME_SFX`]...).
    pub volumes: [u8; 3],
    pub field_19: [u8; 3],
    pub field_1c: [u8; 200],
    pub field_e4: [u8; 4],
    pub field_e8: [u8; 4],
    pub field_ec: [u8; 4],
    pub field_f0: u8,
    pub field_f1: u8,
    /// `joingameMap`.
    pub joingame_map: [u8; 0x40],
    /// `joingameMission`.
    pub joingame_mission: [u8; 0x40],
    /// The profile's name (`auto_XXXXXXXX` by default).
    pub name: [u8; 0x40],
    /// `Player.GetAvailableCostumes` and the byte after it.
    pub available_costumes: [u8; 2],
    pub field_1b4: u8,
    pub field_1b5: [u8; 3],
    pub field_1b8: [u8; 0x50],
    pub field_208: [u8; 0x50],
    pub field_258: [u8; 4],
    /// `Player.GetFuelCapacity`, big-endian.
    pub fuel_capacity: [u8; 4],
}

assert_layout!(ProfileOptions, size = TITLE_SETTINGS_LEN, {
    version: 0x00,
    server_accept: 0x04,
    ea_agreement: 0x06,
    field_08: 0x08,
    hint_counter: 0x0C,
    invert_y: 0x0D,
    autosave: 0x0F,
    sensitivity: 0x10,
    rumble: 0x11,
    tutorials: 0x12,
    brightness: 0x13,
    subtitles: 0x14,
    volumes: 0x16,
    field_1c: 0x1C,
    field_e4: 0xE4,
    field_ec: 0xEC,
    field_f0: 0xF0,
    joingame_map: 0xF2,
    joingame_mission: 0x132,
    name: 0x172,
    available_costumes: 0x1B2,
    field_1b4: 0x1B4,
    field_1b8: 0x1B8,
    field_208: 0x208,
    field_258: 0x258,
    fuel_capacity: 0x25C,
});

/// What the console's own settings give a new profile (`0x8239C850`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ConsoleSettings {
    pub invert_y: bool,
    /// Vibration; `None` keeps the game's default (on).
    pub vibration: Option<bool>,
    /// The control sensitivity setting (1 low, 2 high, else medium).
    pub sensitivity: Option<u32>,
}

/// The look sensitivity the console's sensitivity setting gives a new
/// profile: low 25, high 75, anything else 50 (`0x8239C850`).
pub fn console_sensitivity(level: u32) -> u8 {
    match level {
        1 => 25,
        2 => 75,
        _ => DEFAULT_SENSITIVITY,
    }
}

impl Default for ProfileOptions {
    /// The defaults `0x8239B028` writes (the name left empty: the game
    /// stamps it with the time, [`DEFAULT_NAME_FORMAT`]).
    fn default() -> Self {
        Self {
            version: OPTIONS_VERSION.to_be_bytes(),
            server_accept: 2,
            field_05: 1,
            ea_agreement: 0,
            field_07: 1,
            field_08: [0; 4],
            hint_counter: 0,
            invert_y: 0,
            difficulty: DEFAULT_DIFFICULTY,
            autosave: 1,
            sensitivity: DEFAULT_SENSITIVITY,
            rumble: 1,
            tutorials: 1,
            brightness: DEFAULT_BRIGHTNESS,
            subtitles: 0,
            field_15: 1,
            volumes: [DEFAULT_VOLUME; 3],
            field_19: [0; 3],
            field_1c: [0; 200],
            field_e4: [0; 4],
            field_e8: [0; 4],
            field_ec: 1u32.to_be_bytes(),
            field_f0: 3,
            field_f1: 0,
            joingame_map: [0; 0x40],
            joingame_mission: [0; 0x40],
            name: [0; 0x40],
            available_costumes: [0; 2],
            field_1b4: 0,
            field_1b5: [0; 3],
            field_1b8: [0; 0x50],
            field_208: [0; 0x50],
            field_258: [0; 4],
            fuel_capacity: DEFAULT_FUEL_CAPACITY.to_be_bytes(),
        }
    }
}

impl ProfileOptions {
    /// A new profile's options: the defaults, with the console's Y
    /// inversion, vibration and sensitivity where it has them
    /// (`0x8239C850`, for a profile with no saves).
    pub fn new(console: ConsoleSettings) -> Self {
        let d = Self::default();
        Self {
            invert_y: u8::from(console.invert_y),
            rumble: console.vibration.map_or(d.rumble, u8::from),
            sensitivity: console
                .sensitivity
                .map_or(d.sensitivity, console_sensitivity),
            ..d
        }
    }

    /// The fuel capacity (`Player.GetFuelCapacity`).
    pub fn fuel_capacity(&self) -> u32 {
        u32::from_be_bytes(self.fuel_capacity)
    }

    /// A value applied the 360's way: the byte times 0.01.
    pub fn scaled(byte: u8) -> f32 {
        f32::from(byte) * 0.01
    }

    /// The block's bytes as stored.
    pub fn to_bytes(&self) -> [u8; TITLE_SETTINGS_LEN] {
        let mut b = [0u8; TITLE_SETTINGS_LEN];
        let mut at = 0;
        let mut put = |s: &[u8]| {
            b[at..at + s.len()].copy_from_slice(s);
            at += s.len();
        };
        put(&self.version);
        put(&[
            self.server_accept,
            self.field_05,
            self.ea_agreement,
            self.field_07,
        ]);
        put(&self.field_08);
        put(&[
            self.hint_counter,
            self.invert_y,
            self.difficulty,
            self.autosave,
            self.sensitivity,
            self.rumble,
            self.tutorials,
            self.brightness,
            self.subtitles,
            self.field_15,
        ]);
        put(&self.volumes);
        put(&self.field_19);
        put(&self.field_1c);
        put(&self.field_e4);
        put(&self.field_e8);
        put(&self.field_ec);
        put(&[self.field_f0, self.field_f1]);
        put(&self.joingame_map);
        put(&self.joingame_mission);
        put(&self.name);
        put(&self.available_costumes);
        put(&[self.field_1b4]);
        put(&self.field_1b5);
        put(&self.field_1b8);
        put(&self.field_208);
        put(&self.field_258);
        put(&self.fuel_capacity);
        b
    }
}

/// The CRC the game keeps of the title settings block (`0x8239C850`,
/// `0x8239CCF8`): CRC-32 most significant bit first, polynomial
/// `0x04C11DB7` (its table at `0x82B7C190`), starting from all ones and
/// inverted at the end.
pub fn settings_crc(bytes: &[u8]) -> u32 {
    let mut crc = u32::MAX;
    for &b in bytes {
        crc ^= u32::from(b) << 24;
        for _ in 0..8 {
            crc = if crc & 0x8000_0000 != 0 {
                (crc << 1) ^ 0x04C1_1DB7
            } else {
                crc << 1
            };
        }
    }
    !crc
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The defaults `0x8239B028` writes.
    #[test]
    fn the_defaults_are_the_360s() {
        let o = ProfileOptions::default();
        assert_eq!(o.sensitivity, 50);
        assert_eq!(o.volumes, [75, 75, 75]);
        assert_eq!(
            (o.rumble, o.autosave, o.tutorials, o.invert_y),
            (1, 1, 1, 0)
        );
        assert_eq!(o.fuel_capacity(), 300);
        assert_eq!(o.hint_counter, 0);
        assert_eq!(o.brightness, 50);
        let b = o.to_bytes();
        assert_eq!(&b[0..4], &[0, 0, 0, 4]);
        assert_eq!(b[0x04], 2);
        assert_eq!(b[0x0E], 2);
        assert_eq!(b[0x10], 50);
        assert_eq!(&b[0x16..0x19], &[75; 3]);
        assert_eq!(&b[0xEC..0xF0], &[0, 0, 0, 1]);
        assert_eq!(b[0xF0], 3);
        assert_eq!(&b[0x25C..], &300u32.to_be_bytes());
    }

    /// The console's settings seed a new profile (`0x8239C850`).
    #[test]
    fn the_console_seeds_a_new_profile() {
        assert_eq!(console_sensitivity(1), 25);
        assert_eq!(console_sensitivity(2), 75);
        assert_eq!(console_sensitivity(0), 50);
        assert_eq!(console_sensitivity(7), 50);
        let o = ProfileOptions::new(ConsoleSettings {
            invert_y: true,
            vibration: Some(false),
            sensitivity: Some(2),
        });
        assert_eq!((o.invert_y, o.rumble, o.sensitivity), (1, 0, 75));
        assert_eq!(
            ProfileOptions::new(ConsoleSettings::default()),
            ProfileOptions::default()
        );
    }

    /// Bytes go where the layout has them.
    #[test]
    fn to_bytes_follows_the_layout() {
        let o = ProfileOptions {
            hint_counter: 7,
            invert_y: 1,
            subtitles: 1,
            name: {
                let mut n = [0; 0x40];
                n[..4].copy_from_slice(b"auto");
                n
            },
            available_costumes: [5, 6],
            ..ProfileOptions::default()
        };
        let b = o.to_bytes();
        assert_eq!((b[0x0C], b[0x0D], b[0x14]), (7, 1, 1));
        assert_eq!(&b[0x172..0x176], b"auto");
        assert_eq!(&b[0x1B2..0x1B4], &[5, 6]);
    }

    /// The volumes by name, the language rule for subtitles, and the save
    /// data's header and room.
    #[test]
    fn volumes_subtitles_and_save_data() {
        let o = ProfileOptions::default();
        assert_eq!(o.volumes[VOLUME_MUSIC], 75);
        assert_eq!((VOLUME_SFX, VOLUME_MUSIC, VOLUME_DIALOG), (0, 1, 2));
        assert_eq!((o.difficulty, o.brightness), (2, 50));
        assert!(!default_subtitles(Language::English));
        assert!(default_subtitles(Language::Russian));
        assert_eq!(Language::Russian as u32, 8);
        let h = SaveDataHeader::new(0x1234, 0x9000);
        assert_eq!(h.kind, [0, 0, 0, 1]);
        assert_eq!(h.packed_len, [0, 0, 0x12, 0x34]);
        assert!(fits_in_record(0x3000 - 13));
        assert!(!fits_in_record(0x3000 - 12));
        assert_eq!(SAVE_DATA_CAPACITY, 0x18000);
    }

    /// CRC-32 MSB-first with the 360's polynomial: the standard check
    /// value of that variant ("123456789" -> 0xFC891918).
    #[test]
    fn the_settings_crc_is_msb_first_crc32() {
        assert_eq!(settings_crc(b"123456789"), 0xFC89_1918);
    }
}
