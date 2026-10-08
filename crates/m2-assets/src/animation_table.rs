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
    /// `ASTO`'s words after its count.
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
        // The offset counts ASTO's leading word count (inferred from one
        // cell: offset 2 holds a one-entry list).
        let at = (at as usize).wrapping_sub(1);
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
            ("Seat", None),
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
    fn reads_lists() {
        let asset = table(
            &[("Handle", 4), ("Animation", 8)],
            &[&[ANY, ANY], &[0x10, 2]],
            Some(&[0, 1, 0x9d88_6544]),
        );
        let t = AnimationTable::parse(&asset).unwrap();
        assert_eq!(t.list(1, 1), &[0x9d88_6544]);
        assert_eq!(t.list(0, 1), &[] as &[u32]);
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
}
