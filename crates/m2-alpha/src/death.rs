//! Her death with the game's scripts running: the scripts' own death flow
//! (`MrxPlayer.PlayerDied`), not the alpha's stand-up.
//!
//! From the scripts (`MrxPlayer`, script 8d35cf13 in `vz.wad`; the hero
//! module `Hero`, 51728909):
//!
//! 1. While her hero module's health clamp is on (`Player.SetHealthClamp`)
//!    she cannot die: at the floor its survival rescue runs instead (the
//!    port's [`crate::combat::Combat::health_clamp`]). With the clamp off
//!    (after the one rescue, until it cools down) her health can reach
//!    nothing, and the game kills her object: the `ObjectDeath` event
//!    `MrxPlayer.OnPlayerJoined` set on her runs `PlayerDied`.
//! 2. `PlayerDied` (single player: no other hero alive) counts the death
//!    (`MrxStatsManager.IncreaseDeathCounter`) and, with a contract under
//!    way, only cancels it (`mission:Cancel()`): the contract's fail
//!    fanfare then waits for her answer, and its end (`MrxTaskContract`,
//!    2f0b1589, from its line 560) loads the retry save on Retry, else,
//!    her character being dead, calls `MrxPlayer.MoveToSickbay`. With no
//!    contract, 2 s on it calls `_DialogBoxDismissed(1)`, which on the
//!    island (`vz`) calls `MoveToSickbay`, and clears her GPS. No dialog box and no text: the "You Have Died"
//!    box (`_DisplayOnDeathDialogBox`) is not on this path. In co-op with
//!    a hero alive her partner gets `[ContextAction.Revive]` instead, with
//!    Fiona's nags every 30 s and a sickbay after 60 s.
//! 3. `MoveToSickbay` (unless she is in the PMC's interior already) stops
//!    the voice sequences and lines and enters `STATE_WAITFORGAME` with
//!    `_MoveToSickbayBegin`: out of any vehicle (`Vehicle.Exit`), grapple
//!    let go, what she carries dropped, her weapons set to Pistol, Grenade
//!    and C4 (`Human.Inventory.SetAllWeapons`, as she is dead), then
//!    `WifPmcInterior.Enter` (the loading screen) and, once streaming is
//!    done, `_CompleteMove`: she stands at "Sickbay Hero Respawn 1", the
//!    camera turned to 80 degrees, `Hero.EndSurvivalMode`, and
//!    `RiseFromYourGrave` revives her (`Object.Revive`, full health);
//!    `MedevacComplete` is posted. Leaving the state
//!    (`_MoveToSickbayEnd`) takes `GetMedEvacCost` ($10,000) off her cash
//!    under `[Generic.Medevacs]`.
//!
//! From the 360 code, step 1's engine side: the health change
//! (0x822889F8) takes the hit from her health and, once it is under
//! [`DEATH_BELOW`] (0x820B7AE0, 0.01), posts her object's death on the
//! object-death queue 0x831AD128 (0x8217D8A0, naming the attacker) and
//! marks it dead; 0x822885B0 can hold that back only for a vehicle (body
//! kind 4) and its riders. The death itself runs in 0x82289F40 (her
//! object's death spawns, out of her seat, its death event to the
//! scripts on 0x831B21C0), which is what fires the scripts' `ObjectDeath`.
//!
//! The port runs those scripts as they are; the alpha's part is step 1's
//! engine side: her death goes to the scripts ([`crate::combat::Combat::died`]
//! to `m2_game::Mission::kill` of the player), she stays down
//! ([`down_time`]) and gets up when their `Object.Revive` comes
//! ([`crate::combat::Combat::revive_hp`]), where their move put her.
//!
//! Invented: without the scripts the alpha's stand-up stays (2.5 s, full
//! health, where she fell); with them, should no revive come
//! ([`SCRIPTS_WAIT`]: the PMC interior's own path, a script error) she
//! gets up after 30 s as she did before.

/// Her health under which she is dead: the 360's health change kills an
/// object whose health falls under this (0x822889F8, its 0.01 at
/// 0x820B7AE0), not only at nothing.
pub const DEATH_BELOW: f32 = 0.01;

/// Seconds she lies down without the scripts before the alpha stands her
/// up again (Invented).
pub const DOWN_TIME: f32 = 2.5;

/// Seconds she waits for the scripts' revive before the alpha stands her
/// up itself (Invented fallback; their own flow takes about 3 s plus the
/// sickbay's streaming).
pub const SCRIPTS_WAIT: f32 = 30.0;

/// How long she stays down at a death: for the scripts' revive when they
/// run (`scripted`), else the alpha's own stand-up.
pub fn down_time(scripted: bool) -> f32 {
    if scripted { SCRIPTS_WAIT } else { DOWN_TIME }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn with_the_scripts_she_waits_for_them() {
        assert_eq!(down_time(false), 2.5);
        assert_eq!(down_time(true), 30.0);
        assert!(
            down_time(true) > 2.0 + 1.0,
            "past the scripts' own 2 s delay"
        );
    }
}
