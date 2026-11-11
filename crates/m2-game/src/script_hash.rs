//! The engine's string hash, and the natives that answer it:
//! `String.GetHash` and `ObjectState.GetStringHash`.
//!
//! From the 360 code: both natives read a string (`0x82308518`), hash it
//! with `0x82855718` and push the result with `0x827DFAF8`, which stores
//! the 32-bit value with Lua type tag 2: light userdata, the same push as
//! guids (`0x8222C480`, shared with `Object.GetName`/`GetModelName`). So
//! a hash is userdata that equals a guid of the same value: a faction's
//! guid is the hash of its name, and scripts key tables by either.
//!
//! The hash is [`m2_assets::name_hash`]'s FNV-1a with bit 5 set on each
//! byte and `*` folded in last, with two details only the code shows:
//! the empty string (or a null pointer) hashes to 0, and each byte is
//! sign-extended before bit 5 is set, so bytes from 0x80 up fold in as
//! `0xFFFFFFxx`. The two agree on ASCII.

use mlua::{LightUserData, MultiValue, Value};
use std::ffi::c_void;

/// `0x82855718`: the engine's hash of a string, up to its first 0 byte.
pub fn engine_hash(bytes: &[u8]) -> u32 {
    if bytes.is_empty() || bytes[0] == 0 {
        return 0;
    }

    let mut h: u32 = 0x811C9DC5;
    for &b in bytes {
        if b == 0 {
            break;
        }
        // sign-extend byte to i32, cast to u32, then OR 0x20
        let c = ((b as i8) as i32 as u32) | 0x20;
        h = (h ^ c).wrapping_mul(0x0100_0193);
    }
    // final mixing step
    h = (h ^ 0x2A).wrapping_mul(0x0100_0193);
    h
}

/// A hash as the scripts see it: light userdata, as a guid.
pub fn hash_value(h: u32) -> Value {
    let ptr = (h as usize) as *mut c_void;
    Value::LightUserData(LightUserData(ptr))
}

/// `String.GetHash(s)` / `ObjectState.GetStringHash(s)`: nil unless `s`
/// is a string.
pub fn get_hash_native(args: &[Value]) -> MultiValue {
    if let Some(Value::String(s)) = args.first() {
        let b = s.as_bytes();
        let h = engine_hash(&b[..]);
        MultiValue::from_vec(vec![hash_value(h)])
    } else {
        MultiValue::from_vec(vec![Value::Nil])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mlua::Lua;

    #[test]
    fn test_engine_hash_ascii_matches_name_hash() {
        let cases = [
            ("english", 0xB6A13123),
            ("MrxTask", 0xD302A66F),
            ("shell_disc_00000", 0xA6B7BB36),
            ("objective.Objectivecompleted", 0x923171D8),
        ];

        for (name, expected) in cases {
            let h_engine = engine_hash(name.as_bytes());
            let h_name = m2_assets::name_hash(name);
            assert_eq!(h_engine, expected, "engine_hash({:?})", name);
            assert_eq!(h_name, expected, "name_hash({:?})", name);
            assert_eq!(h_engine, h_name, "engine vs name_hash for {:?}", name);
        }
    }

    #[test]
    fn test_engine_hash_empty_and_zero_prefix() {
        assert_eq!(engine_hash(b""), 0);
        assert_eq!(engine_hash(b"\0abc"), 0);
        assert_eq!(engine_hash(b"ab\0cd"), engine_hash(b"ab"));
    }

    #[test]
    fn test_engine_hash_high_bytes() {
        // Test byte >= 0x80 with sign extension vs zero extension
        let bytes = [b'a', 0xE9]; // 'a' = 0x61, 0xE9 = 233 = 0b11101001

        // Sign-extended version (engine_hash rule):
        // b = 0xE9 -> i8 = -23 -> i32 = -23 -> u32 = 0xFFFFFFE9
        // c = 0xFFFFFFE9 | 0x20 = 0xFFFFFFE9 | 0x20 = 0xFFFFFFEB
        // Then engine_hash:
        let mut h: u32 = 0x811C9DC5;
        for &b in &bytes {
            let c = ((b as i8) as i32 as u32) | 0x20;
            h = (h ^ c).wrapping_mul(0x0100_0193);
        }
        h = (h ^ 0x2A).wrapping_mul(0x0100_0193);
        let expected_engine = h;

        // Zero-extended version (m2_assets::name_hash style):
        // c_zero = (u32::from(b) | 0x20) for each byte
        let mut h_zero: u32 = 0x811C9DC5;
        for &b in &bytes {
            let c = u32::from(b) | 0x20;
            h_zero = (h_zero ^ c).wrapping_mul(0x0100_0193);
        }
        h_zero = (h_zero ^ 0x2A).wrapping_mul(0x0100_0193);
        let expected_zero = h_zero;

        assert_eq!(engine_hash(&bytes[..]), expected_engine);
        assert_ne!(
            expected_engine, expected_zero,
            "engine_hash differs from zero-extended fold"
        );
    }

    #[test]
    fn test_get_hash_native() {
        let lua = Lua::new();

        // String argument
        let s = lua.create_string("english").unwrap();
        let args = vec![Value::String(s)];
        let mv = get_hash_native(&args);
        assert_eq!(mv.len(), 1);
        match &mv[0] {
            Value::LightUserData(ud) => {
                let h = (ud.0 as usize) as u32;
                assert_eq!(h, engine_hash(b"english"));
                assert_eq!(h, 0xB6A13123);
            }
            other => panic!("Expected LightUserData, got {:?}", other),
        }

        // Number argument
        let args = vec![Value::Number(1.0)];
        let mv = get_hash_native(&args);
        assert_eq!(mv.len(), 1);
        assert_eq!(mv[0], Value::Nil);
    }
}
