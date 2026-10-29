//! Animation tables (`animationtable`): rows of name hashes that pick the
//! clip a character or vehicle plays for an animation key.
//!
//! Characters are animated by key: a script sets a state and an action
//! (`Human.SetState(char, "InVehicle", "ActionHijackHijackerA")`) and two
//! tables turn that into a clip. One (`6802c321`) maps a stance and action
//! (the action's [`name_hash`](crate::name_hash)), with filter columns, to
//! an animation handle; another maps the handle, with the vehicle's class
//! and name (`182624e1`) or the character's (`e00b080c`), to the clip.
//!
//! A `UCFX` container of top-level leaf sections `INFO`, `TYPE`, an
//! optional `ASTO`, then `VALU`; everything little-endian.
//!
//! ```text
//! INFO  u16  6 in every table (meaning unknown)
//!       u16  column count c
//!       u16  row count r
//! TYPE  per column: its name (ASCII, NUL-terminated), u16 type code
//!       2 integer, 3 f32, 4 name hash, 8 word offset into ASTO
//! ASTO  u32 word count, then lists: u32 n, n name hashes
//! VALU  r rows of c u32 cells, row-major, nothing between rows
//! ```
//!
//! A list cell is the word offset of its list counted from the first word
//! after `ASTO`'s count, so the first list is at 0. Measured on the
//! on-foot table ([`CHARACTER_ANIMATIONS`], 1485 rows, `ASTO` 0x2f94 bytes
//! at 0xac in the payload): read so, every row's list is followed by the
//! next row's (row 0's `[ANY]` at 0, row 1's at 2, ... row 12's two
//! hashes at 0x18, row 13's at 0x1b), and all 1484 lists of rows 1 on
//! name clips in the archive; counted from the count word itself (as read
//! before), or from any other base, no more than 1418 do and the lists
//! run into each other.
//!
//! Row 0 has every cell [`ANY`]. A filter cell holding [`ANY`] matches
//! anything; the row that matches with the most filters set wins (the
//! vehicle tables give a class-only row and a row for the vehicle itself
//! under one handle).
//!
//! # Provenance
//!
//! Format-from-samples: inferred from the 15 tables in a legally installed
//! copy's `resident` block (round H7), which all parse with these rules.
//! The precedence rule is inferred from how the rows overlap, not seen
//! working. No bytes from the archives are committed.

use crate::bytes::{OutOfBounds, Reader};
use crate::ucfx::{Ucfx, UcfxError};

/// Asset type hash of animation tables.
pub const ANIMATION_TABLE_TYPE: u32 = 0x2073_59c7;
/// The "anything" cell (its source name is unknown).
pub const ANY: u32 = 0x27de_7135;
/// The characters' stance and action table.
pub const ACTIONS: u32 = 0x6802_c321;
/// The vehicle handle table.
pub const VEHICLE_ANIMATIONS: u32 = 0x1826_24e1;
/// The on-foot handle table.
pub const CHARACTER_ANIMATIONS: u32 = 0xe00b_080c;
/// The on-foot table's columns for who plays a row's clips, named as the
/// `HumanAnimationSet` fields that say it of a character (the table's
/// own column names; that the template's fields fill them is inferred, as
/// the vehicle table's `VehicleClass` and `VehicleName` are its
/// `VehicleAnimationSet`'s).
pub const GENDER: &str = "Gender";
pub const CHARACTER_NAME: &str = "CharacterName";
/// The on-foot table's other columns: the animation handle a row gives
/// clips for, what the character holds (its primary weapon's class and
/// name, and the class and name of what is in use, a grenade being
/// thrown), and the clips.
pub const HANDLE: &str = "Handle";
pub const PRIMARY_EQUIPMENT_CLASS: &str = "PrimaryEquipmentClass";
pub const PRIMARY_EQUIPMENT_NAME: &str = "PrimaryEquipmentName";
pub const IN_USE_EQUIPMENT_CLASS: &str = "InUseEquipmentClass";
pub const IN_USE_EQUIPMENT_NAME: &str = "InUseEquipmentName";
pub const ANIMATION: &str = "Animation";
/// The on-foot and vehicle tables' playback-rate limits (f32 cells, -1
/// for "none"). From the 360 code: the clip pick reads the on-foot
/// table's columns 8 and 9 (82802360, 82802398) and the vehicle table's
/// 7 and 8 (828021F0, 82802228), the columns after `Animation`.
pub const MIN_TIME_SCALE: &str = "MinTimeScale";
pub const MAX_TIME_SCALE: &str = "MaxTimeScale";
/// The on-foot table's equipment class of a rifle: `name_hash("rifle")`.
/// The class cells are the name hashes of plain words (`rifle`, `pistol`,
/// `rocket`, `grenade`, `golfclub`, `box`, and one not yet named,
/// `d20ec345`); the heroes' carbine, assault rifle and bullpup are taken
/// to be `rifle`s (inferred: the weapon templates' own equipment class
/// field is not decoded).
pub const RIFLE: u32 = 0xd045_9a41;

/// A column's type code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColumnKind {
    Integer,
    Float,
    Hash,
    /// A word offset into `ASTO`: a list of hashes.
    List,
    Other(u16),
}

impl From<u16> for ColumnKind {
    fn from(code: u16) -> Self {
        match code {
            2 => Self::Integer,
            3 => Self::Float,
            4 => Self::Hash,
            8 => Self::List,
            c => Self::Other(c),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Column {
    pub name: String,
    pub kind: ColumnKind,
}

/// Errors from parsing an animation table.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum AnimationTableError {
    #[error("container: {0}")]
    Ucfx(#[from] UcfxError),
    #[error("truncated: needed {len} bytes at {offset:#x}")]
    Truncated { offset: usize, len: usize },
    #[error("no {0} section")]
    Missing(&'static str),
    #[error("TYPE names {found} columns, INFO says {expected}")]
    Columns { expected: u16, found: usize },
    #[error("VALU is {len:#x} bytes, not {rows} rows of {columns} cells")]
    Values { len: usize, rows: u16, columns: u16 },
}

impl From<OutOfBounds> for AnimationTableError {
    fn from(e: OutOfBounds) -> Self {
        Self::Truncated {
            offset: e.offset,
            len: e.len,
        }
    }
}

/// A parsed animation table.
#[derive(Debug, Clone, PartialEq)]
pub struct AnimationTable {
    pub columns: Vec<Column>,
    rows: usize,
    cells: Vec<u32>,
    /// `ASTO`'s words after its count: what a list cell indexes.
    lists: Vec<u32>,
}

fn words(data: &[u8]) -> Vec<u32> {
    data.chunks_exact(4)
        .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        .collect()
}

impl AnimationTable {
    pub fn parse(asset: &[u8]) -> Result<Self, AnimationTableError> {
        let u = Ucfx::parse(asset)?;
        let info = u
            .top_level(*b"INFO")
            .ok_or(AnimationTableError::Missing("INFO"))?;
        let mut r = Reader::at(info, 0);
        r.u16()?;
        let columns = r.u16()?;
        let rows = r.u16()?;
        let ty = u
            .top_level(*b"TYPE")
            .ok_or(AnimationTableError::Missing("TYPE"))?;
        let mut r = Reader::at(ty, 0);
        let mut cols = Vec::new();
        while r.remaining() > 0 && cols.len() < usize::from(columns) {
            let name = String::from_utf8_lossy(r.cstr()?).into_owned();
            let kind = ColumnKind::from(r.u16()?);
            cols.push(Column { name, kind });
        }
        if cols.len() != usize::from(columns) {
            return Err(AnimationTableError::Columns {
                expected: columns,
                found: cols.len(),
            });
        }
        let valu = u
            .top_level(*b"VALU")
            .ok_or(AnimationTableError::Missing("VALU"))?;
        if valu.len() != usize::from(rows) * usize::from(columns) * 4 {
            return Err(AnimationTableError::Values {
                len: valu.len(),
                rows,
                columns,
            });
        }
        let lists = u
            .top_level(*b"ASTO")
            .map(|a| words(a).into_iter().skip(1).collect())
            .unwrap_or_default();
        Ok(Self {
            columns: cols,
            rows: usize::from(rows),
            cells: words(valu),
            lists,
        })
    }

    pub fn rows(&self) -> usize {
        self.rows
    }

    /// The index of the column with this name.
    pub fn column(&self, name: &str) -> Option<usize> {
        self.columns.iter().position(|c| c.name == name)
    }

    /// A cell's raw word.
    pub fn cell(&self, row: usize, column: usize) -> Option<u32> {
        if column >= self.columns.len() {
            return None;
        }
        self.cells.get(row * self.columns.len() + column).copied()
    }

    /// The hashes of a [`ColumnKind::List`] cell.
    pub fn list(&self, row: usize, column: usize) -> &[u32] {
        let Some(at) = self.cell(row, column) else {
            return &[];
        };
        // Counted from the word after ASTO's count (see the module docs).
        let at = at as usize;
        let Some(&n) = self.lists.get(at) else {
            return &[];
        };
        self.lists.get(at + 1..at + 1 + n as usize).unwrap_or(&[])
    }

    /// The row matching every filter, `(column, value)` with `None` for
    /// "not given", that sets the most of them. A filter matches a cell
    /// holding its value or [`ANY`]; a filter not given matches only
    /// [`ANY`]. Columns not named are ignored. Ties go to the first row.
    pub fn best(&self, filters: &[(usize, Option<u32>)]) -> Option<usize> {
        let mut best: Option<(usize, usize)> = None;
        'rows: for row in 1..self.rows {
            let mut set = 0;
            for &(column, value) in filters {
                let cell = self.cell(row, column)?;
                if cell == ANY {
                    continue;
                }
                if Some(cell) != value {
                    continue 'rows;
                }
                set += 1;
            }
            if best.is_none_or(|(_, s)| set > s) {
                best = Some((row, set));
            }
        }
        best.map(|(row, _)| row)
    }

    /// [`best`](Self::best) by column names; unknown columns fail.
    pub fn lookup(&self, filters: &[(&str, Option<u32>)], out: &str) -> Option<u32> {
        let row = self.lookup_row(filters)?;
        self.cell(row, self.column(out)?).filter(|&v| v != ANY)
    }

    /// The row [`lookup`](Self::lookup) reads.
    pub fn lookup_row(&self, filters: &[(&str, Option<u32>)]) -> Option<usize> {
        let filters: Option<Vec<_>> = filters
            .iter()
            .map(|&(name, v)| Some((self.column(name)?, v)))
            .collect();
        self.best(&filters?)
    }
}

/// The clip a vehicle-seated character plays for a state and action, via
/// the [`ACTIONS`] and [`VEHICLE_ANIMATIONS`] tables, for a vehicle of
/// this `VehicleAnimationSet` class and name.
pub fn vehicle_clip(
    actions: &AnimationTable,
    vehicles: &AnimationTable,
    stance: &str,
    action: &str,
    class: u32,
    name: u32,
) -> Option<u32> {
    seat_clip(actions, vehicles, stance, action, (class, name), None)
}

/// [`vehicle_clip`] for a character in the seat whose name hashes to
/// `seat` (`seat_driver`), where the vehicle table gives seats their own.
pub fn seat_clip(
    actions: &AnimationTable,
    vehicles: &AnimationTable,
    stance: &str,
    action: &str,
    set: (u32, u32),
    seat: Option<u32>,
) -> Option<u32> {
    seat_clip_row(actions, vehicles, stance, action, set, seat).map(|(clip, _)| clip)
}

/// [`seat_clip`], with the row of the vehicle table that gives it: which
/// of its cells name the vehicle's class, the vehicle and the seat, and
/// which are [`ANY`], says whether the clip is the vehicle's own.
///
/// The seat filters both tables: the stance and action table's `Seat`
/// column names seats too (round 13: its 43 `InVehicle` `Idle` rows each
/// name one, `seat_driver` among them, and none is any), so without it
/// the lookup fell through to the table's catch-all row 1 and every
/// seat of every vehicle got the vehicle table's catch-all clip. That
/// the column takes the `SeatLink` `Name` is inferred from the values:
/// the tables' seat cells are the templates' seat names.
pub fn seat_clip_row(
    actions: &AnimationTable,
    vehicles: &AnimationTable,
    stance: &str,
    action: &str,
    (class, name): (u32, u32),
    seat: Option<u32>,
) -> Option<(u32, usize)> {
    let handle = actions.lookup(
        &[
            ("Stance", Some(crate::name_hash(stance))),
            ("Action", Some(crate::name_hash(action))),
            ("AimState", None),
            ("Tandem", None),
            ("Seat", seat),
            ("Target", None),
            ("ActionDirection", None),
            ("DamageDirection", None),
        ],
        "AnimationHandles",
    )?;
    let row = vehicles.lookup_row(&[
        ("Handle", Some(handle)),
        ("VehicleClass", Some(class)),
        ("VehicleName", Some(name)),
        ("SeatName", seat),
        ("MountedEquipmentClass", None),
        ("MountedEquipmentName", None),
    ])?;
    let clip = vehicles
        .cell(row, vehicles.column("Animation")?)
        .filter(|&v| v != ANY)?;
    Some((clip, row))
}

/// How far down the on-foot table's precedence a character's clip was
/// found ([`character_clip`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Precedence {
    /// The character's own row for the handle and equipment.
    Own,
    /// Anyone's row for the handle and equipment.
    Equipment,
    /// The character's own row for the handle, whatever is held (the
    /// heroes' own pickups: Chris's and Mattias's rows name no weapon).
    OwnAny,
    /// Anyone's row for the handle, whatever is held.
    Handle,
}

/// A clip the on-foot table gives a character ([`character_clip`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CharacterClip {
    pub clip: u32,
    /// The animation handle the key clip's row gave.
    pub handle: u32,
    /// The row the clip is listed in.
    pub row: usize,
    pub by: Precedence,
}

/// The first row (after row 0) whose `Animation` list holds `clip`.
pub fn row_listing(table: &AnimationTable, clip: u32) -> Option<usize> {
    let list = table.column(ANIMATION)?;
    (1..table.rows()).find(|&r| table.list(r, list).contains(&clip))
}

/// The order a row's rate range is shared out in: the `n`th of eight
/// steps from `MinTimeScale` to `MaxTimeScale`, by `(id ^ clip) & 7`. From
/// the 360 code (82800FB8, a byte table on the stack).
pub const RATE_SPREAD: [u8; 8] = [5, 2, 7, 1, 4, 0, 6, 3];

/// The playback rate a clip node gives `clip` (its name hash) picked from
/// a row with `min` and `max` ([`MIN_TIME_SCALE`], [`MAX_TIME_SCALE`]),
/// for the character `id`. From the 360 code (82801650, 82800FB8,
/// 82175618):
///
/// - 1.0 when `min` is 0 or less (the tables' -1) or `max` is below it;
/// - `min` when the two are equal;
/// - else `min + (max - min) * RATE_SPREAD[(id ^ clip) & 7] / 7`, worked
///   as the game does (the step times the f32 1/7, then the lerp in
///   double, rounded to f32).
///
/// So each character plays a clip at its own fixed rate within the row's
/// range, the same every time. `id` is the game's entity handle (82420A00
/// passes it in); how the game numbers its handles is not known, so the
/// alpha's ids are stand-ins and which character gets which rate is
/// invented (the spread over a crowd is not). A NaN `min` gives 1.0.
pub fn clip_rate(min: f32, max: f32, id: u32, clip: u32) -> f32 {
    if min.is_nan() || min <= 0.0 || max < min {
        return 1.0;
    }
    if min == max {
        return min;
    }
    let step = f32::from(RATE_SPREAD[((id ^ clip) & 7) as usize]) * (1.0f32 / 7.0);
    (f64::from(max - min) * f64::from(step) + f64::from(min)) as f32
}

/// A row's `(MinTimeScale, MaxTimeScale)`; `None` if the table lacks the
/// columns or the row.
pub fn time_scales(table: &AnimationTable, row: usize) -> Option<(f32, f32)> {
    let min = table.cell(row, table.column(MIN_TIME_SCALE)?)?;
    let max = table.cell(row, table.column(MAX_TIME_SCALE)?)?;
    Some((f32::from_bits(min), f32::from_bits(max)))
}

/// Each listed clip's `(MinTimeScale, MaxTimeScale)`, from the first row
/// listing it ([`row_listing`]). Inferred: the game reads the limits of
/// the row its key query picked (82801508), which the alpha does not run
/// for every clip; a clip listed in rows with different limits takes the
/// first's. Rows with no limits (-1, the most) are left out.
pub fn clip_time_scales(table: &AnimationTable) -> std::collections::HashMap<u32, (f32, f32)> {
    let mut out = std::collections::HashMap::new();
    let Some(list) = table.column(ANIMATION) else {
        return out;
    };
    let mut seen = std::collections::HashSet::new();
    for row in 1..table.rows() {
        let scales = time_scales(table, row);
        for &clip in table.list(row, list) {
            if !seen.insert(clip) {
                continue;
            }
            if let Some(s) = scales.filter(|&(min, _)| min > 0.0) {
                out.insert(clip, s);
            }
        }
    }
    out
}

/// The clip a character plays for what `key` is played for, by the
/// on-foot table ([`CHARACTER_ANIMATIONS`]). The first row listing `key`
/// ([`row_listing`]; the heroes' keys are Jennifer's clips) gives the
/// animation handle, and what is in use (its `InUseEquipment` cells: a
/// grenade for a throw). Of that handle's rows with that in use and no
/// weapon named (`PrimaryEquipmentName` [`ANY`]), the first of:
///
/// 1. the character's own (`who`: its `HumanAnimationSet` gender and
///    character name; the row's gender may be [`ANY`]) with `equipment`
///    as the primary weapon's class ([`Precedence::Own`]);
/// 2. anyone's (character [`ANY`]; the gender the character's or
///    [`ANY`]) with `equipment` ([`Precedence::Equipment`]);
/// 3. the character's own with any equipment ([`Precedence::OwnAny`];
///    inferred: its place between 2 and 4 is not known);
/// 4. anyone's with any equipment ([`Precedence::Handle`]);
///
/// with a clip that `fits` (is on the character's rig): `key` if the row
/// lists it, else the first listed. An empty list, or one with nothing
/// that fits, falls through to the next (inferred: the game would have
/// nothing to play from it). Within a step, rows naming the gender come
/// before those that don't. `None` when no row lists `key`, nothing
/// fits, or the table lacks a column named here.
pub fn character_clip(
    table: &AnimationTable,
    key: u32,
    who: (u32, u32),
    equipment: u32,
    fits: impl Fn(u32) -> bool,
) -> Option<CharacterClip> {
    let col = |n: &str| table.column(n);
    let handle = col(HANDLE)?;
    let (uc, un) = (col(IN_USE_EQUIPMENT_CLASS)?, col(IN_USE_EQUIPMENT_NAME)?);
    let first = row_listing(table, key)?;
    let cell = |row: usize, column: usize| table.cell(row, column).unwrap_or(ANY);
    let h = cell(first, handle);
    let in_use = (cell(first, uc), cell(first, un));
    handle_rows(table, h, in_use, who, equipment, Some(key), fits)
}

/// The clip a character plays for animation handle `handle` (one the
/// stance and action table gives, [`action_handle`]), with nothing in
/// use: as [`character_clip`] picks it, the first listed that `fits`.
pub fn handle_clip(
    table: &AnimationTable,
    handle: u32,
    who: (u32, u32),
    equipment: u32,
    fits: impl Fn(u32) -> bool,
) -> Option<CharacterClip> {
    handle_rows(table, handle, (ANY, ANY), who, equipment, None, fits)
}

/// The animation handle the stance and action table ([`ACTIONS`]) gives
/// a character in `stance` doing `action`, with no other filter set
/// (`Upright`, `EquipRightIdle`). `None` where the row's handle is
/// [`ANY`] (`None` in the table's own words: the locomotion's clips, not
/// one of the on-foot table's).
pub fn action_handle(actions: &AnimationTable, stance: &str, action: &str) -> Option<u32> {
    actions.lookup(
        &[
            ("Stance", Some(crate::name_hash(stance))),
            ("Action", Some(crate::name_hash(action))),
            ("AimState", None),
            ("Tandem", None),
            ("Seat", None),
            ("Target", None),
            ("ActionDirection", None),
            ("DamageDirection", None),
        ],
        "AnimationHandles",
    )
}

/// [`character_clip`]'s and [`handle_clip`]'s search over handle `h`'s
/// rows with `in_use`, preferring `key` where a row lists it.
fn handle_rows(
    table: &AnimationTable,
    h: u32,
    in_use: (u32, u32),
    (gender, name): (u32, u32),
    equipment: u32,
    key: Option<u32>,
    fits: impl Fn(u32) -> bool,
) -> Option<CharacterClip> {
    let col = |n: &str| table.column(n);
    let handle = col(HANDLE)?;
    let (g, c) = (col(GENDER)?, col(CHARACTER_NAME)?);
    let (pc, pn) = (col(PRIMARY_EQUIPMENT_CLASS)?, col(PRIMARY_EQUIPMENT_NAME)?);
    let (uc, un) = (col(IN_USE_EQUIPMENT_CLASS)?, col(IN_USE_EQUIPMENT_NAME)?);
    let list = col(ANIMATION)?;
    if table.columns[list].kind != ColumnKind::List {
        return None;
    }
    let cell = |row: usize, column: usize| table.cell(row, column).unwrap_or(ANY);
    let steps = [
        (Precedence::Own, name, equipment),
        (Precedence::Equipment, ANY, equipment),
        (Precedence::OwnAny, name, ANY),
        (Precedence::Handle, ANY, ANY),
    ];
    for (by, character, class) in steps {
        let rows = (1..table.rows()).filter(|&r| {
            cell(r, handle) == h
                && cell(r, c) == character
                && cell(r, pc) == class
                && cell(r, pn) == ANY
                && (cell(r, uc), cell(r, un)) == in_use
        });
        let named = rows.clone().filter(|&r| cell(r, g) == gender);
        let unnamed = rows.filter(|&r| gender != ANY && cell(r, g) == ANY);
        for row in named.chain(unnamed) {
            let clips = table.list(row, list);
            let clip = if let Some(key) = key.filter(|&k| clips.contains(&k) && fits(k)) {
                Some(key)
            } else {
                clips.iter().copied().find(|&x| fits(x))
            };
            if let Some(clip) = clip {
                return Some(CharacterClip {
                    clip,
                    handle: h,
                    row,
                    by,
                });
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::name_hash;
    use crate::testutil::{UcfxNode, ucfx};

    fn table(columns: &[(&str, u16)], rows: &[&[u32]], asto: Option<&[u32]>) -> Vec<u8> {
        let mut info = vec![6, 0];
        info.extend_from_slice(&(columns.len() as u16).to_le_bytes());
        info.extend_from_slice(&(rows.len() as u16).to_le_bytes());
        let mut ty = Vec::new();
        for (name, code) in columns {
            ty.extend_from_slice(name.as_bytes());
            ty.push(0);
            ty.extend_from_slice(&code.to_le_bytes());
        }
        let valu: Vec<u8> = rows
            .iter()
            .flat_map(|r| r.iter())
            .flat_map(|w| w.to_le_bytes())
            .collect();
        let mut nodes = vec![UcfxNode::Leaf(*b"INFO", info), UcfxNode::Leaf(*b"TYPE", ty)];
        if let Some(a) = asto {
            let words: Vec<u8> = std::iter::once(a.len() as u32)
                .chain(a.iter().copied())
                .flat_map(u32::to_le_bytes)
                .collect();
            nodes.push(UcfxNode::Leaf(*b"ASTO", words));
        }
        nodes.push(UcfxNode::Leaf(*b"VALU", valu));
        ucfx(&nodes)
    }

    fn tables() -> (AnimationTable, AnimationTable) {
        let a_cols = [
            "Stance",
            "Action",
            "AimState",
            "Tandem",
            "Seat",
            "Target",
            "ActionDirection",
            "DamageDirection",
            "AnimationHandles",
        ];
        let a_cols: Vec<(&str, u16)> = a_cols.iter().map(|&c| (c, 4)).collect();
        let stance = name_hash("InVehicle");
        let action = name_hash("ActionHijackHijackerA");
        let any8 = [ANY; 9];
        let a = table(
            &a_cols,
            &[
                &any8,
                &[stance, action, ANY, ANY, ANY, ANY, ANY, ANY, 0x1111],
                &[stance, action, 7, ANY, ANY, ANY, ANY, ANY, 0x2222],
            ],
            None,
        );
        let b_cols: Vec<(&str, u16)> = [
            ("Handle", 4),
            ("VehicleClass", 4),
            ("VehicleName", 4),
            ("SeatName", 4),
            ("MountedEquipmentClass", 4),
            ("MountedEquipmentName", 4),
            ("Animation", 4),
            ("MinTimeScale", 3),
            ("MaxTimeScale", 3),
        ]
        .to_vec();
        let m = (-1.0f32).to_bits();
        let b = table(
            &b_cols,
            &[
                &[ANY; 9],
                &[0x1111, 0xc1, ANY, ANY, ANY, ANY, 0xaaaa, m, m],
                &[0x1111, 0xc1, 0x51, ANY, ANY, ANY, 0xbbbb, m, m],
            ],
            None,
        );
        (
            AnimationTable::parse(&a).unwrap(),
            AnimationTable::parse(&b).unwrap(),
        )
    }

    #[test]
    fn reads_columns_and_cells() {
        let (a, b) = tables();
        assert_eq!(a.rows(), 3);
        assert_eq!(a.columns[8].name, "AnimationHandles");
        assert_eq!(b.columns[7].kind, ColumnKind::Float);
        assert_eq!(b.cell(2, 6), Some(0xbbbb));
        assert_eq!(b.cell(1, 9), None);
    }

    #[test]
    fn the_vehicle_row_outranks_its_class_row() {
        let (a, b) = tables();
        let clip = |name| vehicle_clip(&a, &b, "InVehicle", "ActionHijackHijackerA", 0xc1, name);
        assert_eq!(clip(0x51), Some(0xbbbb));
        assert_eq!(clip(0x52), Some(0xaaaa));
        assert_eq!(
            vehicle_clip(&a, &b, "InVehicle", "ActionHijackHijackerA", 0xc2, 0x51),
            None
        );
        assert_eq!(vehicle_clip(&a, &b, "InVehicle", "Idle", 0xc1, 0x51), None);
        // With the row it came from: the vehicle's own, else its class's.
        let row = |name| {
            seat_clip_row(
                &a,
                &b,
                "InVehicle",
                "ActionHijackHijackerA",
                (0xc1, name),
                None,
            )
        };
        assert_eq!(row(0x51), Some((0xbbbb, 2)));
        assert_eq!(row(0x52), Some((0xaaaa, 1)));
    }

    #[test]
    fn lists_count_from_the_word_after_asto_count() {
        // As the on-foot table's first rows lie: row 0's [ANY] at word 0
        // (just after the count), then lists end to end.
        let asset = table(
            &[("Handle", 4), ("Animation", 8)],
            &[&[ANY, 0], &[0x10, 2], &[0x11, 4], &[0x12, 7]],
            Some(&[1, ANY, 1, 0x9d88_6544, 2, 0x696b_b391, 0x7bfb_a041, 0]),
        );
        let t = AnimationTable::parse(&asset).unwrap();
        assert_eq!(t.list(0, 1), &[ANY]);
        assert_eq!(t.list(1, 1), &[0x9d88_6544]);
        assert_eq!(t.list(2, 1), &[0x696b_b391, 0x7bfb_a041]);
        assert_eq!(t.list(3, 1), &[] as &[u32]);
        // Not a list cell's offset: nothing.
        assert_eq!(t.list(1, 0), &[] as &[u32]);
        assert_eq!(row_listing(&t, 0x7bfb_a041), Some(2));
        assert_eq!(row_listing(&t, ANY), None);
    }

    /// An on-foot table laid out as the game's (its columns, row 0, lists
    /// after ASTO's count), with rows of the game's for a few handles.
    fn on_foot() -> AnimationTable {
        let hero = name_hash("Hero");
        let (jen, mat, chris) = (
            name_hash("jennifer"),
            name_hash("mattias"),
            name_hash("Chris"),
        );
        let (pistol, rocket, grenade) = (
            name_hash("pistol"),
            name_hash("rocket"),
            name_hash("grenade"),
        );
        let (aim, run, takeoff, jump, throw, still) = (
            0x9f4e_b947,
            0xb99a_d3b3,
            0x2ae2_19a8,
            0xcca0_2d6b,
            0x399f_45f4,
            0x24af_79c8,
        );
        // (handle, gender, character, primary class, in-use class, clips)
        type Row<'a> = (u32, u32, u32, u32, u32, &'a [u32]);
        let rows: [Row; 21] = [
            (ANY, ANY, ANY, ANY, ANY, &[ANY]),
            (aim, ANY, ANY, RIFLE, ANY, &[0xb29a_c474]),
            (aim, ANY, ANY, pistol, ANY, &[0xed1d_af4f]),
            (aim, hero, mat, RIFLE, ANY, &[0xd2b7_e258]),
            (aim, hero, mat, pistol, ANY, &[0xb200_25cb]),
            (aim, hero, jen, RIFLE, ANY, &[0x0163_f2aa]),
            (run, ANY, ANY, RIFLE, ANY, &[0xfa14_1739]),
            (run, hero, jen, RIFLE, ANY, &[0xe340_14c3]),
            (run, hero, chris, RIFLE, ANY, &[]),
            (run, hero, mat, RIFLE, ANY, &[0x1091_2665]),
            (takeoff, ANY, ANY, RIFLE, ANY, &[0xc722_9469]),
            (takeoff, hero, jen, rocket, ANY, &[0xdaf7_966f]),
            (takeoff, hero, jen, RIFLE, ANY, &[0x0e18_b683]),
            (takeoff, hero, chris, RIFLE, ANY, &[0xbf3a_9fa7]),
            (jump, ANY, ANY, ANY, ANY, &[0xc846_e6fd]),
            (throw, ANY, ANY, RIFLE, grenade, &[0x690b_c7d1]),
            (throw, ANY, ANY, rocket, grenade, &[0xec6f_fa31]),
            (still, ANY, ANY, ANY, ANY, &[0xe824_3172, 0x3aee_9ae8]),
            // `PickupWeapon`'s handle: anyone's, Chris's own whatever he
            // holds, Jennifer's with a pistol.
            (PICKUP, ANY, ANY, ANY, ANY, &[0x9874_62fc]),
            (PICKUP, hero, chris, ANY, ANY, &[0x4916_51da]),
            (PICKUP, hero, jen, pistol, ANY, &[0x777b_d24e]),
        ];
        let mut asto = Vec::new();
        let m = (-1.0f32).to_bits();
        let cells: Vec<[u32; 10]> = rows
            .iter()
            .map(|&(h, g, c, p, u, clips)| {
                let at = asto.len() as u32;
                asto.push(clips.len() as u32);
                asto.extend_from_slice(clips);
                [h, g, c, p, ANY, u, ANY, at, m, m]
            })
            .collect();
        // One more throw row, not in use: never picked for a throw.
        let mut cells = cells;
        let at = asto.len() as u32;
        asto.extend_from_slice(&[1, 0xdead]);
        cells.push([throw, ANY, ANY, RIFLE, ANY, ANY, ANY, at, m, m]);
        let columns = [
            (HANDLE, 4),
            (GENDER, 4),
            (CHARACTER_NAME, 4),
            (PRIMARY_EQUIPMENT_CLASS, 4),
            (PRIMARY_EQUIPMENT_NAME, 4),
            (IN_USE_EQUIPMENT_CLASS, 4),
            (IN_USE_EQUIPMENT_NAME, 4),
            (ANIMATION, 8),
            ("MinTimeScale", 3),
            ("MaxTimeScale", 3),
        ];
        let rows: Vec<&[u32]> = cells.iter().map(|r| &r[..]).collect();
        AnimationTable::parse(&table(&columns, &rows, Some(&asto))).unwrap()
    }

    #[test]
    fn rifle_is_the_name_hash_of_its_word() {
        assert_eq!(name_hash("rifle"), RIFLE);
    }

    #[test]
    fn a_hero_gets_their_own_rifle_row_then_anyones() {
        let t = on_foot();
        let hero = name_hash("Hero");
        let who = |name: &str| (hero, name_hash(name));
        let all = |_| true;
        let pick = |key, name: &str| {
            character_clip(&t, key, who(name), RIFLE, all).map(|p| (p.clip, p.by))
        };
        use Precedence::*;
        // The aim, keyed by hers: his own rifle aim (not his pistol one);
        // Chris has no row: anyone's rifle aim.
        assert_eq!(pick(0x0163_f2aa, "jennifer"), Some((0x0163_f2aa, Own)));
        assert_eq!(pick(0x0163_f2aa, "mattias"), Some((0xd2b7_e258, Own)));
        assert_eq!(pick(0x0163_f2aa, "Chris"), Some((0xb29a_c474, Equipment)));
        let p = character_clip(&t, 0x0163_f2aa, who("mattias"), RIFLE, all).unwrap();
        assert_eq!((p.handle, p.row), (0x9f4e_b947, 3));
        // Chris's own rifle run is an empty list: anyone's.
        assert_eq!(pick(0xe340_14c3, "Chris"), Some((0xfa14_1739, Equipment)));
        assert_eq!(pick(0xe340_14c3, "mattias"), Some((0x1091_2665, Own)));
        // Her take-off is a rocket one: the rifle's for everyone, hers too.
        assert_eq!(pick(0xdaf7_966f, "jennifer"), Some((0x0e18_b683, Own)));
        assert_eq!(pick(0xdaf7_966f, "Chris"), Some((0xbf3a_9fa7, Own)));
        assert_eq!(pick(0xdaf7_966f, "mattias"), Some((0xc722_9469, Equipment)));
        // Only anyone's with anything: the jump.
        assert_eq!(pick(0xc846_e6fd, "mattias"), Some((0xc846_e6fd, Handle)));
        // A throw keeps the grenade in use.
        assert_eq!(pick(0xec6f_fa31, "mattias"), Some((0x690b_c7d1, Equipment)));
        // The key itself where its row lists it and it fits, else the
        // first that fits; a row with nothing fitting falls through.
        assert_eq!(pick(0x3aee_9ae8, "Chris"), Some((0x3aee_9ae8, Handle)));
        let not = |x: u32| move |c: u32| c != x;
        let only = |key, name: &str, fits: &dyn Fn(u32) -> bool| {
            character_clip(&t, key, who(name), RIFLE, fits).map(|p| p.clip)
        };
        assert_eq!(
            only(0x3aee_9ae8, "Chris", &not(0x3aee_9ae8)),
            Some(0xe824_3172)
        );
        assert_eq!(
            only(0x0163_f2aa, "mattias", &not(0xd2b7_e258)),
            Some(0xb29a_c474)
        );
        assert_eq!(only(0x0163_f2aa, "mattias", &|_| false), None);
        // Not listed: nothing.
        assert_eq!(pick(0x1234_5678, "mattias"), None);
    }

    #[test]
    fn a_table_without_the_on_foot_columns_gives_nothing() {
        let asset = table(
            &[("Handle", 4), (CHARACTER_NAME, 4), ("Animation", 8)],
            &[&[ANY, ANY, 0], &[0x10, ANY, 2]],
            Some(&[1, ANY, 1, 0x0163_f2aa]),
        );
        let t = AnimationTable::parse(&asset).unwrap();
        assert_eq!(row_listing(&t, 0x0163_f2aa), Some(1));
        assert_eq!(
            character_clip(&t, 0x0163_f2aa, (1, 2), RIFLE, |_| true),
            None
        );
    }

    #[test]
    fn rejects_a_short_value_section() {
        let mut asset = table(&[("Handle", 4)], &[&[ANY], &[1]], None);
        // INFO's row count: say 3 rows.
        let at = asset
            .windows(6)
            .position(|w| w == [6, 0, 1, 0, 2, 0])
            .unwrap();
        asset[at + 4] = 3;
        assert!(matches!(
            AnimationTable::parse(&asset),
            Err(AnimationTableError::Values { .. })
        ));
    }

    /// `PickupWeapon`'s animation handle in the game's stance and action
    /// table.
    const PICKUP: u32 = 0x9e6d_9806;

    #[test]
    fn a_handle_gives_the_heros_own_row_whatever_is_held() {
        let t = on_foot();
        let hero = name_hash("Hero");
        let pick = |name: &str, class| {
            handle_clip(&t, PICKUP, (hero, name_hash(name)), class, |_| true)
                .map(|p| (p.clip, p.by))
        };
        let pistol = name_hash("pistol");
        use Precedence::*;
        // Chris's own row names no weapon: his, before anyone's.
        assert_eq!(pick("Chris", pistol), Some((0x4916_51da, OwnAny)));
        // Hers names the pistol.
        assert_eq!(pick("jennifer", pistol), Some((0x777b_d24e, Own)));
        // Mattias has no row of his own here: anyone's.
        assert_eq!(pick("mattias", pistol), Some((0x9874_62fc, Handle)));
        // A handle no row gives: nothing.
        assert_eq!(handle_clip(&t, 0x1234, (hero, 1), RIFLE, |_| true), None);
    }

    #[test]
    fn a_seats_own_idle_row_outranks_the_catch_all() {
        let stance = name_hash("InVehicle");
        let idle = name_hash("Idle");
        let seat = name_hash("seat_driver");
        let a_cols: Vec<(&str, u16)> = [
            "Stance",
            "Action",
            "AimState",
            "Tandem",
            "Seat",
            "Target",
            "ActionDirection",
            "DamageDirection",
            "AnimationHandles",
        ]
        .iter()
        .map(|&c| (c, 4))
        .collect();
        // As the game's: row 1 a catch-all, and every `InVehicle` `Idle`
        // row naming a seat.
        let a = table(
            &a_cols,
            &[
                &[ANY; 9],
                &[ANY, ANY, ANY, ANY, ANY, ANY, ANY, ANY, 0x9999],
                &[stance, idle, ANY, ANY, seat, ANY, ANY, ANY, 0x3333],
            ],
            None,
        );
        let m = (-1.0f32).to_bits();
        let b_cols: Vec<(&str, u16)> = [
            ("Handle", 4),
            ("VehicleClass", 4),
            ("VehicleName", 4),
            ("SeatName", 4),
            ("MountedEquipmentClass", 4),
            ("MountedEquipmentName", 4),
            ("Animation", 4),
            ("MinTimeScale", 3),
            ("MaxTimeScale", 3),
        ]
        .to_vec();
        let b = table(
            &b_cols,
            &[
                &[ANY; 9],
                &[ANY, ANY, ANY, ANY, ANY, ANY, 0x9d88_6544, m, m],
                &[0x3333, 0xc1, ANY, ANY, ANY, ANY, 0xcccc, m, m],
            ],
            None,
        );
        let (a, b) = (
            AnimationTable::parse(&a).unwrap(),
            AnimationTable::parse(&b).unwrap(),
        );
        let clip = |seat| seat_clip(&a, &b, "InVehicle", "Idle", (0xc1, 0x51), seat);
        // The driver's seat: its own row's handle, the class's clip.
        assert_eq!(clip(Some(seat)), Some(0xcccc));
        // No seat given, or one no row names: the catch-all.
        assert_eq!(clip(None), Some(0x9d88_6544));
        assert_eq!(clip(Some(0x0bad)), Some(0x9d88_6544));
        assert_eq!(action_handle(&a, "InVehicle", "Idle"), Some(0x9999));
    }
    #[test]
    fn a_rate_is_one_without_limits() {
        // -1 (most rows), 0, max below min, NaN: the clip plays at 1.0.
        assert_eq!(clip_rate(-1.0, -1.0, 7, 0x1234), 1.0);
        assert_eq!(clip_rate(0.0, 1.5, 7, 0x1234), 1.0);
        assert_eq!(clip_rate(1.2, 0.8, 7, 0x1234), 1.0);
        assert_eq!(clip_rate(f32::NAN, 1.5, 7, 0x1234), 1.0);
        // Equal limits: that rate, whoever plays it.
        assert_eq!(clip_rate(1.5, 1.5, 7, 0x1234), 1.5);
        assert_eq!(clip_rate(2.0, 2.0, 0, 0), 2.0);
    }

    #[test]
    fn a_rate_steps_through_the_range_by_id_and_clip() {
        // Each of the eight (id ^ clip) & 7 picks its own step of the
        // spread, min + (max - min) * step / 7.
        let clip = 0x0123_4560; // low 3 bits 0
        for (low, &step) in RATE_SPREAD.iter().enumerate() {
            let want =
                (f64::from(0.5f32) * f64::from(f32::from(step) * (1.0f32 / 7.0)) + 1.0) as f32;
            assert_eq!(clip_rate(1.0, 1.5, low as u32, clip), want, "id {low}");
        }
        assert_eq!(clip_rate(1.0, 1.5, 5, clip), 1.0); // step 0: min
        assert_eq!(clip_rate(1.0, 1.5, 2, clip), 1.5); // step 7: max
        assert_eq!(
            clip_rate(0.8, 1.2, 0, clip),
            (0.4f64 * f64::from(5.0f32 / 7.0) + 0.8) as f32
        );
        // Only the low 3 bits of id ^ clip count, from either side.
        assert_eq!(
            clip_rate(1.0, 1.5, 0x10, clip),
            clip_rate(1.0, 1.5, 0, clip)
        );
        assert_eq!(
            clip_rate(1.0, 1.5, 0, clip | 3),
            clip_rate(1.0, 1.5, 3, clip)
        );
        assert_eq!(
            clip_rate(1.0, 1.5, 3, clip | 3),
            clip_rate(1.0, 1.5, 0, clip)
        );
    }

    #[test]
    fn the_spread_is_each_step_once() {
        let mut s = RATE_SPREAD;
        s.sort_unstable();
        assert_eq!(s, [0, 1, 2, 3, 4, 5, 6, 7]);
    }

    #[test]
    fn a_clips_limits_come_from_the_first_row_listing_it() {
        let m = (-1.0f32).to_bits();
        let (lo, hi) = (1.0f32.to_bits(), 1.5f32.to_bits());
        let asto = [1, 0xaaaa, 2, 0xbbbb, 0xaaaa, 1, 0xcccc, 1, 0xdddd];
        let columns = [
            (HANDLE, 4),
            (ANIMATION, 8),
            (MIN_TIME_SCALE, 3),
            (MAX_TIME_SCALE, 3),
        ];
        let rows: [&[u32]; 5] = [
            &[ANY, 0, m, m],
            &[1, 0, lo, hi],
            &[2, 2, m, m],
            &[3, 5, m, m],
            &[4, 7, hi, hi],
        ];
        let t = AnimationTable::parse(&table(&columns, &rows, Some(&asto))).unwrap();
        assert_eq!(time_scales(&t, 1), Some((1.0, 1.5)));
        assert_eq!(time_scales(&t, 3), Some((-1.0, -1.0)));
        let s = clip_time_scales(&t);
        // aaaa: row 1 first (row 2 lists it again, without limits).
        assert_eq!(s.get(&0xaaaa), Some(&(1.0, 1.5)));
        // bbbb and cccc: rows without limits are left out.
        assert_eq!(s.get(&0xbbbb), None);
        assert_eq!(s.get(&0xcccc), None);
        assert_eq!(s.get(&0xdddd), Some(&(1.5, 1.5)));
        assert_eq!(s.len(), 2);
    }
}
