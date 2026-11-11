//! `Junk.FormatTime` and `Junk.IsInstallable`.
//!
//! From the 360 code: `Junk.FormatTime(seconds [, tenths])` (`0x82327058`)
//! reads a number (nil without one) and an optional boolean, then
//! builds two strings with the C formats at `0x820D5D28`..`0x820D5D44`:
//! `"%d:%02d:"` (hours, minutes) when there are hours, else `"%d:"`
//! (minutes), then `"%02d"` (seconds) or, with tenths, `"%02d.%1.0f"`.
//! The hours, minutes and plain seconds come from the time rounded to a
//! whole second (`floor(t + 0.5)`); the tenths form takes its seconds
//! from `floor(t)` instead, so 59.6 s reads "1:59.6", and a fraction of
//! .95 or more prints as "10" ("0:00.10"). Hours stop at 9999. All of
//! this is kept.
//!
//! `Junk.IsInstallable` (`0x823272B0`) always answers false.

use mlua::{Lua, MultiValue, Result, Value};

/// Lua's `tonumber` for a native's argument.
fn number(v: Option<&Value>) -> Option<f32> {
    match v? {
        Value::Integer(i) => Some(*i as f32),
        Value::Number(n) => Some(*n as f32),
        Value::String(s) => s
            .to_str()
            .ok()?
            .trim()
            .parse::<f64>()
            .ok()
            .map(|n| n as f32),
        _ => None,
    }
}

/// Format time in HH:MM:SS or H:MM:SS or MM:SS or M:SS format, with optional tenths.
pub fn format_time(secs: f32, tenths: bool) -> String {
    let total = (secs + 0.5_f32).floor() as i32;
    let minutes = total / 60;
    let mut hours = minutes / 60;
    if hours > 9999 {
        hours = 9999;
    }

    // First part
    let first = if hours > 0 {
        format!("{hours}:{:02}:", minutes % 60)
    } else {
        format!("{}:", minutes % 60)
    };

    // Second part
    let second = if tenths {
        let whole = secs.floor() as i32;
        let frac = (secs - secs.floor()) * 10.0_f32;
        format!("{:02}.{:.0}", whole % 60, f64::from(frac))
    } else {
        format!("{:02}", total % 60)
    };

    format!("{first}{second}")
}

/// Native Lua function: `format_time(secs, tenths?)`
pub fn format_time_native(lua: &Lua, args: &[Value]) -> Result<MultiValue> {
    let secs = number(args.first());
    let tenths = match args.get(1) {
        Some(Value::Boolean(b)) => *b,
        _ => false,
    };

    let secs = match secs {
        Some(s) => s,
        None => return Ok(MultiValue::from_vec(vec![Value::Nil])),
    };

    let formatted = format_time(secs, tenths);
    let s = lua.create_string(&formatted)?;
    Ok(MultiValue::from_vec(vec![Value::String(s)]))
}

/// Native Lua function: `is_installable()`
pub fn is_installable() -> MultiValue {
    MultiValue::from_vec(vec![Value::Boolean(false)])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_time() {
        assert_eq!(format_time(0.0, false), "0:00");
        assert_eq!(format_time(59.4, false), "0:59");
        assert_eq!(format_time(59.5, false), "1:00");
        assert_eq!(format_time(3725.0, false), "1:02:05");
        assert_eq!(format_time(59.6, true), "1:59.6");
        assert_eq!(format_time(0.96, true), "0:00.10");
        assert_eq!(format_time(36000000.0, false), "9999:00:00");
        assert_eq!(format_time(-5.0, false), "0:-5");
    }

    #[test]
    fn test_native() {
        let lua = Lua::new();

        // format_time_native tests
        // no args -> Nil
        let result = format_time_native(&lua, &[]).unwrap();
        assert_eq!(result.len(), 1);
        assert!(matches!(result[0], Value::Nil));

        // ("x") -> Nil (parse fails)
        let args = vec![Value::String(lua.create_string("x").unwrap())];
        let result = format_time_native(&lua, &args).unwrap();
        assert_eq!(result.len(), 1);
        assert!(matches!(result[0], Value::Nil));

        // (Number(65.0)) -> "1:05"
        let args = vec![Value::Number(65.0)];
        let result = format_time_native(&lua, &args).unwrap();
        assert_eq!(result.len(), 1);
        if let Value::String(s) = &result[0] {
            assert_eq!(s.to_str().unwrap(), "1:05");
        } else {
            panic!("expected String");
        }

        // is_installable
        let result = is_installable();
        assert_eq!(result.len(), 1);
        if let Value::Boolean(b) = result[0] {
            assert!(!b);
        } else {
            panic!("expected Boolean");
        }
    }
}
