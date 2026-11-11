//! The scripts' network calls that were missing, as the 360 answers them
//! with no network session (gap list).
//!
//! From the 360 code: each of these tests the session flag 0x82de698c
//! (several also 0x82de6990, the server flag) before reading its
//! arguments or sending anything, and with it clear it returns at once:
//! the `SendEvent_*` messages, the tray, fanfare, movie, blip and
//! dangerous-building events, and the mission spiel calls return true;
//! the connection and lobby questions return false. A few return a fixed
//! answer whatever the session (`IsDedicated`, `AutoLobby`, `AutoClient`
//! false; `SendEvent_AddSupportItem`, `SendEvent_RemoveSupportItem`,
//! `SendEvent_EnableHeroWeapons` true). That 0x82de698c means "a network
//! session is running" is inferred from which calls test it.
//!
//! Left out: the lobby and server calls that post a session message
//! whatever the flag (`EnterLobby`, `Stop`, `ConnectToServer`,
//! `StartServer`, the friends lobby, `KickPlayer`, the sign-in dialogs),
//! `SendEvent_TeleportPlayerToHardPoint` (reads hard points before any
//! test), `GetHostName`, `ShouldPlayOnline` (a setting) and
//! `ApplyCachedFactionRelations` (applies the faction cache).

use mlua::{Lua, MultiValue, Table, Value};

use crate::api::ScriptWorld;

type Handler = fn(&mut ScriptWorld, &Lua, &[Value]) -> mlua::Result<MultiValue>;

/// Calls that return true with no session (their 360 addresses).
pub const OFFLINE_TRUE: [(&str, u32); 36] = [
    ("SendEvent_ShowMessage", 0x8232_8cb8),
    ("SendEvent_TeleportPlayer", 0x8232_9bc8),
    ("SetLastHeroTeleportLocation", 0x8232_9ec0),
    ("SendEvent_Fanfare", 0x8232_9fb8),
    ("SendEvent_CloseFanfare", 0x8232_a450),
    ("SendEvent_Support", 0x8232_a5f8),
    ("SendEvent_AddSupportItem", 0x8232_a878),
    ("SendEvent_RemoveSupportItem", 0x8232_a8d8),
    ("SendEvent_RevivePlayer", 0x8232_a938),
    ("SendEvent_RequestPosition", 0x8232_aa60),
    ("SendEvent_SetObjectiveTraySlotText", 0x8232_aaa8),
    ("SendEvent_SetObjectiveTraySlotImage", 0x8232_ac18),
    ("SendEvent_ClearObjectiveTraySlot", 0x8232_add0),
    ("SendEvent_ShowMovie", 0x8232_af00),
    ("SendEvent_HideMovie", 0x8232_aff8),
    ("SendEvent_EnableHeroWeapons", 0x8232_b278),
    ("SendEvent_AddDangerousBuilding", 0x8232_b2d8),
    ("SendEvent_RemoveDangerousBuilding", 0x8232_b3c0),
    ("SendEvent_SetOccupiedDangerousBuilding", 0x8232_b500),
    ("SendEvent_AddRandomDangerousBuilding", 0x8232_b620),
    ("SendEvent_TextFanfare", 0x8232_b740),
    ("SendEvent_CardFanfare", 0x8232_b888),
    ("SendEvent_HVTFanfare", 0x8232_ba58),
    ("SendEvent_UnlockFanfare", 0x8232_bbc8),
    ("SendEvent_BatchUnlockFanfare", 0x8232_bd48),
    ("SendEvent_ForceClientTether", 0x8232_bf58),
    ("SendEvent_RemovePDAMission", 0x8232_c510),
    ("SendEvent_AddHqPdaBlip", 0x8232_c630),
    ("SendEvent_RemoveHqPdaBlip", 0x8232_c8c8),
    ("SendEvent_AddPmcPdaBlip", 0x8232_c9c8),
    ("SendEvent_RemovePmcPdaBlip", 0x8232_cbd0),
    ("LoadMissionSpiel", 0x8232_d370),
    ("UnloadMissionSpiel", 0x8232_d508),
    ("SetBriefingStarters", 0x8232_d6e0),
    ("SetShootingGalleryBorder", 0x8232_dbe0),
    ("IsReadyToTether", 0x8232_e120),
];

/// Calls that return false with no session.
pub const OFFLINE_FALSE: [(&str, u32); 10] = [
    ("IsPlatformConnected", 0x8232_8258),
    ("IsConnectedToInternet", 0x8232_82b0),
    ("IsEnabled", 0x8232_8360),
    ("IsLobby", 0x8232_8418),
    ("IsDedicated", 0x8232_8538),
    ("AutoLobby", 0x8232_8598),
    ("AutoClient", 0x8232_85f8),
    ("AutoServer", 0x8232_8658),
    ("UpdatePresence", 0x8232_de90),
    ("SendEvent_JoinPOForceRequest", 0x8232_df60),
];

pub(crate) fn install(
    lib: &dyn Fn(&str) -> mlua::Result<Table>,
    add: &dyn Fn(&Table, &str, Handler) -> mlua::Result<()>,
) -> mlua::Result<()> {
    let net = lib("Net")?;
    for (name, _) in OFFLINE_TRUE {
        add(&net, name, |_, _, _| {
            Ok(MultiValue::from_vec(vec![Value::Boolean(true)]))
        })?;
    }
    for (name, _) in OFFLINE_FALSE {
        add(&net, name, |_, _, _| {
            Ok(MultiValue::from_vec(vec![Value::Boolean(false)]))
        })?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::{Object, Shared, install};
    use crate::script::ScriptHost;

    #[test]
    fn network_calls_answer_as_offline_on_the_360() {
        let host = ScriptHost::new().unwrap();
        let world = Shared::default();
        {
            let mut w = world.borrow_mut();
            w.insert(1, Object::new("Mattias", [0.0; 3]));
            w.player = Some(1);
        }
        install(host.lua(), &world).unwrap();
        let lua = host.lua();
        let answer = |name: &str| -> Value {
            lua.load(format!(
                "return Net.{name}(Player.GetLocalPlayer(), 1, 'x')"
            ))
            .eval()
            .unwrap()
        };
        for (name, _) in OFFLINE_TRUE {
            assert_eq!(answer(name), Value::Boolean(true), "{name}");
        }
        for (name, _) in OFFLINE_FALSE {
            assert_eq!(answer(name), Value::Boolean(false), "{name}");
        }
    }
}
