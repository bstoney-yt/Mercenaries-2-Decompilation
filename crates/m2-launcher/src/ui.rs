//! The launcher's window: a sidebar on the left (the player, the game,
//! the pages to come, Settings), and beside it a header with the tabs
//! (Play, Saves, Options) over the page. Play is the banner picture over
//! the bar with the save picker, the Play button and who is playing;
//! Saves lists the slots with Play, Copy and Delete and starts new games;
//! Options keeps the window and folders. Colours and sizes are invented.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant, SystemTime};

use egui::{
    Align2, Color32, CornerRadius, Id, Mesh, Painter, Pos2, Rect, Sense, Stroke, StrokeKind,
    TextureHandle, TextureOptions, Ui, UiBuilder, pos2, vec2,
};

use crate::assets::{self, Loaded, Role};
use crate::launch::{self, GameSettings, Settings, Start};
use crate::saves::{self, Slot};
use crate::text::{Fonts, GameFont};
use crate::{Args, Paths};

const BG: Color32 = Color32::from_rgb(0x26, 0x26, 0x26);
const SIDEBAR: Color32 = Color32::from_rgb(0x1a, 0x1a, 0x1a);
const SIDEBAR_HOVER: Color32 = Color32::from_rgb(0x2c, 0x2c, 0x2c);
const SIDEBAR_SELECTED: Color32 = Color32::from_rgb(0x36, 0x36, 0x36);
const HEADER: Color32 = Color32::from_rgb(0x22, 0x22, 0x22);
const BAR: Color32 = Color32::from_rgb(0x1e, 0x1e, 0x1e);
const LINE: Color32 = Color32::from_rgb(0x3a, 0x3a, 0x3a);
const TEXT: Color32 = Color32::from_rgb(0xf2, 0xf2, 0xf2);
const DIM: Color32 = Color32::from_rgb(0xa0, 0xa0, 0xa0);
const FAINT: Color32 = Color32::from_rgb(0x6a, 0x6a, 0x6a);
const GREEN: Color32 = Color32::from_rgb(0x3c, 0x85, 0x27);
const GREEN_HOVER: Color32 = Color32::from_rgb(0x48, 0x9c, 0x30);
const GREEN_EDGE: Color32 = Color32::from_rgb(0x27, 0x5e, 0x18);
const RED: Color32 = Color32::from_rgb(0xb8, 0x3a, 0x2e);
const WARN: Color32 = Color32::from_rgb(0xe8, 0xa1, 0x3a);

const SIDEBAR_W: f32 = 236.0;
const HEADER_H: f32 = 100.0;
const BAR_H: f32 = 112.0;
const PAD: f32 = 24.0;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Tab {
    #[default]
    Play,
    Saves,
    Options,
}

/// What the picker has chosen for Play.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Pick {
    New,
    Slot(u32),
}

const HEROES: [(&str, &str); 3] = [
    ("mattias", "Mattias"),
    ("jennifer", "Jennifer"),
    ("chris", "Chris"),
];

fn hero_colour(hero: Option<&str>) -> Color32 {
    match hero.map(str::to_ascii_lowercase).as_deref() {
        Some("mattias") => Color32::from_rgb(0xc8, 0x6a, 0x1e),
        Some("chris") => Color32::from_rgb(0x2f, 0x6f, 0xb0),
        Some("jennifer") => Color32::from_rgb(0xa8, 0x3a, 0x6a),
        _ => Color32::from_rgb(0x55, 0x55, 0x55),
    }
}

/// The screenshot `--screenshot` asked for.
#[derive(Debug)]
struct Shot {
    path: PathBuf,
    /// Frames drawn since everything was in.
    settled: u32,
    asked: bool,
}

pub struct App {
    paths: Paths,
    settings: Settings,
    /// The install, and whether the game must be told where it is.
    install: Option<m2_engine::Install>,
    pass_install: bool,
    install_arg: Option<PathBuf>,
    game_settings: GameSettings,
    loading: Option<Receiver<Loaded>>,
    fonts: Fonts,
    art: HashMap<u32, TextureHandle>,
    tab: Tab,
    banner: usize,
    banner_since: Instant,
    slots: Vec<Slot>,
    scanned: Instant,
    pick: Pick,
    picker_open: bool,
    confirm_delete: Option<u32>,
    status: Option<(String, bool)>,
    child: Option<std::process::Child>,
    shot: Option<Shot>,
    started: Instant,
}

impl std::fmt::Debug for App {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("App")
            .field("tab", &self.tab)
            .field("slots", &self.slots.len())
            .finish_non_exhaustive()
    }
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, paths: Paths, args: Args) -> Self {
        let mut style = (*cc.egui_ctx.global_style()).clone();
        style.visuals = egui::Visuals::dark();
        style.visuals.panel_fill = BG;
        style.visuals.selection.bg_fill = GREEN;
        cc.egui_ctx.set_global_style(style);

        let settings = Settings::read(&paths.settings);
        let game_settings = GameSettings::read(&paths.game_settings);
        let (install, pass_install) =
            crate::find_install(args.install.as_deref(), &settings, &game_settings);
        let loading = install.as_ref().map(|i| {
            let data = i.data.clone();
            let ctx = cc.egui_ctx.clone();
            let (tx, rx) = std::sync::mpsc::channel();
            std::thread::spawn(move || {
                let loaded = assets::load(&data);
                let _ = tx.send(loaded);
                ctx.request_repaint();
            });
            rx
        });
        let slots = saves::scan(&paths.saves);
        let pick = slots
            .iter()
            .find(|s| s.problem.is_none())
            .map_or(Pick::New, |s| Pick::Slot(s.number));
        Self {
            paths,
            settings,
            install,
            pass_install,
            install_arg: args.install,
            game_settings,
            loading,
            fonts: Fonts::default(),
            art: HashMap::new(),
            tab: args.tab,
            banner: 0,
            banner_since: Instant::now(),
            slots,
            scanned: Instant::now(),
            pick,
            picker_open: false,
            confirm_delete: None,
            status: None,
            child: None,
            shot: args.screenshot.map(|path| Shot {
                path,
                settled: 0,
                asked: false,
            }),
            started: Instant::now(),
        }
    }

    /// Takes in what the loading thread read.
    fn poll(&mut self, ctx: &egui::Context) {
        let Some(rx) = &self.loading else { return };
        let Ok(loaded) = rx.try_recv() else { return };
        self.loading = None;
        for line in &loaded.log {
            eprintln!("launcher: {line}");
        }
        let upload = |name: String, rgba: &assets::Rgba| {
            ctx.load_texture(
                name,
                egui::ColorImage::from_rgba_unmultiplied([rgba.width, rgba.height], &rgba.pixels),
                TextureOptions::LINEAR,
            )
        };
        for (role, font, page) in loaded.fonts {
            let handle = upload(format!("font {role:?}"), &page);
            self.fonts.fonts.insert(
                role,
                GameFont {
                    font,
                    page: handle.id(),
                },
            );
            // The handle keeps the texture alive.
            self.art.insert(u32::MAX - role as u32, handle);
        }
        for (hash, rgba) in loaded.art {
            let handle = upload(format!("art {hash:08x}"), &rgba);
            self.art.insert(hash, handle);
        }
    }

    fn banners(&self) -> Vec<&assets::Art> {
        assets::BANNER
            .iter()
            .filter(|a| self.art.contains_key(&a.hash))
            .collect()
    }

    fn save_settings(&mut self) {
        if let Err(e) = self.settings.write(&self.paths.settings) {
            self.status = Some((
                format!("Could not save {}: {e}", self.paths.settings.display()),
                true,
            ));
        }
    }

    fn rescan(&mut self) {
        self.slots = saves::scan(&self.paths.saves);
        self.scanned = Instant::now();
        if let Pick::Slot(n) = self.pick
            && !self.slots.iter().any(|s| s.number == n)
        {
            self.pick = Pick::New;
        }
    }

    fn hero(&self) -> &str {
        &self.settings.hero
    }

    fn play(&mut self, ctx: &egui::Context, start: Start) {
        if let Some(c) = &mut self.child
            && matches!(c.try_wait(), Ok(None))
        {
            self.status = Some(("The game is already running.".to_owned(), true));
            return;
        }
        let game = launch::game_path(&self.settings, &self.paths.dir);
        let install = self
            .pass_install
            .then(|| self.install.as_ref().map(|i| i.root.clone()))
            .flatten()
            .or_else(|| self.install_arg.clone());
        let args = launch::args(&self.settings, install.as_deref(), &start);
        eprintln!("launcher: {} {}", game.display(), args.join(" "));
        match launch::spawn(&game, &args) {
            Ok(child) => {
                self.child = Some(child);
                self.status = Some(("Starting the game...".to_owned(), false));
                if !self.settings.keep_open {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            }
            Err(e) => self.status = Some((format!("Could not start the game: {e}"), true)),
        }
    }

    fn pick_start(&self) -> Start {
        match self.pick {
            Pick::New => Start::New(self.settings.hero.clone()),
            Pick::Slot(n) => Start::Slot(n),
        }
    }

    fn slot(&self, n: u32) -> Option<&Slot> {
        self.slots.iter().find(|s| s.number == n)
    }

    /// A save's line under its title: who, cash, fuel, when.
    fn detail(&self, s: &Slot, fuel: bool) -> String {
        let mut parts = vec![
            s.hero
                .as_deref()
                .map_or("Unknown mercenary", saves::hero_name)
                .to_owned(),
            saves::money(s.cash),
        ];
        if fuel {
            parts.push(format!("Fuel {}", s.fuel.round()));
        }
        // The 360's save list's play time, contract and percent (the
        // record's; old saves have none).
        if let Some(t) = s.played_text() {
            parts.push(t);
        }
        if fuel && !s.contract.is_empty() {
            parts.push(s.contract.clone());
        }
        if let Some(p) = s.percent {
            parts.push(format!("{p}%"));
        }
        if let Some(w) = s.written {
            parts.push(saves::ago(SystemTime::now(), w));
        }
        parts.join("  -  ")
    }

    // ---- drawing ----

    fn sidebar(&mut self, ui: &mut Ui, rect: Rect) {
        let p = ui.painter_at(rect);
        p.rect_filled(rect, 0.0, SIDEBAR);
        p.vline(rect.right(), rect.y_range(), Stroke::new(1.0, LINE));

        // The player.
        let top = Rect::from_min_size(rect.min, vec2(rect.width(), 84.0));
        let badge =
            Rect::from_center_size(pos2(top.left() + 36.0, top.center().y), vec2(40.0, 40.0));
        p.rect_filled(badge, 6.0, hero_colour(Some(self.hero())));
        let initial: String = self
            .hero()
            .chars()
            .take(1)
            .collect::<String>()
            .to_uppercase();
        self.fonts.text(
            &p,
            Role::Heading,
            badge.center(),
            Align2::CENTER_CENTER,
            24.0,
            TEXT,
            &initial,
        );
        self.fonts.text(
            &p,
            Role::Body,
            pos2(badge.right() + 12.0, top.center().y - 2.0),
            Align2::LEFT_BOTTOM,
            14.0,
            DIM,
            "Playing as",
        );
        self.fonts.text(
            &p,
            Role::BodyBold,
            pos2(badge.right() + 12.0, top.center().y + 1.0),
            Align2::LEFT_TOP,
            18.0,
            TEXT,
            saves::hero_name(self.hero()),
        );
        p.hline(rect.x_range(), top.bottom(), Stroke::new(1.0, LINE));

        // The game.
        let mut y = top.bottom() + 12.0;
        let item = Rect::from_min_size(pos2(rect.left() + 8.0, y), vec2(rect.width() - 16.0, 60.0));
        let r = ui.interact(item, Id::new("side-game"), Sense::click());
        if r.clicked() {
            self.tab = Tab::Play;
        }
        p.rect_filled(item, 4.0, SIDEBAR_SELECTED);
        p.rect_filled(
            Rect::from_min_size(item.min, vec2(3.0, item.height())),
            CornerRadius::same(1),
            GREEN,
        );
        let icon =
            Rect::from_center_size(pos2(item.left() + 30.0, item.center().y), vec2(36.0, 36.0));
        match self.art.get(&assets::SKULL.hash) {
            Some(t) => {
                p.rect_filled(icon, 6.0, Color32::from_rgb(0x10, 0x10, 0x10));
                p.image(
                    t.id(),
                    icon.shrink(3.0),
                    Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                    Color32::WHITE,
                );
            }
            None => {
                p.rect_filled(icon, 6.0, GREEN_EDGE);
            }
        }
        self.fonts.text(
            &p,
            Role::Heading,
            pos2(icon.right() + 12.0, item.center().y + 1.0),
            Align2::LEFT_BOTTOM,
            19.0,
            TEXT,
            "MERCENARIES 2",
        );
        self.fonts.text(
            &p,
            Role::Body,
            pos2(icon.right() + 12.0, item.center().y + 2.0),
            Align2::LEFT_TOP,
            14.0,
            DIM,
            "World in Flames",
        );
        y = item.bottom() + 18.0;

        // The pages to come.
        self.fonts.text(
            &p,
            Role::Heading,
            pos2(rect.left() + 20.0, y),
            Align2::LEFT_TOP,
            13.0,
            FAINT,
            "COMING SOON",
        );
        y += 24.0;
        for (i, name) in ["What's New", "Achievements", "Mods", "Gallery"]
            .into_iter()
            .enumerate()
        {
            let item =
                Rect::from_min_size(pos2(rect.left() + 8.0, y), vec2(rect.width() - 16.0, 40.0));
            let r = ui.interact(item, Id::new(("side-soon", i)), Sense::hover());
            if r.hovered() {
                p.rect_filled(item, 4.0, SIDEBAR_HOVER);
            }
            self.fonts.text(
                &p,
                Role::Body,
                pos2(item.left() + 14.0, item.center().y),
                Align2::LEFT_CENTER,
                17.0,
                FAINT,
                name,
            );
            let pill = Rect::from_min_size(
                pos2(item.right() - 54.0, item.center().y - 9.0),
                vec2(44.0, 18.0),
            );
            p.rect_stroke(pill, 9.0, Stroke::new(1.0, FAINT), StrokeKind::Inside);
            self.fonts.text(
                &p,
                Role::Heading,
                pill.center(),
                Align2::CENTER_CENTER,
                11.0,
                FAINT,
                "SOON",
            );
            y += 42.0;
        }

        // Settings, at the foot.
        let foot = Rect::from_min_max(pos2(rect.left(), rect.bottom() - 92.0), rect.max);
        p.hline(rect.x_range(), foot.top(), Stroke::new(1.0, LINE));
        let item = Rect::from_min_size(
            pos2(rect.left() + 8.0, foot.top() + 10.0),
            vec2(rect.width() - 16.0, 40.0),
        );
        let r = ui.interact(item, Id::new("side-settings"), Sense::click());
        if r.hovered() || self.tab == Tab::Options {
            p.rect_filled(item, 4.0, SIDEBAR_HOVER);
        }
        if r.clicked() {
            self.tab = Tab::Options;
        }
        gear(&p, pos2(item.left() + 22.0, item.center().y), 8.0, DIM);
        self.fonts.text(
            &p,
            Role::Body,
            pos2(item.left() + 42.0, item.center().y),
            Align2::LEFT_CENTER,
            17.0,
            TEXT,
            "Settings",
        );
        self.fonts.text(
            &p,
            Role::Body,
            pos2(rect.left() + 20.0, rect.bottom() - 18.0),
            Align2::LEFT_CENTER,
            13.0,
            FAINT,
            &format!("Launcher {}", env!("CARGO_PKG_VERSION")),
        );
    }

    fn header(&mut self, ui: &mut Ui, rect: Rect) {
        let p = ui.painter_at(rect);
        p.rect_filled(rect, 0.0, HEADER);
        p.hline(rect.x_range(), rect.bottom(), Stroke::new(1.0, LINE));
        self.fonts.text(
            &p,
            Role::Heading,
            pos2(rect.left() + PAD, rect.top() + 16.0),
            Align2::LEFT_TOP,
            24.0,
            TEXT,
            "MERCENARIES 2: WORLD IN FLAMES",
        );
        let mut x = rect.left() + PAD;
        let tabs = [
            (Some(Tab::Play), "PLAY"),
            (Some(Tab::Saves), "SAVES"),
            (Some(Tab::Options), "OPTIONS"),
            (None, "PATCH NOTES"),
        ];
        for (i, (tab, name)) in tabs.into_iter().enumerate() {
            let w = self.fonts.width(&p, Role::Heading, 19.0, name);
            let hit = Rect::from_min_max(
                pos2(x - 6.0, rect.bottom() - 44.0),
                pos2(x + w + 6.0, rect.bottom()),
            );
            let r = ui.interact(hit, Id::new(("tab", i)), Sense::click());
            let active = tab == Some(self.tab);
            let colour = match (tab, active, r.hovered()) {
                (None, ..) => FAINT,
                (_, true, _) | (_, _, true) => TEXT,
                _ => DIM,
            };
            self.fonts.text(
                &p,
                Role::Heading,
                pos2(x, rect.bottom() - 22.0),
                Align2::LEFT_CENTER,
                19.0,
                colour,
                name,
            );
            if active {
                p.rect_filled(
                    Rect::from_min_max(pos2(x, rect.bottom() - 4.0), pos2(x + w, rect.bottom())),
                    CornerRadius::same(1),
                    GREEN,
                );
            }
            if let (true, Some(t)) = (r.clicked(), tab) {
                self.tab = t;
                self.picker_open = false;
            }
            x += w + 36.0;
        }
    }

    fn play_tab(&mut self, ui: &mut Ui, rect: Rect) {
        let ctx = ui.ctx().clone();
        let hero = Rect::from_min_max(rect.min, pos2(rect.right(), rect.bottom() - BAR_H));
        let bar = Rect::from_min_max(pos2(rect.left(), hero.bottom()), rect.max);
        let p = ui.painter_at(rect);

        // The banner.
        let banners = self.banners();
        let count = banners.len();
        if count > 0 {
            let art = banners[self.banner % count];
            let tex = &self.art[&art.hash];
            if art.hash == assets::BANNER[0].hash
                && let Some(head) = self.art.get(&assets::BILL_HEAD.hash)
            {
                // Behind the bill, in its oval (invented placement: the
                // bill's middle, 80% of its height).
                let bill = cover_rect(tex, hero);
                // The oval is left of the bill's middle (measured on the
                // VM's screenshot: 6% of its width).
                let side = bill.height() * 0.9;
                let centre = bill.center() - vec2(bill.width() * 0.06, 0.0);
                p.rect_filled(hero, 0.0, Color32::from_rgb(0x12, 0x10, 0x0c));
                p.image(
                    head.id(),
                    Rect::from_center_size(centre, vec2(side, side)),
                    Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                    Color32::WHITE,
                );
            }
            cover(&p, tex, hero);
            // Darker toward the bar, for the text over it.
            gradient(
                &p,
                Rect::from_min_max(pos2(hero.left(), hero.bottom() - 220.0), hero.max),
                Color32::TRANSPARENT,
                Color32::from_black_alpha(210),
            );
            gradient(
                &p,
                Rect::from_min_max(hero.min, pos2(hero.right(), hero.top() + 70.0)),
                Color32::from_black_alpha(110),
                Color32::TRANSPARENT,
            );
            self.fonts.text(
                &p,
                Role::Heading,
                pos2(hero.left() + PAD, hero.bottom() - 18.0),
                Align2::LEFT_BOTTOM,
                15.0,
                Color32::from_white_alpha(170),
                &art.caption.to_uppercase(),
            );
            // Its dots, and arrows to step through them.
            let dots_w = count as f32 * 16.0;
            for i in 0..count {
                let c = pos2(
                    hero.center().x - dots_w / 2.0 + 8.0 + i as f32 * 16.0,
                    hero.bottom() - 24.0,
                );
                let r = ui.interact(
                    Rect::from_center_size(c, vec2(14.0, 14.0)),
                    Id::new(("dot", i)),
                    Sense::click(),
                );
                if r.clicked() {
                    self.banner = i;
                    self.banner_since = Instant::now();
                }
                let on = i == self.banner % count;
                p.circle_filled(
                    c,
                    if on { 4.5 } else { 3.5 },
                    if on {
                        TEXT
                    } else {
                        Color32::from_white_alpha(90)
                    },
                );
            }
            for (side, step) in [(-1.0f32, count - 1), (1.0, 1)] {
                let c = pos2(
                    if side < 0.0 {
                        hero.left() + 36.0
                    } else {
                        hero.right() - 36.0
                    },
                    hero.center().y,
                );
                let r = ui.interact(
                    Rect::from_center_size(c, vec2(44.0, 44.0)),
                    Id::new(("arrow", step)),
                    Sense::click(),
                );
                let a = if r.hovered() { 170 } else { 90 };
                p.circle_filled(c, 20.0, Color32::from_black_alpha(a));
                chevron(&p, c, side, TEXT);
                if r.clicked() {
                    self.banner = (self.banner + step) % count;
                    self.banner_since = Instant::now();
                }
            }
            // On by itself every 12 s (not while a screenshot waits).
            if self.shot.is_none() && self.banner_since.elapsed() > Duration::from_secs(12) {
                self.banner = (self.banner + 1) % count;
                self.banner_since = Instant::now();
            }
            ctx.request_repaint_after(Duration::from_millis(500));
        } else {
            gradient(
                &p,
                hero,
                Color32::from_rgb(0x3a, 0x2c, 0x1c),
                Color32::from_rgb(0x14, 0x12, 0x10),
            );
            let why = if self.loading.is_some() {
                "Reading the game's pictures..."
            } else if self.install.is_none() {
                "Game install not found: set its folder in Options"
            } else {
                "The game's pictures could not be read"
            };
            self.fonts.text(
                &p,
                Role::Body,
                hero.center() + vec2(0.0, 56.0),
                Align2::CENTER_CENTER,
                17.0,
                DIM,
                why,
            );
        }
        // The game's name, at the banner's foot (clear of the pictures'
        // middles).
        let logo = pos2(hero.left() + PAD, hero.bottom() - 62.0);
        for (offset, colour) in [
            (vec2(3.0, 3.0), Color32::from_black_alpha(200)),
            (vec2(0.0, 0.0), TEXT),
        ] {
            self.fonts.text(
                &p,
                Role::Heading,
                logo + offset,
                Align2::LEFT_BOTTOM,
                50.0,
                colour,
                "MERCENARIES 2",
            );
            self.fonts.text(
                &p,
                Role::Heading,
                logo + offset + vec2(2.0, 2.0),
                Align2::LEFT_TOP,
                21.0,
                if offset.x > 0.0 {
                    colour
                } else {
                    Color32::from_rgb(0xf0, 0xb0, 0x3a)
                },
                "WORLD IN FLAMES",
            );
        }

        // The bar.
        p.rect_filled(bar, 0.0, BAR);
        p.hline(bar.x_range(), bar.top(), Stroke::new(1.0, LINE));

        // The save picker, at its left.
        let picker = Rect::from_min_size(
            pos2(bar.left() + PAD, bar.center().y - 30.0),
            vec2(330.0_f32.min(bar.width() * 0.32), 60.0),
        );
        let r = ui.interact(picker, Id::new("picker"), Sense::click());
        if r.clicked() {
            self.picker_open = !self.picker_open;
        }
        p.rect_filled(
            picker,
            4.0,
            if r.hovered() || self.picker_open {
                SIDEBAR_SELECTED
            } else {
                Color32::from_rgb(0x2a, 0x2a, 0x2a)
            },
        );
        let (title, detail, who) = self.pick_lines(self.pick);
        self.choice(&p, picker, &title, &detail, who.as_deref());
        caret(
            &p,
            pos2(picker.right() - 18.0, picker.center().y),
            DIM,
            self.picker_open,
        );

        // Play, in the middle.
        let button = Rect::from_center_size(bar.center(), vec2(300.0, 68.0));
        let r = ui.interact(button, Id::new("play"), Sense::click());
        let fill = if r.hovered() { GREEN_HOVER } else { GREEN };
        p.rect_filled(button, 4.0, GREEN_EDGE);
        p.rect_filled(
            Rect::from_min_max(button.min, pos2(button.right(), button.bottom() - 5.0)),
            4.0,
            fill,
        );
        self.fonts.text(
            &p,
            Role::Heading,
            button.center() - vec2(0.0, 2.0),
            Align2::CENTER_CENTER,
            36.0,
            TEXT,
            "PLAY",
        );
        if r.clicked() {
            self.picker_open = false;
            let start = self.pick_start();
            self.play(&ctx, start);
        }

        // Who plays, and what is happening, at its right.
        let right = pos2(bar.right() - PAD, bar.center().y);
        let game = launch::game_path(&self.settings, &self.paths.dir);
        let (line, colour) = match &self.status {
            Some((s, err)) => (s.clone(), if *err { WARN } else { DIM }),
            None if !game.is_file() => (format!("{} not found", file_name(&game)), WARN),
            None => ("Ready to play".to_owned(), DIM),
        };
        self.fonts.text(
            &p,
            Role::BodyBold,
            right - vec2(0.0, 2.0),
            Align2::RIGHT_BOTTOM,
            19.0,
            TEXT,
            saves::hero_name(match self.pick {
                Pick::Slot(n) => self
                    .slot(n)
                    .and_then(|s| s.hero.as_deref())
                    .unwrap_or(self.hero()),
                Pick::New => self.hero(),
            }),
        );
        let w = self.fonts.width(&p, Role::Body, 14.0, &line);
        let max = bar.right() - PAD - (button.right() + 24.0);
        let line = if w > max { ellipsis(&line, 48) } else { line };
        self.fonts.text(
            &p,
            Role::Body,
            right + vec2(0.0, 3.0),
            Align2::RIGHT_TOP,
            14.0,
            colour,
            &line,
        );

        if self.picker_open {
            self.picker_list(ui, picker);
        }
    }

    /// The picker's lines for `pick`: title, detail, whose.
    fn pick_lines(&self, pick: Pick) -> (String, String, Option<String>) {
        match pick {
            Pick::New => (
                "New game".to_owned(),
                format!("Start as {}", saves::hero_name(self.hero())),
                Some(self.hero().to_owned()),
            ),
            Pick::Slot(n) => match self.slot(n) {
                Some(s) => (
                    if self.slots.first().map(|f| f.number) == Some(n) {
                        format!("Continue - {}", s.title())
                    } else {
                        s.title()
                    },
                    self.detail(s, false),
                    s.hero.clone(),
                ),
                None => ("New game".to_owned(), String::new(), None),
            },
        }
    }

    /// One choice: a badge with the hero's initial, a title and a line.
    fn choice(&self, p: &Painter, rect: Rect, title: &str, detail: &str, hero: Option<&str>) {
        let badge =
            Rect::from_center_size(pos2(rect.left() + 30.0, rect.center().y), vec2(36.0, 36.0));
        p.rect_filled(badge, 4.0, hero_colour(hero));
        let initial = hero
            .and_then(|h| h.chars().next())
            .map_or("?".to_owned(), |c| c.to_uppercase().to_string());
        self.fonts.text(
            p,
            Role::Heading,
            badge.center(),
            Align2::CENTER_CENTER,
            20.0,
            TEXT,
            &initial,
        );
        self.fonts.text(
            p,
            Role::BodyBold,
            pos2(badge.right() + 12.0, rect.center().y),
            Align2::LEFT_BOTTOM,
            17.0,
            TEXT,
            title,
        );
        self.fonts.text(
            p,
            Role::Body,
            pos2(badge.right() + 12.0, rect.center().y + 3.0),
            Align2::LEFT_TOP,
            13.0,
            DIM,
            &ellipsis(detail, 44),
        );
    }

    fn picker_list(&mut self, ui: &mut Ui, picker: Rect) {
        let mut picks = vec![Pick::New];
        picks.extend(
            self.slots
                .iter()
                .filter(|s| s.problem.is_none())
                .map(|s| Pick::Slot(s.number)),
        );
        let row_h = 56.0;
        let h = (picks.len() as f32 * row_h + 8.0).min(picker.top() - 120.0);
        let area = Rect::from_min_max(
            pos2(picker.left(), picker.top() - 6.0 - h),
            pos2(picker.right() + 40.0, picker.top() - 6.0),
        );
        egui::Area::new(Id::new("picker-list"))
            .order(egui::Order::Foreground)
            .fixed_pos(area.min)
            .show(ui.ctx(), |ui| {
                let p = ui.painter().clone();
                p.rect_filled(area, 4.0, Color32::from_rgb(0x2e, 0x2e, 0x2e));
                p.rect_stroke(area, 4.0, Stroke::new(1.0, LINE), StrokeKind::Inside);
                ui.allocate_exact_size(area.size(), Sense::hover());
                let mut y = area.top() + 4.0;
                for (i, pick) in picks.into_iter().enumerate() {
                    if y + row_h > area.bottom() {
                        break;
                    }
                    let row = Rect::from_min_size(
                        pos2(area.left() + 4.0, y),
                        vec2(area.width() - 8.0, row_h),
                    );
                    let r = ui.interact(row, Id::new(("pick", i)), Sense::click());
                    if r.hovered() || pick == self.pick {
                        p.rect_filled(row, 3.0, SIDEBAR_SELECTED);
                    }
                    let (title, detail, who) = self.pick_lines(pick);
                    self.choice(&p, row, &title, &detail, who.as_deref());
                    if r.clicked() {
                        self.pick = pick;
                        self.picker_open = false;
                    }
                    y += row_h;
                }
            });
        if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.picker_open = false;
        }
    }

    fn saves_tab(&mut self, ui: &mut Ui, rect: Rect) {
        let ctx = ui.ctx().clone();
        let p = ui.painter_at(rect);
        p.rect_filled(rect, 0.0, BG);
        // The new game row.
        let tools = Rect::from_min_size(
            pos2(rect.left() + PAD, rect.top() + 20.0),
            vec2(rect.width() - 2.0 * PAD, 52.0),
        );
        let new = Rect::from_min_size(tools.min, vec2(190.0, 46.0));
        if self.green_button(ui, &p, new, "NEW GAME", 20.0, Id::new("new-game")) {
            let start = Start::New(self.settings.hero.clone());
            self.play(&ctx, start);
        }
        self.fonts.text(
            &p,
            Role::Body,
            pos2(new.right() + 24.0, new.center().y),
            Align2::LEFT_CENTER,
            15.0,
            DIM,
            "as",
        );
        let mut x = new.right() + 50.0;
        for (key, name) in HEROES {
            let w = self.fonts.width(&p, Role::BodyBold, 16.0, name) + 28.0;
            let chip = Rect::from_min_size(pos2(x, new.top() + 6.0), vec2(w, 34.0));
            let r = ui.interact(chip, Id::new(("hero", key)), Sense::click());
            let on = self.settings.hero == key;
            p.rect_filled(
                chip,
                17.0,
                if on {
                    hero_colour(Some(key))
                } else if r.hovered() {
                    SIDEBAR_SELECTED
                } else {
                    Color32::from_rgb(0x30, 0x30, 0x30)
                },
            );
            self.fonts.text(
                &p,
                Role::BodyBold,
                chip.center(),
                Align2::CENTER_CENTER,
                16.0,
                if on { TEXT } else { DIM },
                name,
            );
            if r.clicked() {
                key.clone_into(&mut self.settings.hero);
                self.save_settings();
            }
            x += w + 8.0;
        }
        self.fonts.text(
            &p,
            Role::Body,
            pos2(tools.right(), new.center().y),
            Align2::RIGHT_CENTER,
            14.0,
            FAINT,
            &format!(
                "{} of {} slots used  -  {}",
                self.slots.iter().filter(|s| s.number > 0).count(),
                saves::SLOTS,
                self.paths.saves.display()
            ),
        );

        // The list.
        let list = Rect::from_min_max(
            pos2(rect.left() + PAD, tools.bottom() + 16.0),
            pos2(rect.right() - PAD, rect.bottom() - 12.0),
        );
        p.hline(list.x_range(), list.top() - 4.0, Stroke::new(1.0, LINE));
        if self.slots.is_empty() {
            self.fonts.text(
                &p,
                Role::Body,
                pos2(list.center().x, list.top() + 80.0),
                Align2::CENTER_CENTER,
                18.0,
                DIM,
                "No saves yet. Start a new game, and the game's saves show up here.",
            );
            return;
        }
        let slots = self.slots.clone();
        let mut action: Option<(u32, &str)> = None;
        let mut child = ui.new_child(UiBuilder::new().max_rect(list));
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(&mut child, |ui| {
                for s in &slots {
                    let (row, r) =
                        ui.allocate_exact_size(vec2(ui.available_width(), 76.0), Sense::hover());
                    let p = ui.painter().clone();
                    if r.hovered() {
                        p.rect_filled(row, 4.0, Color32::from_rgb(0x2e, 0x2e, 0x2e));
                    }
                    let badge = Rect::from_center_size(
                        pos2(row.left() + 36.0, row.center().y),
                        vec2(46.0, 46.0),
                    );
                    p.rect_filled(badge, 6.0, hero_colour(s.hero.as_deref()));
                    let initial = s
                        .hero
                        .as_deref()
                        .and_then(|h| h.chars().next())
                        .map_or("?".to_owned(), |c| c.to_uppercase().to_string());
                    self.fonts.text(
                        &p,
                        Role::Heading,
                        badge.center(),
                        Align2::CENTER_CENTER,
                        26.0,
                        TEXT,
                        &initial,
                    );
                    self.fonts.text(
                        &p,
                        Role::Heading,
                        pos2(badge.right() + 16.0, row.center().y - 1.0),
                        Align2::LEFT_BOTTOM,
                        21.0,
                        TEXT,
                        &s.title().to_uppercase(),
                    );
                    let (line, colour) = match &s.problem {
                        Some(why) => (format!("Can't be loaded: {why}"), WARN),
                        None => (self.detail(s, true), DIM),
                    };
                    self.fonts.text(
                        &p,
                        Role::Body,
                        pos2(badge.right() + 16.0, row.center().y + 3.0),
                        Align2::LEFT_TOP,
                        15.0,
                        colour,
                        &line,
                    );
                    // Its buttons, from the right.
                    let mut bx = row.right() - 8.0;
                    let confirm = self.confirm_delete == Some(s.number);
                    let buttons: [(&str, &str, Color32); 3] = [
                        (
                            "delete",
                            if confirm { "SURE?" } else { "DELETE" },
                            if confirm {
                                RED
                            } else {
                                Color32::from_rgb(0x3a, 0x3a, 0x3a)
                            },
                        ),
                        ("copy", "COPY", Color32::from_rgb(0x3a, 0x3a, 0x3a)),
                        ("play", "PLAY", GREEN),
                    ];
                    for (key, label, fill) in buttons {
                        if key == "play" && s.problem.is_some() {
                            continue;
                        }
                        if key == "copy"
                            && s.number == 0
                            && saves::free_slot(&self.paths.saves).is_none()
                        {
                            continue;
                        }
                        let w = 96.0;
                        let b =
                            Rect::from_min_size(pos2(bx - w, row.center().y - 19.0), vec2(w, 38.0));
                        let r = ui.interact(b, Id::new((key, s.number)), Sense::click());
                        let fill = if r.hovered() {
                            fill.gamma_multiply(1.25)
                        } else {
                            fill
                        };
                        p.rect_filled(b, 4.0, fill);
                        self.fonts.text(
                            &p,
                            Role::Heading,
                            b.center(),
                            Align2::CENTER_CENTER,
                            17.0,
                            TEXT,
                            label,
                        );
                        if r.clicked() {
                            action = Some((s.number, key));
                        }
                        bx -= w + 8.0;
                    }
                    p.hline(
                        row.x_range(),
                        row.bottom(),
                        Stroke::new(1.0, Color32::from_rgb(0x30, 0x30, 0x30)),
                    );
                }
            });
        match action {
            Some((n, "play")) => {
                self.pick = Pick::Slot(n);
                self.play(&ctx, Start::Slot(n));
            }
            Some((n, "copy")) => {
                self.confirm_delete = None;
                self.status = Some(match saves::copy(&self.paths.saves, n) {
                    Ok(to) => (format!("Copied into save {to}."), false),
                    Err(e) => (format!("Could not copy: {e}"), true),
                });
                self.rescan();
            }
            Some((n, "delete")) => {
                if self.confirm_delete == Some(n) {
                    self.confirm_delete = None;
                    self.status = Some(match saves::delete(&self.paths.saves, n) {
                        Ok(()) => (format!("Deleted save {n}."), false),
                        Err(e) => (format!("Could not delete: {e}"), true),
                    });
                    self.rescan();
                } else {
                    self.confirm_delete = Some(n);
                }
            }
            _ => {}
        }
    }

    fn options_tab(&mut self, ui: &mut Ui, rect: Rect) {
        let p = ui.painter_at(rect);
        p.rect_filled(rect, 0.0, BG);
        let inner = rect.shrink2(vec2(PAD + 8.0, 20.0));
        let mut child = ui.new_child(UiBuilder::new().max_rect(inner));
        let mut changed = false;
        let fonts = &self.fonts;
        let heading = |ui: &mut Ui, text: &str| {
            let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 40.0), Sense::hover());
            fonts.text(
                ui.painter(),
                Role::Heading,
                r.left_bottom() - vec2(0.0, 6.0),
                Align2::LEFT_BOTTOM,
                21.0,
                TEXT,
                text,
            );
        };
        child.spacing_mut().item_spacing = vec2(10.0, 10.0);
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(&mut child, |ui| {
                ui.set_max_width(720.0);
                heading(ui, "GAME WINDOW");
                egui::Grid::new("window")
                    .num_columns(2)
                    .spacing([24.0, 12.0])
                    .show(ui, |ui| {
                        ui.label("Resolution");
                        let now = (self.settings.width, self.settings.height);
                        egui::ComboBox::from_id_salt("size")
                            .width(220.0)
                            .selected_text(format!("{} x {}", now.0, now.1))
                            .show_ui(ui, |ui| {
                                for (w, h) in launch::SIZES {
                                    if ui
                                        .selectable_label(now == (w, h), format!("{w} x {h}"))
                                        .clicked()
                                    {
                                        self.settings.width = w;
                                        self.settings.height = h;
                                        changed = true;
                                    }
                                }
                            });
                        ui.end_row();
                        ui.label("Display");
                        egui::ComboBox::from_id_salt("display")
                            .width(220.0)
                            .selected_text(self.settings.display.label())
                            .show_ui(ui, |ui| {
                                for d in launch::Display::ALL {
                                    if ui
                                        .selectable_label(self.settings.display == d, d.label())
                                        .clicked()
                                    {
                                        self.settings.display = d;
                                        changed = true;
                                    }
                                }
                            });
                        ui.end_row();
                    });
                heading(ui, "LAUNCHING");
                changed |= ui
                    .checkbox(
                        &mut self.settings.skip_intro,
                        "Skip the EA and Pandemic movies",
                    )
                    .changed();
                changed |= ui
                    .checkbox(
                        &mut self.settings.keep_open,
                        "Keep the launcher open while the game runs",
                    )
                    .changed();
                heading(ui, "FOLDERS");
                egui::Grid::new("folders")
                    .num_columns(2)
                    .spacing([24.0, 12.0])
                    .show(ui, |ui| {
                        ui.label("Game install");
                        let mut text = self
                            .settings
                            .install
                            .as_ref()
                            .map(|p| p.display().to_string())
                            .unwrap_or_default();
                        let hint = self
                            .install
                            .as_ref()
                            .map_or("Not found".to_owned(), |i| i.root.display().to_string());
                        if ui
                            .add(
                                egui::TextEdit::singleline(&mut text)
                                    .hint_text(hint)
                                    .desired_width(460.0),
                            )
                            .changed()
                        {
                            self.settings.install =
                                (!text.trim().is_empty()).then(|| text.trim().into());
                            changed = true;
                        }
                        ui.end_row();
                        ui.label("");
                        let (line, colour) = match &self.install {
                            Some(i) => (format!("Found: {}", i.data.display()), DIM),
                            None => (
                                "No install found (the folder holding data\\vz.wad).".to_owned(),
                                WARN,
                            ),
                        };
                        ui.colored_label(colour, line);
                        ui.end_row();
                        ui.label("Game program");
                        let game = launch::game_path(&self.settings, &self.paths.dir);
                        let mut text = self
                            .settings
                            .game
                            .as_ref()
                            .map(|p| p.display().to_string())
                            .unwrap_or_default();
                        if ui
                            .add(
                                egui::TextEdit::singleline(&mut text)
                                    .hint_text(game.display().to_string())
                                    .desired_width(460.0),
                            )
                            .changed()
                        {
                            self.settings.game =
                                (!text.trim().is_empty()).then(|| text.trim().into());
                            changed = true;
                        }
                        ui.end_row();
                        ui.label("Saves");
                        ui.colored_label(DIM, self.paths.saves.display().to_string());
                        ui.end_row();
                    });
                ui.add_space(8.0);
                ui.colored_label(
                    FAINT,
                    "Changes are kept at once, in m2-launcher.toml beside the launcher.",
                );
            });
        if changed {
            let install_changed = {
                let (i, pass) = crate::find_install(
                    self.install_arg.as_deref(),
                    &self.settings,
                    &self.game_settings,
                );
                let same = i.as_ref().map(|i| &i.root) == self.install.as_ref().map(|i| &i.root);
                self.install = i;
                self.pass_install = pass;
                !same
            };
            self.save_settings();
            if install_changed && let Some(i) = &self.install {
                let data = i.data.clone();
                let ctx = ui.ctx().clone();
                let (tx, rx) = std::sync::mpsc::channel();
                std::thread::spawn(move || {
                    let _ = tx.send(assets::load(&data));
                    ctx.request_repaint();
                });
                self.loading = Some(rx);
            }
        }
    }

    fn green_button(&self, ui: &Ui, p: &Painter, rect: Rect, label: &str, px: f32, id: Id) -> bool {
        let r = ui.interact(rect, id, Sense::click());
        p.rect_filled(rect, 4.0, GREEN_EDGE);
        p.rect_filled(
            Rect::from_min_max(rect.min, pos2(rect.right(), rect.bottom() - 4.0)),
            4.0,
            if r.hovered() { GREEN_HOVER } else { GREEN },
        );
        self.fonts.text(
            p,
            Role::Heading,
            rect.center() - vec2(0.0, 2.0),
            Align2::CENTER_CENTER,
            px,
            TEXT,
            label,
        );
        r.clicked()
    }

    /// `--screenshot`: once everything is in (or 30 s on), a few frames
    /// later, the window to a PNG; then the launcher closes.
    fn screenshot(&mut self, ctx: &egui::Context) {
        let Some(shot) = &mut self.shot else { return };
        let ready = self.loading.is_none() || self.started.elapsed() > Duration::from_secs(30);
        if ready {
            shot.settled += 1;
        }
        if shot.settled > 5 && !shot.asked {
            shot.asked = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
        }
        let image = ctx.input(|i| {
            i.events.iter().find_map(|e| match e {
                egui::Event::Screenshot { image, .. } => Some(image.clone()),
                _ => None,
            })
        });
        if let Some(image) = image {
            match write_png(&shot.path, &image) {
                Ok(()) => eprintln!("launcher: screenshot {}", shot.path.display()),
                Err(e) => eprintln!("launcher: could not write {}: {e}", shot.path.display()),
            }
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        ctx.request_repaint();
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.poll(&ctx);
        if self.scanned.elapsed() > Duration::from_secs(5) {
            self.rescan();
        }
        if let Some(c) = &mut self.child
            && let Ok(Some(status)) = c.try_wait()
        {
            self.child = None;
            self.rescan();
            self.status = Some((
                if status.success() {
                    "The game has closed.".to_owned()
                } else {
                    format!("The game stopped ({status}). Its log is last-run.log beside it.")
                },
                !status.success(),
            ));
        }
        let full = ui.max_rect();
        ui.painter().rect_filled(full, 0.0, BG);
        let side = Rect::from_min_max(full.min, pos2(full.left() + SIDEBAR_W, full.bottom()));
        let main = Rect::from_min_max(pos2(side.right(), full.top()), full.max);
        let header = Rect::from_min_max(main.min, pos2(main.right(), main.top() + HEADER_H));
        let body = Rect::from_min_max(pos2(main.left(), header.bottom()), main.max);
        self.sidebar(ui, side);
        self.header(ui, header);
        match self.tab {
            Tab::Play => self.play_tab(ui, body),
            Tab::Saves => self.saves_tab(ui, body),
            Tab::Options => self.options_tab(ui, body),
        }
        if self.tab != Tab::Play {
            self.picker_open = false;
        }
        self.screenshot(&ctx);
    }
}

// ---- shapes ----

/// `tex` over `rect`, filling it and cut to its shape (centred).
/// Where all of `tex` lands when [`cover`] draws it over `rect`.
fn cover_rect(tex: &TextureHandle, rect: Rect) -> Rect {
    let [w, h] = tex.size();
    let scale = (rect.width() / w.max(1) as f32).max(rect.height() / h.max(1) as f32);
    Rect::from_center_size(rect.center(), vec2(w as f32 * scale, h as f32 * scale))
}

fn cover(p: &Painter, tex: &TextureHandle, rect: Rect) {
    let [w, h] = tex.size();
    let image = w as f32 / h.max(1) as f32;
    let target = rect.width() / rect.height().max(1.0);
    let uv = if image > target {
        let u = target / image;
        Rect::from_min_max(pos2((1.0 - u) / 2.0, 0.0), pos2((1.0 + u) / 2.0, 1.0))
    } else {
        let v = image / target;
        Rect::from_min_max(pos2(0.0, (1.0 - v) / 2.0), pos2(1.0, (1.0 + v) / 2.0))
    };
    p.image(tex.id(), rect, uv, Color32::WHITE);
}

fn gradient(p: &Painter, rect: Rect, top: Color32, bottom: Color32) {
    let mut mesh = Mesh::default();
    let i = mesh.vertices.len() as u32;
    for (pos, colour) in [
        (rect.left_top(), top),
        (rect.right_top(), top),
        (rect.right_bottom(), bottom),
        (rect.left_bottom(), bottom),
    ] {
        mesh.colored_vertex(pos, colour);
    }
    mesh.add_triangle(i, i + 1, i + 2);
    mesh.add_triangle(i, i + 2, i + 3);
    p.add(mesh);
}

fn chevron(p: &Painter, c: Pos2, side: f32, colour: Color32) {
    let s = 7.0;
    p.line_segment(
        [c + vec2(-side * s * 0.5, -s), c + vec2(side * s * 0.5, 0.0)],
        Stroke::new(2.5, colour),
    );
    p.line_segment(
        [c + vec2(side * s * 0.5, 0.0), c + vec2(-side * s * 0.5, s)],
        Stroke::new(2.5, colour),
    );
}

fn caret(p: &Painter, c: Pos2, colour: Color32, up: bool) {
    let d = if up { -1.0 } else { 1.0 };
    p.add(egui::Shape::convex_polygon(
        vec![
            c + vec2(-6.0, -3.0 * d),
            c + vec2(6.0, -3.0 * d),
            c + vec2(0.0, 4.0 * d),
        ],
        colour,
        Stroke::NONE,
    ));
}

fn gear(p: &Painter, c: Pos2, r: f32, colour: Color32) {
    for i in 0..8 {
        let a = i as f32 * std::f32::consts::TAU / 8.0;
        let d = vec2(a.cos(), a.sin());
        p.line_segment(
            [c + d * r * 0.6, c + d * (r + 3.0)],
            Stroke::new(3.0, colour),
        );
    }
    p.circle_filled(c, r * 0.85, colour);
    p.circle_filled(c, r * 0.35, SIDEBAR);
}

fn ellipsis(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_owned()
    } else {
        let mut out: String = s.chars().take(max.saturating_sub(3)).collect();
        out.push_str("...");
        out
    }
}

fn file_name(p: &std::path::Path) -> String {
    p.file_name().map_or_else(
        || p.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    )
}

fn write_png(path: &std::path::Path, image: &egui::ColorImage) -> Result<(), String> {
    let file = std::fs::File::create(path).map_err(|e| e.to_string())?;
    let [w, h] = image.size;
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), w as u32, h as u32);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    let mut writer = enc.write_header().map_err(|e| e.to_string())?;
    let bytes: Vec<u8> = image
        .pixels
        .iter()
        .flat_map(|c| {
            let [r, g, b, _] = c.to_srgba_unmultiplied();
            [r, g, b, 255]
        })
        .collect();
    writer.write_image_data(&bytes).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn long_lines_are_cut_with_an_ellipsis() {
        assert_eq!(ellipsis("short", 10), "short");
        assert_eq!(ellipsis("a long line of text", 10), "a long ...");
    }
}
