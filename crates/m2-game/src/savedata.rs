//! The scripts' own save data: what the bootstrap's `GenerateSaveData`
//! gathers from every module's `SaveSingleton` (the factions' relations,
//! the contracts done, her stockpile...), as Lua text the game can keep
//! in its save file and read back with Lua's own `load`. Guids (the
//! engine's userdata) are written as `Sys.StringToGuid("0x...")`;
//! functions and anything else that can't be written, and tables met a
//! second time on the way down, are left out.
//!
//! The engine's own save format is not followed (it isn't read); the data
//! is the scripts'. Loading it back into the scripts
//! (`LoadSingleton(data)` on a load instead of a new game) is not done
//! yet.

use mlua::Lua;

/// Lua that writes a value as Lua text (`__m2_save_text(v)`).
const WRITER: &str = r#"
local function write(v, seen, out)
    local t = type(v)
    if t == "number" then
        if v ~= v then out[#out + 1] = "0/0"
        elseif v == math.huge then out[#out + 1] = "math.huge"
        elseif v == -math.huge then out[#out + 1] = "-math.huge"
        else out[#out + 1] = string.format("%.17g", v) end
    elseif t == "string" then
        out[#out + 1] = string.format("%q", v)
    elseif t == "boolean" then
        out[#out + 1] = tostring(v)
    elseif t == "userdata" then
        local hex = string.match(tostring(v), "(%x+)$")
        out[#out + 1] = hex and ('Sys.StringToGuid("0x' .. hex .. '")') or "nil"
    elseif t == "table" and not seen[v] then
        seen[v] = true
        out[#out + 1] = "{"
        for k, x in pairs(v) do
            local tk, tx = type(k), type(x)
            if (tk == "string" or tk == "number" or tk == "boolean")
                and tx ~= "function" and tx ~= "thread" then
                out[#out + 1] = "["
                write(k, seen, out)
                out[#out + 1] = "]="
                write(x, seen, out)
                out[#out + 1] = ","
            end
        end
        out[#out + 1] = "}"
        seen[v] = nil
    else
        out[#out + 1] = "nil"
    end
end
function __m2_save_text(v)
    local out = {}
    write(v, {}, out)
    return table.concat(out)
end
"#;

/// The scripts' save data now (`Scripts[bootstrap].GenerateSaveData()`),
/// as Lua text: `return` it to have it back.
pub fn generate(lua: &Lua, bootstrap: u32) -> mlua::Result<String> {
    lua.load(WRITER).set_name("=m2-save writer").exec()?;
    lua.load(format!(
        "return __m2_save_text(Scripts[\"{bootstrap:08x}\"].GenerateSaveData())"
    ))
    .set_name("=m2-save")
    .eval()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn save_data_written_reads_back_the_same() {
        let lua = Lua::new();
        lua.load(
            r#"
            Sys = { StringToGuid = function(s) return "guid " .. tonumber(s) end }
            Scripts = { ["00000001"] = { GenerateSaveData = function()
                local t = { nCash = 25000, sName = "Line one\nline \"two\"",
                    tList = { 1, 2.5, true }, fCallback = print, [7] = -0.125 }
                t.tSelf = t
                return t
            end } }
            "#,
        )
        .exec()
        .unwrap();
        let text = generate(&lua, 1).unwrap();
        lua.globals().set("__text", text).unwrap();
        let (cash, name, second, seven, self_, f): (i64, String, f64, f64, bool, bool) = lua
            .load(
                "local t = loadstring('return ' .. __text)() \
                 return t.nCash, t.sName, t.tList[2], t[7], t.tSelf == nil, t.fCallback == nil",
            )
            .eval()
            .unwrap();
        assert_eq!(
            (cash, name.as_str(), second, seven, self_, f),
            (25000, "Line one\nline \"two\"", 2.5, -0.125, true, true)
        );
    }
}
