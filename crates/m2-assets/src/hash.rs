//! The hash the archives key names by: assets by name, string tables by
//! text id, scripts by module name.
//!
//! It is 32-bit FNV-1a (offset `0x811c9dc5`, prime `0x01000193`) over the
//! name's bytes each with bit 5 set (a cheap lower-casing: `A` becomes
//! `a`, but `_` becomes `0x7f`), with a `*` appended: `name_hash("english")`
//! is the English string table's `b6a13123`, `name_hash("MrxTask")` the
//! script module's `d302a66f`, `name_hash("shell_disc_00000")` that
//! texture's `a6b7bb36`, and `name_hash("objective.Objectivecompleted")`
//! the key `923171d8` of "Objective completed:".
//!
//! # Provenance
//!
//! Found from the names and hashes of a legally installed copy (13,846
//! named assets, the 18,299 keys of its string tables, and the modules the
//! scripts load): the hashes of names that differ in one character were
//! solved bit by bit for the per-byte step, which came out as FNV-1a's,
//! with a constant trailing byte; the names that then still missed were
//! those with an underscore, and one byte value for it fits them all. No
//! code from the game was consulted.

/// FNV-1a's offset basis.
const OFFSET: u32 = 0x811c_9dc5;
/// FNV-1a's 32-bit prime.
const PRIME: u32 = 0x0100_0193;
/// The byte the game appends to every name before hashing.
const TAIL: u8 = b'*';
/// Set on every byte: the game's lower-casing.
const CASE: u8 = 0x20;

/// The hash of a name, text id or module name as the archives key it.
pub fn name_hash(name: &str) -> u32 {
    name.bytes()
        .map(|b| b | CASE)
        .chain([TAIL])
        .fold(OFFSET, |h, b| (h ^ u32::from(b)).wrapping_mul(PRIME))
}

#[cfg(test)]
mod tests {
    use super::*;

    // Hashes read from the archives of a legally installed copy.
    #[test]
    fn names_hash_as_the_archives_key_them() {
        // The three string tables.
        assert_eq!(name_hash("english"), 0xb6a1_3123);
        assert_eq!(name_hash("japanese"), 0x2bff_a040);
        assert_eq!(name_hash("allcaps"), 0x9d17_fa7d);
        // Script modules, whatever case the scripts load them in.
        assert_eq!(name_hash("MrxTask"), 0xd302_a66f);
        assert_eq!(name_hash("mrxtask"), 0xd302_a66f);
        assert_eq!(name_hash("VzaCon001"), 0x650b_5d57);
        // A text id.
        assert_eq!(name_hash("objective.Objectivecompleted"), 0x9231_71d8);
        // Assets: without and with underscores.
        assert_eq!(name_hash("dbgrest"), 0xcf56_7180);
        assert_eq!(name_hash("dbgmountedgun"), 0xf8fe_4910);
        assert_eq!(name_hash("shell_disc_00000"), 0xa6b7_bb36);
        assert_eq!(name_hash("SHELL_EXPLOSION_00001"), 0x6b87_925d);
        assert_eq!(name_hash("pause_menu_i100"), 0x6c2b_3b9e);
        assert_eq!(
            name_hash("vz_terrainglobal_r00_c00_0x0003fcd3"),
            0x542c_7583
        );
        // Entity layers and a texture with a localised copy.
        assert_eq!(name_hash("vz_state_pmccon005"), 0xe257_f59f);
        assert_eq!(name_hash("vz_state_gurcon003"), 0x546a_7bfb);
        assert_eq!(name_hash("pda_titles"), 0xb042_41e4);
    }
}
