//! `--shop-check`: every item of the scripts' support catalogue bought in
//! play and called in, one after another, as a player would: cash given
//! her first (as if picked up), the item bought in the first faction's
//! shop that sells it (everything unlocked: the shop cheat), chosen in
//! the support menu and called in, its designator thrown, lased or
//! confirmed through the satellite, and then what comes of it counted: a
//! helicopter at its drop point for a delivery (a crate, a vehicle, a
//! boat, a helicopter landing), an aircraft over the mark or ordnance
//! going off for a strike. For testing; every step is logged as
//! `shop-check:` lines, and a summary at the end.
//!
//! She is kept at full health meanwhile, and the support's recruits are
//! freed before each item ([`MissionLink::free_recruits`]): in play the
//! helicopter pilot is busy until his last helicopter is gone, and each
//! pilot rests a while between calls.

use crate::game::Game;
use crate::mission::{MissionLink, SHOPS, SupportCounts};
use crate::player::Intent;
use crate::support::{Aim, Entry, Phase};

/// She stands ashore this long before the check starts (seconds), for
/// the scripts' layers to be in.
const SETTLE: f32 = 10.0;
/// Fuel given her with each item's cash: more than any item costs (180).
const FUEL: u32 = 300;
/// The cash has this long to reach the scripts, the bought item this
/// long to reach the support menu, and the call this long to bring
/// something (seconds).
const PAY_WAIT: f32 = 3.0;
const STOCK_WAIT: f32 = 3.0;
const CALL_WAIT: f32 = 60.0;
/// After something came, a few seconds for the rest of it (seconds).
const LINGER: f32 = 3.0;

#[derive(Clone, Copy, Debug, PartialEq)]
enum Stage {
    /// Waiting for her to stand ashore.
    Settling(f32),
    /// Cash given; buying once the scripts have it.
    Paid(f32),
    /// Bought; waiting for it in the menu.
    Stocking(f32),
    /// The support menu opened; choosing it next tick.
    Menu,
    /// Called in: the designator, then what comes (seconds since).
    Calling(f32),
    /// Something came: a little longer (seconds left).
    Lingering(f32),
}

/// What became of one item.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Outcome {
    pub key: String,
    /// A strike, not a delivery.
    pub strike: bool,
    /// The shop that sold it, its price, and her cash before and after.
    pub shop: Option<&'static str>,
    pub price: f64,
    pub cash: (f64, f64),
    pub bought: bool,
    pub called: bool,
    /// The designator the scripts handed her: "thrown", or the aimed
    /// one's template.
    pub designator: Option<String>,
    /// What came of it, from its designator coming down.
    pub came: SupportCounts,
    pub seconds: f32,
}

impl Outcome {
    /// Bought with its price taken.
    pub fn paid(&self) -> bool {
        self.bought && self.cash.0 - self.cash.1 >= self.price - 0.5
    }

    /// What it is for came: a strike's aircraft or ordnance, a delivery's
    /// helicopter.
    pub fn came(&self) -> bool {
        if self.strike {
            self.came.flown > 0 || self.came.burst > 0
        } else {
            self.came.arrived > 0
        }
    }

    pub fn delivered(&self) -> bool {
        self.paid() && self.came()
    }
}

pub struct ShopCheck {
    /// The items asked for (all, if none), and those to try: key, and
    /// whether a strike.
    asked: Vec<String>,
    keys: Vec<(String, bool)>,
    next: usize,
    stage: Stage,
    now: Outcome,
    /// What had come when its designator came down, once it has.
    marked: Option<SupportCounts>,
    /// Fire pressed last tick (presses alternate).
    tapped: bool,
    pub done: Vec<Outcome>,
}

impl ShopCheck {
    /// A check of the items `asked` for, or of the whole catalogue.
    pub fn new(asked: Vec<String>) -> Self {
        Self {
            asked,
            keys: Vec::new(),
            next: 0,
            stage: Stage::Settling(0.0),
            now: Outcome::default(),
            marked: None,
            tapped: false,
            done: Vec::new(),
        }
    }

    /// Every item has been tried.
    pub fn finished(&self) -> bool {
        !self.keys.is_empty() && self.next >= self.keys.len() && self.now.key.is_empty()
    }

    /// One tick: what she does (the rest of the time she stands). `None`
    /// until she has stood ashore a while: until then she plays as she
    /// would (`--autoplay` takes her ashore).
    pub fn step(
        &mut self,
        game: &mut Game,
        link: &mut MissionLink,
        ashore: bool,
        dt: f32,
    ) -> Option<Intent> {
        let mut intent = Intent::default();
        game.combat.hp = game.combat.max_hp;
        // While the scripts have her controls off (a cinematic, a fade),
        // she waits, the clock stopped.
        let settling = matches!(self.stage, Stage::Settling(_));
        if game.input_locked && !settling {
            return Some(intent);
        }
        match self.stage {
            Stage::Settling(t) => {
                if !link.running() {
                    return None;
                }
                let ready = ashore && game.on_foot && !game.input_locked;
                let t = if ready { t + dt } else { 0.0 };
                self.stage = Stage::Settling(t);
                if t < SETTLE {
                    return None;
                }
                let all = link.support_keys();
                self.keys = if self.asked.is_empty() {
                    all
                } else {
                    all.into_iter()
                        .filter(|(k, _)| self.asked.contains(k))
                        .collect()
                };
                eprintln!("shop-check: {} items to try", self.keys.len());
                self.begin(game, link);
            }
            Stage::Paid(t) => {
                let cash = link.resources().map_or(0.0, |r| r.0);
                if cash >= self.now.price || t > PAY_WAIT {
                    self.buy(link);
                } else {
                    self.stage = Stage::Paid(t + dt);
                }
            }
            // Her PDA left open (the contract's tutorial has her open it)
            // keeps the support menu shut: closed first.
            Stage::Stocking(_) if game.support.pda => {
                self.tapped = !self.tapped;
                intent.trigger.pda = self.tapped;
            }
            Stage::Stocking(t) => {
                let index = game
                    .support
                    .entries()
                    .iter()
                    .position(|e| matches!(e, Entry::Stocked(s) if s.key == self.now.key));
                match index {
                    Some(i) => {
                        game.support.selected = i;
                        intent.trigger.support = true;
                        self.stage = Stage::Menu;
                    }
                    None if t > STOCK_WAIT => self.finish(game, link, "not in the support menu"),
                    None => self.stage = Stage::Stocking(t + dt),
                }
            }
            Stage::Menu => {
                if game.support.phase == Phase::Menu {
                    intent.use_pressed = true;
                    self.now.called = true;
                    self.marked = None;
                    self.stage = Stage::Calling(0.0);
                } else {
                    self.finish(game, link, "the support menu did not open");
                }
            }
            Stage::Calling(t) => {
                self.call(game, &mut intent);
                // Counted from the designator coming down: thrown, or
                // aimed and confirmed.
                let down = self.now.designator.is_some()
                    && !game.combat.to_throw
                    && !matches!(game.support.phase, Phase::Designating { .. });
                if down && self.marked.is_none() {
                    self.marked = Some(link.counts);
                }
                if let Some(at) = self.marked {
                    self.now.came = since(link.counts, at);
                }
                self.now.seconds = t;
                if self.now.came() {
                    self.stage = Stage::Lingering(LINGER);
                } else if t > CALL_WAIT {
                    self.finish(game, link, "nothing came");
                } else {
                    self.stage = Stage::Calling(t + dt);
                }
            }
            Stage::Lingering(left) => {
                if let Some(at) = self.marked {
                    self.now.came = since(link.counts, at);
                }
                if left <= dt {
                    self.finish(game, link, "");
                } else {
                    self.stage = Stage::Lingering(left - dt);
                }
            }
        }
        Some(intent)
    }

    /// Her keys with the designator the scripts handed her.
    fn call(&mut self, game: &Game, intent: &mut Intent) {
        if game.combat.to_throw {
            self.now.designator.get_or_insert_with(|| "thrown".into());
            intent.trigger.throw = true;
        }
        if let Phase::Designating { aim, .. } = game.support.phase {
            self.now
                .designator
                .get_or_insert_with(|| aim.designator().into());
            match aim {
                // Confirmed where the view opens, over her.
                Aim::Satellite => {
                    self.tapped = !self.tapped;
                    intent.trigger.fire = self.tapped;
                }
                // Held on whatever is ahead.
                Aim::Laser => intent.trigger.fire = true,
            }
        }
    }

    /// Gives her cash for the next item, or ends the check.
    fn begin(&mut self, game: &mut Game, link: &mut MissionLink) {
        let Some((key, strike)) = self.keys.get(self.next).cloned() else {
            self.now = Outcome::default();
            return;
        };
        self.next += 1;
        link.free_recruits();
        // Its price in the first shop that sells it.
        let mut found = None;
        for (faction, name) in SHOPS {
            let Some(view) = link.open_shop(faction, true) else {
                continue;
            };
            link.close_shop();
            if let Some(item) = view.items.iter().find(|i| i.id == key && i.unlocked) {
                found = Some((faction, name, item.price));
                break;
            }
        }
        self.now = Outcome {
            key: key.clone(),
            strike,
            ..Outcome::default()
        };
        match found {
            Some((faction, name, price)) => {
                eprintln!("shop-check: {key}: sold by {name} for {price}");
                self.now.shop = Some(faction);
                self.now.price = price;
                game.earned.push((price.ceil() as u32 + 1000, FUEL));
                self.stage = Stage::Paid(0.0);
            }
            None => self.finish(game, link, "no shop sells it"),
        }
    }

    fn buy(&mut self, link: &mut MissionLink) {
        let Some(faction) = self.now.shop else {
            return;
        };
        let before = link.open_shop(faction, true).map_or(0.0, |v| v.cash);
        let after = link.buy(&self.now.key);
        link.close_shop();
        let (bought, cash) = after.map_or((false, before), |(v, b)| (b, v.cash));
        self.now.bought = bought;
        self.now.cash = (before, cash);
        eprintln!(
            "shop-check: {}: bought {bought}, cash {before:.0} -> {cash:.0}",
            self.now.key
        );
        self.stage = Stage::Stocking(0.0);
    }

    /// Records the item tried, and goes on to the next.
    fn finish(&mut self, game: &mut Game, link: &mut MissionLink, why: &str) {
        let done = std::mem::take(&mut self.now);
        eprintln!(
            "shop-check: {}: bought {}, called {}, designator {}, came {:?} after {:.1} s{}{}",
            done.key,
            done.bought,
            done.called,
            done.designator.as_deref().unwrap_or("none"),
            done.came,
            done.seconds,
            if why.is_empty() { "" } else { ": " },
            why
        );
        self.done.push(done);
        // Anything left in her hands is put away.
        game.combat.to_throw = false;
        game.support.unequip();
        game.support.phase = Phase::Idle;
        self.begin(game, link);
    }

    /// The summary lines.
    pub fn summary(&self) -> Vec<String> {
        let n = self.done.len();
        let count = |f: &dyn Fn(&Outcome) -> bool| self.done.iter().filter(|o| f(o)).count();
        let failed: Vec<&str> = self
            .done
            .iter()
            .filter(|o| !o.delivered())
            .map(|o| o.key.as_str())
            .collect();
        vec![
            format!(
                "shop-check: {n} items: {} sold in a shop, {} bought with the price taken, \
                 {} called in, {} with a designator, {} delivered or struck",
                count(&|o| o.shop.is_some()),
                count(&Outcome::paid),
                count(&|o| o.called),
                count(&|o| o.designator.is_some()),
                count(&Outcome::delivered),
            ),
            format!(
                "shop-check: deliveries {}/{}, strikes {}/{}",
                count(&|o| !o.strike && o.delivered()),
                count(&|o| !o.strike),
                count(&|o| o.strike && o.delivered()),
                count(&|o| o.strike),
            ),
            format!("shop-check: not delivered: {}", failed.join(" ")),
        ]
    }
}

/// What came between `before` and `now`.
fn since(now: SupportCounts, before: SupportCounts) -> SupportCounts {
    SupportCounts {
        arrived: now.arrived.saturating_sub(before.arrived),
        flown: now.flown.saturating_sub(before.flown),
        burst: now.burst.saturating_sub(before.burst),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_item_counts_when_paid_for_and_what_it_is_for_came() {
        let crate_ = Outcome {
            price: 7500.0,
            cash: (8500.0, 1000.0),
            bought: true,
            came: SupportCounts {
                arrived: 1,
                ..SupportCounts::default()
            },
            ..Outcome::default()
        };
        assert!(crate_.delivered());
        // A strike isn't done by a helicopter coming, nor anything unpaid.
        let strike = Outcome {
            strike: true,
            ..crate_.clone()
        };
        assert!(!strike.delivered());
        let unpaid = Outcome {
            cash: (8500.0, 8500.0),
            ..crate_
        };
        assert!(!unpaid.delivered());
    }
}
