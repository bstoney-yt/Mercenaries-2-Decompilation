//! The scripts' reverb: presets `Sound.DefineReverbPreset` declares, the
//! one `Sound.SetReverbPreset` picks (by name, or by index on libraries
//! before version 10), and `Sound.SetReverb` turning it on or off. The
//! sound bootstrap does all three once, for the whole game (see
//! [`m2_engine::reverb`]).

use m2_engine::reverb::ReverbPreset;

use crate::api::Command;

/// The reverb as the scripts set it.
#[derive(Debug, Default)]
pub struct Reverb {
    /// Presets by the index they were defined at.
    presets: Vec<(i64, ReverbPreset)>,
    chosen: Option<i64>,
    on: bool,
    /// What the game was last told plays.
    playing: Option<ReverbPreset>,
}

/// How a script names a preset.
#[derive(Debug, Clone, PartialEq)]
pub enum PresetName {
    Index(i64),
    Name(String),
}

impl Reverb {
    /// Declares preset `index`.
    pub fn define(&mut self, index: i64, preset: ReverbPreset) -> Option<Command> {
        self.presets.retain(|(i, _)| *i != index);
        self.presets.push((index, preset));
        self.changed()
    }

    /// Picks the preset to play; false when none is named so.
    pub fn choose(&mut self, name: &PresetName) -> (bool, Option<Command>) {
        let found = self.presets.iter().find(|(i, p)| match name {
            PresetName::Index(n) => i == n,
            PresetName::Name(n) => p.name.eq_ignore_ascii_case(n),
        });
        match found.map(|(i, _)| *i) {
            Some(i) => {
                self.chosen = Some(i);
                (true, self.changed())
            }
            None => (false, None),
        }
    }

    pub fn set_on(&mut self, on: bool) -> Option<Command> {
        self.on = on;
        self.changed()
    }

    /// The preset playing now.
    pub fn playing(&self) -> Option<&ReverbPreset> {
        let i = self.chosen.filter(|_| self.on)?;
        self.presets.iter().find(|(n, _)| *n == i).map(|(_, p)| p)
    }

    fn changed(&mut self) -> Option<Command> {
        let now = self.playing().cloned();
        (now != self.playing).then(|| {
            self.playing.clone_from(&now);
            Command::Reverb(now)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bootstrap_turns_its_preset_on() {
        let mut r = Reverb::default();
        let p = ReverbPreset::city_light_reflections();
        assert_eq!(r.define(1, p.clone()), None);
        assert_eq!(r.choose(&PresetName::Name("nope".into())), (false, None));
        assert_eq!(
            r.choose(&PresetName::Name("CITY_KG_LIGHT_REFLECTIONS".into())),
            (true, None)
        );
        assert_eq!(r.choose(&PresetName::Index(1)), (true, None));
        assert_eq!(r.set_on(true), Some(Command::Reverb(Some(p.clone()))));
        assert_eq!(r.set_on(true), None);
        assert_eq!(r.playing(), Some(&p));
        assert_eq!(r.set_on(false), Some(Command::Reverb(None)));
    }
}
