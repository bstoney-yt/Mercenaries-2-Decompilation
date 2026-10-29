//! The scripts' own save data: what the bootstrap's `GenerateSaveData`
//! gathers from every module's `SaveSingleton` (the factions' relations,
//! the contracts done, her stockpile...), as Lua text the game can keep
//! in its save file and read back with Lua's own `load`. Guids (the
//! engine's userdata) are written as `Sys.StringToGuid("0x...")`;
//! functions and anything else that can't be written, and tables met a
//! second time on the way down, are left out.
//!
//! The engine's own save format is not followed (it isn't read); the data
//! is the scripts'. Loading it hands it back to the bootstrap
//! (`LoadSingleton(data)` instead of a new game's `LoadSingleton(nil)`:
//! [`Mission::load_save`](crate::mission::Mission::load_save)), which
//! gives each module its part as the game does.

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

/// Lua that says where the story stands: `WifMissionFlow.GetMissionStates`
/// (each contract `"complete"` or `"incomplete"`, by whether the flow
/// holds its key, then the one playing as `"active"`) and the contracts
/// the flow has unlocked (`tActiveMissions` of its `SaveSingleton`, as a
/// save keeps them), each list sorted. From the scripts' bytecode.
///
/// `SaveSingleton` walks the flow's `_tActiveMissions`, which only its
/// `Reset` creates; the game calls it at save time, after the bootstrap
/// has reset the flow, but autoplay asks from the first frame the scripts
/// run. Until then nothing is unlocked, so nothing is on offer.
pub const CONTRACTS: &str = r#"
local done, active, offered = {}, {}, {}
for _, s in ipairs(WifMissionFlow.GetMissionStates() or {}) do
    if s[2] == "complete" then done[#done + 1] = s[1]
    elseif s[2] == "active" then active[#active + 1] = s[1] end
end
local flow = {}
if getfenv(WifMissionFlow.SaveSingleton)._tActiveMissions ~= nil then
    flow = WifMissionFlow.SaveSingleton() or {}
end
for k in pairs(flow.tActiveMissions or {}) do
    offered[#offered + 1] = tostring(k)
end
table.sort(done) table.sort(active) table.sort(offered)
return done, active, offered
"#;

/// Where the story stands ([`CONTRACTS`]).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Contracts {
    pub done: Vec<String>,
    pub active: Vec<String>,
    pub offered: Vec<String>,
}

impl std::fmt::Display for Contracts {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} done [{}], active [{}], on offer [{}]",
            self.done.len(),
            self.done.join(" "),
            self.active.join(" "),
            self.offered.join(" ")
        )
    }
}

/// Where the story stands now ([`CONTRACTS`]).
pub fn contracts(lua: &Lua) -> mlua::Result<Contracts> {
    let (done, active, offered) = lua.load(CONTRACTS).set_name("=m2-contracts").eval()?;
    Ok(Contracts {
        done,
        active,
        offered,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contracts_are_read_from_the_flows_states_and_unlocks() {
        let lua = Lua::new();
        lua.load(
            r#"
            _tActiveMissions = {}
            WifMissionFlow = {
                SaveSingleton = function()
                    return { tActiveMissions = { PmcCon001 = {}, AllCon001 = {} } }
                end,
                GetMissionStates = function()
                    return { { "VzaCon001", "complete" }, { "PmcCon001", "incomplete" },
                        { "AllCon001", "incomplete" }, { "PmcCon001", "active" } }
                end,
            }
            "#,
        )
        .exec()
        .unwrap();
        let c = contracts(&lua).unwrap();
        assert_eq!(
            c,
            Contracts {
                done: vec!["VzaCon001".into()],
                active: vec!["PmcCon001".into()],
                offered: vec!["AllCon001".into(), "PmcCon001".into()],
            }
        );
        assert_eq!(
            c.to_string(),
            "1 done [VzaCon001], active [PmcCon001], on offer [AllCon001 PmcCon001]"
        );
    }

    #[test]
    fn contracts_before_the_flow_is_reset_offer_nothing() {
        let lua = Lua::new();
        lua.load(
            r#"
            WifMissionFlow = {
                SaveSingleton = function()
                    local t = {}
                    for k in pairs(_tActiveMissions) do t[k] = true end
                    return { tActiveMissions = t }
                end,
                GetMissionStates = function()
                    return { { "VzaCon001", "complete" }, { "PmcCon001", "active" } }
                end,
            }
            "#,
        )
        .exec()
        .unwrap();
        let c = contracts(&lua).unwrap();
        assert_eq!(
            c,
            Contracts {
                done: vec!["VzaCon001".into()],
                active: vec!["PmcCon001".into()],
                offered: vec![],
            }
        );
        lua.load("_tActiveMissions = { OilCon020 = {} }")
            .exec()
            .unwrap();
        assert_eq!(
            contracts(&lua).unwrap().offered,
            vec!["OilCon020".to_string()]
        );
    }

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
