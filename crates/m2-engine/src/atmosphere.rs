//! The island's light and air: the template database's `AtmosphereBase`
//! settings that the game's shaders take their ambient cube, sunlight
//! strength, rim light and haze from.
//!
//! Three templates carry an `AtmosphereBase`: `Atmosphere` (its `Name`
//! field `Default`, outdoors), `Atmosphere (interior)` (`PMC`) and
//! `Will_atmospheres`. `Will_atmospheres` holds a record named `Default`
//! too (with more bloom and the wind due along a diagonal), so the
//! island's default is the record of the `Atmosphere` template
//! ([`ATMOSPHERE_TEMPLATE`]) where there is one. Field names are the name
//! hashes of the names below (the game's executable lists them in plain
//! text); colours are words of red, green, blue and alpha bytes
//! ([`channels`]), whose alpha is mostly not used (the `...Multiplier`
//! fields scale them instead).
//!
//! Over that default, the layer `vz_atmosphere_regions` places about 50
//! line regions (`rgn_atmo_carmonaisland`, `rgn_atmo_caracas`, ...), each
//! with one or more `AtmosphereBase` records of its own, told apart by
//! `Name` (`Default`, and `night`, `warzone`, ...): the light where the
//! player is ([`AtmosphereRegion`]). Scripts pick a region's record by
//! name (`Graphics.Atmosphere.ChangeLineRegionSetting`): `VzaCon001` puts
//! `rgn_atmo_carmonaislandrain` on `night` for its boat ride.
//!
//! The time of day is each record's `LightTime` (`LightTimeSpeed` is 0 in
//! every record, so the sun stands still): 0.05 to 0.1 in the records of
//! the islands, whose reference footage is at sunset and dusk, 0.2 in the
//! default, 0.3 for the scripts' `afternoon`, 0.9 in Cumana's. The sun is
//! taken to cross the sky on a half circle as `LightTime` goes from 0 to
//! 1, at the horizon at 0 and 1 and overhead at 0.5 ([`SUN_SETS`] says
//! which way): inferred from those values and the footage, not confirmed.
//!
//! # Provenance
//!
//! The field names come from matching name hashes; what each one does is
//! read from its name and from the game's pixel shaders (ambient cube
//! faces in the order +x, -x, +y, -y, +z, -z: the `Default` cube is
//! brightest in face 2 and darkest in face 3, sky above and ground
//! below). The light's colours (ambient, rim, sun) are kept as the game
//! uses them, display values: its shaders light surfaces in display
//! (gamma) space, never decoding their textures, so sunlight and ambient
//! light are summed there and the sum decoded (see
//! [`Lighting::gamma_light`]). A colour alone decoded to linear light
//! gives the same as the game; summed in linear light, shade came out
//! too bright against sunlight.

use std::collections::HashMap;

use m2_assets::layer::{Component, Layer};
use m2_assets::name_hash;
use m2_math::glam::{Vec2, Vec3, Vec4};
use m2_render::{Haze, Lighting, ToneMap};

/// Settings read from an `AtmosphereBase` record.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Atmosphere {
    /// `AmbientCube0`..`5` times their multipliers: light from +x, -x,
    /// +y, -y, +z and -z, display values.
    pub ambient_cube: [Vec3; 6],
    /// `AmbientColor` times its multiplier, display values: what it does
    /// is unknown. `M2_AMBIENT=color` scales the ambient cube by it, to
    /// try that reading.
    pub ambient_color: Vec3,
    /// `LightIntensity`: the sunlight's strength.
    pub light_intensity: f32,
    /// `RimColor` times `RimColorMultiplier`, display values.
    pub rim: Vec3,
    /// `RimColor`'s alpha byte, taken as the rim light's threshold (the
    /// shaders' `rimLightColor.w`): inferred, the other colours' alpha
    /// bytes have no use.
    pub rim_threshold: f32,
    /// `ScatterInscatteringMultiplier`, `ScatterExtinctionMultiplier`,
    /// `ScatterBetaMieMultiplier` and `ScatterHgG`: the haze.
    pub inscattering: f32,
    pub extinction: f32,
    pub mie: f32,
    pub hg_g: f32,
    /// `Tone_R`, `Tone_G`, `Tone_B`: among the cloud settings (between
    /// `ShadowK` and `LightBlend`), so taken as the clouds' tint.
    pub tone: Vec3,
    /// `BloomAmount`, `BloomMultiplier`, `BloomThreshold`,
    /// `BloomBlurRadius` (the radius is not used yet).
    pub bloom: [f32; 4],
    /// `LightTime` and `LightTimeSpeed`: the time of day, probably.
    pub light_time: [f32; 2],
    /// `Coverage`: how much of the sky is cloud.
    pub coverage: f32,
    /// `Layer1_Scale`..`Layer3_Scale`: the cloud noise layers' scales.
    pub cloud_scales: [f32; 3],
    /// `WindDirection_X`, `WindDirection_Y`: the way the clouds drift.
    pub wind: [f32; 2],
    /// `LightMultiplier_X`..`_Z`: the sunlight's colour (1, 0.85, 0.8 in
    /// the islands' evening records).
    pub light_color: Vec3,
    /// `InterpolateTime`: seconds the change to these settings takes.
    pub interpolate_time: f32,
    /// `Water0`..`2_Color1`/`Color2`: the water's colour and opacity by
    /// how much of it is looked through, as four stops (each band's
    /// colours, joined end to end; every record's bands tile the map in
    /// thirds and its colours' alpha bytes run 0 to 255 with them), in
    /// linear RGB and opacity.
    pub water_ramp: Option<[Vec4; 4]>,
    /// `BloomTargetLuminance`, `BloomAdaptiveLuminance*` and
    /// `BloomContast*` (the game's spelling): its adaptation and tone map.
    pub tone_map: Option<ToneMap>,
    /// `Gradient0_Color2`: the middle of the post pass's monochrome ramp
    /// (a display colour) and, in its alpha byte, how much of the frame
    /// the ramp takes.
    pub monochrome: Option<Vec4>,
}

/// Which way the sun stands at `LightTime` 0 (and 0.1, low in the
/// evening), over the horizon: 60 degrees from `+z` towards `+x`, so
/// that from the start of `VzaCon001` it stands behind the island 20 to
/// 25 degrees left of its searchlights, as the footage of the boat ride
/// has it. Inferred from the footage.
pub const SUN_SETS: Vec3 = Vec3::new(0.87, 0.0, 0.49);

/// The lowest the sun is drawn, in radians over the horizon, so that low
/// `LightTime`s still light the ground a little.
const SUN_LOWEST: f32 = 0.05;

/// The template database id of the `Atmosphere` template, whose one
/// record is the outdoor `Default`.
pub const ATMOSPHERE_TEMPLATE: u32 = 0x9000_00cf;

impl Atmosphere {
    /// The record whose `Name` field is `name` (`Default` outdoors), from
    /// the template database `db`: of those, the [`ATMOSPHERE_TEMPLATE`]'s,
    /// else the first.
    pub fn read(db: &Layer<'_>, name: &str) -> Option<Self> {
        let c = db.component("AtmosphereBase")?;
        let field = |rec: &[u8], field: &str| -> Option<u32> {
            let f = c.fields.iter().find(|f| f.name_hash == name_hash(field))?;
            let at = usize::from(f.offset);
            rec.get(at..at + 4)
                .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        };
        let records = c.records()?;
        let named: Vec<&(u32, &[u8])> = records
            .iter()
            .filter(|(_, r)| field(r, "Name") == Some(name_hash(name)))
            .collect();
        let (_, rec) = named
            .iter()
            .find(|(id, _)| *id == ATMOSPHERE_TEMPLATE)
            .or(named.first())?;
        Some(Self::from_record(c, rec))
    }

    /// The settings of one `AtmosphereBase` record `rec` of component `c`.
    pub fn from_record(c: &Component<'_>, rec: &[u8]) -> Self {
        let field = |field: &str| -> Option<u32> {
            let f = c.fields.iter().find(|f| f.name_hash == name_hash(field))?;
            let at = usize::from(f.offset);
            rec.get(at..at + 4)
                .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        };
        let float = |name: &str| field(name).map(f32::from_bits);
        let colour = |name: &str| {
            let v = field(name)?;
            let scale = float(&format!("{name}Multiplier")).unwrap_or(1.0);
            let [r, g, b, _] = channels(v, Lit::Light);
            Some(Vec3::new(r, g, b) * scale)
        };
        let mut ambient_cube = [Vec3::splat(0.2); 6];
        for (i, face) in ambient_cube.iter_mut().enumerate() {
            if let Some(c) = colour(&format!("AmbientCube{i}")) {
                *face = c;
            }
        }
        Self {
            ambient_cube,
            ambient_color: colour("AmbientColor").unwrap_or(Vec3::ONE),
            light_intensity: float("LightIntensity").unwrap_or(1.0),
            rim: colour("RimColor").unwrap_or(Vec3::ZERO),
            rim_threshold: field("RimColor").map_or(0.0, |v| channels(v, Lit::Light)[3]),
            inscattering: float("ScatterInscatteringMultiplier").unwrap_or(0.0),
            extinction: float("ScatterExtinctionMultiplier").unwrap_or(0.0),
            mie: float("ScatterBetaMieMultiplier").unwrap_or(0.0),
            hg_g: float("ScatterHgG").unwrap_or(0.0),
            tone: Vec3::new(
                float("Tone_R").unwrap_or(1.0),
                float("Tone_G").unwrap_or(1.0),
                float("Tone_B").unwrap_or(1.0),
            ),
            bloom: [
                "BloomAmount",
                "BloomMultiplier",
                "BloomThreshold",
                "BloomBlurRadius",
            ]
            .map(|n| float(n).unwrap_or(0.0)),
            light_time: ["LightTime", "LightTimeSpeed"].map(|n| float(n).unwrap_or(0.0)),
            coverage: float("Coverage").unwrap_or(0.0),
            cloud_scales: ["Layer1_Scale", "Layer2_Scale", "Layer3_Scale"]
                .map(|n| float(n).unwrap_or(1.0)),
            wind: ["WindDirection_X", "WindDirection_Y"].map(|n| float(n).unwrap_or(0.0)),
            light_color: Vec3::new(
                float("LightMultiplier_X").unwrap_or(1.0),
                float("LightMultiplier_Y").unwrap_or(1.0),
                float("LightMultiplier_Z").unwrap_or(1.0),
            ),
            interpolate_time: float("InterpolateTime").unwrap_or(0.0),
            monochrome: field("Gradient0_Color2").map(|v| Vec4::from_array(channels(v, Lit::Post))),
            tone_map: (|| {
                let f = |names: &[&str]| names.iter().find_map(|n| float(n));
                Some(ToneMap {
                    target: f(&["BloomTargetLuminance"])?,
                    retention: f(&["BloomAdaptiveLuminancePercent"])?,
                    min: f(&["BloomAdaptiveLuminanceMin", "AdaptiveLuminanceMin"])?,
                    max: f(&["BloomAdaptiveLuminanceMax", "AdaptiveLuminanceMax"])?,
                    min_key: f(&[
                        "BloomAdaptiveLuminanceMinKeyScale",
                        "AdaptiveLuminanceMinKeyScale",
                        "BloomMinKeyScale",
                        "MinKeyScale",
                    ])?,
                    max_key: f(&[
                        "BloomAdaptiveLuminanceMaxKeyScale",
                        "AdaptiveLuminanceMaxKeyScale",
                        "BloomMaxKeyScale",
                        "MaxKeyScale",
                    ])?,
                    contrast_limit: f(&["BloomContastLimit", "BloomContrastLimit"])?,
                    contrast: f(&["BloomContastMultiplier", "BloomContrastMultiplier"])?,
                    // `M2_LUMINANCE=log`: the geometric mean, as before.
                    log_average: env_is("M2_LUMINANCE", "log"),
                })
            })(),
            water_ramp: {
                let stop = |name: &str| {
                    let v = field(name)?;
                    let [r, g, b, a] = channels(v, Lit::Post);
                    Some(Vec4::new(
                        srgb_to_linear(r),
                        srgb_to_linear(g),
                        srgb_to_linear(b),
                        a,
                    ))
                };
                (|| {
                    Some([
                        stop("Water0_Color1")?,
                        stop("Water0_Color2")?,
                        stop("Water1_Color2")?,
                        stop("Water2_Color2")?,
                    ])
                })()
            },
        }
    }

    /// Settings `t` of the way from `self` to `to` (0 is `self`).
    pub fn lerp(&self, to: &Self, t: f32) -> Self {
        let t = t.clamp(0.0, 1.0);
        let f = |a: f32, b: f32| a + (b - a) * t;
        let v = |a: Vec3, b: Vec3| a.lerp(b, t);
        Self {
            ambient_cube: std::array::from_fn(|i| v(self.ambient_cube[i], to.ambient_cube[i])),
            ambient_color: v(self.ambient_color, to.ambient_color),
            light_intensity: f(self.light_intensity, to.light_intensity),
            rim: v(self.rim, to.rim),
            rim_threshold: f(self.rim_threshold, to.rim_threshold),
            inscattering: f(self.inscattering, to.inscattering),
            extinction: f(self.extinction, to.extinction),
            mie: f(self.mie, to.mie),
            hg_g: f(self.hg_g, to.hg_g),
            tone: v(self.tone, to.tone),
            bloom: std::array::from_fn(|i| f(self.bloom[i], to.bloom[i])),
            light_time: std::array::from_fn(|i| f(self.light_time[i], to.light_time[i])),
            coverage: f(self.coverage, to.coverage),
            cloud_scales: std::array::from_fn(|i| f(self.cloud_scales[i], to.cloud_scales[i])),
            wind: std::array::from_fn(|i| f(self.wind[i], to.wind[i])),
            light_color: v(self.light_color, to.light_color),
            interpolate_time: to.interpolate_time,
            tone_map: match (self.tone_map, to.tone_map) {
                (Some(a), Some(b)) => Some(ToneMap {
                    target: f(a.target, b.target),
                    min: f(a.min, b.min),
                    max: f(a.max, b.max),
                    min_key: f(a.min_key, b.min_key),
                    max_key: f(a.max_key, b.max_key),
                    retention: f(a.retention, b.retention),
                    contrast_limit: f(a.contrast_limit, b.contrast_limit),
                    contrast: f(a.contrast, b.contrast),
                    log_average: b.log_average,
                }),
                (_, b) => b,
            },
            monochrome: match (self.monochrome, to.monochrome) {
                (Some(a), Some(b)) => Some(a.lerp(b, t)),
                (_, b) => b,
            },
            water_ramp: match (self.water_ramp, to.water_ramp) {
                (Some(a), Some(b)) => Some(std::array::from_fn(|i| a[i].lerp(b[i], t))),
                (_, b) => b,
            },
        }
    }

    /// The way the sunlight travels at this `LightTime`: from a sun on
    /// a half circle over [`SUN_SETS`] (see the module's notes).
    pub fn sun_direction(&self) -> Vec3 {
        let half = std::f32::consts::PI * self.light_time[0].clamp(0.0, 1.0);
        let up = half.sin().max(SUN_LOWEST.sin());
        -(SUN_SETS.normalize() * half.cos() + Vec3::Y * up)
    }

    /// Puts the ambient cube, sunlight strength, rim light, haze, bloom,
    /// grade and cloud settings into `lighting` (not the cloud noise
    /// texture, which the world loads).
    pub fn apply(&self, lighting: &mut Lighting) {
        let decode = |c: Vec3| c.max(Vec3::ZERO).powf(GAMMA);
        let ambient = if env_is("M2_AMBIENT", "color") {
            self.ambient_color
        } else {
            Vec3::ONE
        };
        lighting.ambient_cube = self.ambient_cube.map(|c| decode(c * ambient));
        lighting.rim_color = decode(self.rim);
        lighting.rim_threshold = self.rim_threshold;
        lighting.sun_direction = self.sun_direction();
        // `M2_LIGHTING=linear` sums the light in linear light, with the
        // sun's colour taken as linear, as before, for comparison.
        lighting.gamma_light = gamma_light();
        let sun = self.light_color * self.light_intensity;
        lighting.sun_color = if lighting.gamma_light {
            decode(sun)
        } else {
            sun
        };
        lighting.haze = self.haze();
        // `BloomAmount` times `BloomMultiplier` above `BloomThreshold`
        // (inferred from the names).
        lighting.bloom = self.bloom[0] * self.bloom[1];
        lighting.bloom_threshold = self.bloom[2];
        lighting.clouds.tone = self.tone;
        lighting.clouds.coverage = self.coverage;
        lighting.clouds.scales = self.cloud_scales;
        lighting.clouds.wind = self.wind;
        lighting.water_ramp = self.water_ramp;
        lighting.tone_map = self.tone_map;
        // The atmospheres' monochrome ramps are not applied: the post
        // shader blends towards one, but at their own colours' alpha
        // (a quarter to a half) they turned the day red (read as red,
        // green, blue, alpha bytes) or blue (the other way) against footage
        // on PC round E, where without them the AMX-30's red to green and
        // the jungle's hue matched. The blend is likely set by code, for
        // effects; inferred. `M2_MONO=on` applies them.
        lighting.monochrome = self.monochrome.filter(|_| env_is("M2_MONO", "on"));
    }

    /// The haze these settings describe: Hoffman and Preetham's standard
    /// Rayleigh and Mie scattering (Mie for a turbidity of 2), scaled by
    /// the `Scatter...` multipliers. That the game's multipliers apply to
    /// these standard coefficients is inferred: its shaders take the
    /// products from code. On their own they haze land a few percent per
    /// kilometre; [`HAZE_DENSITY`] thickens that to match footage.
    pub fn haze(&self) -> Haze {
        if self.inscattering <= 0.0 {
            return Haze::default();
        }
        Haze {
            rayleigh: RAYLEIGH,
            mie: MIE * self.mie,
            g: self.hg_g,
            inscattering: self.inscattering,
            extinction: self.extinction,
            density: HAZE_DENSITY,
            ..Haze::default()
        }
    }
}

/// A line region with atmospheres of its own (`rgn_atmo_carmonaisland`):
/// where the player is inside its outline, its chosen record is the light.
#[derive(Clone, Debug, PartialEq)]
pub struct AtmosphereRegion {
    pub entity: u32,
    pub name: String,
    /// Corners on the ground, world `(x, z)`.
    pub outline: Vec<Vec2>,
    /// Square metres inside the outline: where regions overlap, the
    /// smallest is taken (inferred: it is the most particular).
    pub area: f32,
    /// Its records by `Name` hash; [`default_setting`](Self::default_setting)
    /// is the one shown until a script picks another.
    pub settings: Vec<(u32, Atmosphere)>,
}

impl AtmosphereRegion {
    /// A region of `settings` whose outline is `outline`.
    pub fn new(
        entity: u32,
        name: String,
        outline: Vec<Vec2>,
        settings: Vec<(u32, Atmosphere)>,
    ) -> Self {
        let n = outline.len();
        let area = (0..n)
            .map(|i| outline[i].perp_dot(outline[(i + 1) % n]))
            .sum::<f32>()
            .abs()
            * 0.5;
        Self {
            entity,
            name,
            outline,
            area,
            settings,
        }
    }

    /// Whether the column over `(x, z)` is inside.
    pub fn contains(&self, x: f32, z: f32) -> bool {
        let mut inside = false;
        let n = self.outline.len();
        for i in 0..n {
            let (a, b) = (self.outline[i], self.outline[(i + n - 1) % n]);
            if (a.y > z) != (b.y > z) && x < (b.x - a.x) * (z - a.y) / (b.y - a.y) + a.x {
                inside = !inside;
            }
        }
        inside
    }

    /// The record named `Default`, else the first.
    pub fn default_setting(&self) -> usize {
        let default = name_hash("Default");
        self.settings
            .iter()
            .position(|(n, _)| *n == default)
            .unwrap_or(0)
    }
}

/// Reads the regions of a layer with `AtmosphereBase` and `LineRegion`
/// components: each region entity, its records and the shape its
/// `LineRegion` names (outlines are read by `shape`, from the shape's
/// hash, in the entity's own space).
pub fn read_regions(
    layer: &Layer<'_>,
    mut shape: impl FnMut(u32) -> Option<Vec<[f32; 2]>>,
) -> Vec<AtmosphereRegion> {
    let Some(c) = layer.component("AtmosphereBase") else {
        return Vec::new();
    };
    let Some(records) = c.records() else {
        return Vec::new();
    };
    let name_field = c.fields.iter().find(|f| f.name_hash == name_hash("Name"));
    let mut settings: HashMap<u32, Vec<(u32, Atmosphere)>> = HashMap::new();
    for (entity, rec) in records {
        let name = name_field
            .and_then(|f| rec.get(usize::from(f.offset)..usize::from(f.offset) + 4))
            .map_or(0, |b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]));
        settings
            .entry(entity)
            .or_default()
            .push((name, Atmosphere::from_record(c, rec)));
    }
    let lines = layer.hashes("LineRegion");
    let placements = layer.placements();
    let names = layer.names();
    let mut out: Vec<AtmosphereRegion> = settings
        .into_iter()
        .filter_map(|(entity, settings)| {
            let p = placements.get(&entity)?;
            let corners = shape(*lines.get(&entity)?)?;
            let at = Vec3::from(p.position);
            let [x, y, z, w] = p.rotation;
            let q = m2_math::glam::Quat::from_xyzw(x, y, z, w).normalize();
            let outline = corners
                .iter()
                .map(|&[cx, cz]| {
                    let v = at + q * Vec3::new(cx, 0.0, cz);
                    Vec2::new(v.x, v.z)
                })
                .collect();
            let name = names.get(&entity).copied().unwrap_or("").to_owned();
            Some(AtmosphereRegion::new(entity, name, outline, settings))
        })
        .collect();
    out.sort_unstable_by_key(|r| r.entity);
    out
}

/// The light the player is in as it changes: the record of the region
/// they stand in (the island's default outside them all), eased into
/// over the new record's `InterpolateTime`.
#[derive(Clone, Debug, Default)]
pub struct AtmosphereMix {
    /// Changes happen at once rather than easing in (the scripts'
    /// `EnableImmediatelyChangeMode`, on while a mission loads).
    pub immediate: bool,
    /// Records scripts chose, by region entity.
    chosen: HashMap<u32, u32>,
    /// The light eased from.
    from: Option<Atmosphere>,
    /// The light eased towards, and the (region, record) indices it is
    /// (`None` outside every region).
    to: Option<(Option<(usize, usize)>, Atmosphere)>,
    /// Seconds into the change, and its length.
    t: f32,
    length: f32,
}

impl AtmosphereMix {
    /// Puts region `entity` (by its entity id, else its name) on its record
    /// named `setting`; false when there is no such region or record.
    pub fn choose(&mut self, regions: &[AtmosphereRegion], region: u32, setting: &str) -> bool {
        let wanted = name_hash(setting);
        match regions.iter().find(|r| r.entity == region) {
            Some(r) if r.settings.iter().any(|(n, _)| *n == wanted) => {
                self.chosen.insert(r.entity, wanted);
                true
            }
            _ => false,
        }
    }

    /// The region and record over `at`, if any: of the regions holding it,
    /// the smallest.
    pub fn over(&self, regions: &[AtmosphereRegion], at: Vec3) -> Option<(usize, usize)> {
        let (i, r) = regions
            .iter()
            .enumerate()
            .filter(|(_, r)| r.contains(at.x, at.z))
            .min_by(|a, b| a.1.area.total_cmp(&b.1.area))?;
        let setting = self
            .chosen
            .get(&r.entity)
            .and_then(|n| r.settings.iter().position(|(s, _)| s == n))
            .unwrap_or_else(|| r.default_setting());
        Some((i, setting))
    }

    /// Moves on by `dt` seconds with the player at `at`: when the region or
    /// its record changes, eases towards the new one. Returns the new
    /// region and record when they changed (for the log).
    pub fn update(
        &mut self,
        default: &Atmosphere,
        regions: &[AtmosphereRegion],
        at: Vec3,
        dt: f32,
    ) -> Option<Option<(usize, usize)>> {
        let key = self.over(regions, at);
        let target = key.map_or(*default, |(r, s)| regions[r].settings[s].1);
        self.t += dt;
        match &self.to {
            Some((k, _)) if *k == key => None,
            Some(_) => {
                self.from = Some(self.current(default));
                self.length = if self.immediate {
                    0.0
                } else {
                    target.interpolate_time.max(0.0)
                };
                self.t = 0.0;
                self.to = Some((key, target));
                Some(key)
            }
            None => {
                // The first look: no easing in.
                self.from = None;
                self.to = Some((key, target));
                Some(key)
            }
        }
    }

    /// The light now.
    pub fn current(&self, default: &Atmosphere) -> Atmosphere {
        let Some((_, to)) = &self.to else {
            return *default;
        };
        match &self.from {
            Some(from) if self.t < self.length => from.lerp(to, self.t / self.length),
            _ => *to,
        }
    }
}

/// Air near the ground against the standard coefficients: fitted so land
/// 2 km away is about half haze, as in PC footage of the game (the game's
/// own density comes from code).
const HAZE_DENSITY: f32 = 4.0;
/// Rayleigh scattering of air per metre at 650, 570 and 475 nm.
const RAYLEIGH: Vec3 = Vec3::new(6.95e-6, 1.18e-5, 2.44e-5);
/// Mie scattering per metre at the same wavelengths for a turbidity of 2,
/// before the game's multiplier: `0.434 c pi (2 pi / lambda)^2 K` with
/// `c = (0.6544 T - 0.6510) 1e-16`.
const MIE: Vec3 = Vec3::new(5.75e-3, 7.39e-3, 1.045e-2);

/// What a record colour is for: light on surfaces (ambient, rim) or the
/// water and post pass (depth colour map, monochrome ramp).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Lit {
    Light,
    Post,
}

/// Whether environment variable `name` is set to `value`.
fn env_is(name: &str, value: &str) -> bool {
    std::env::var(name).is_ok_and(|v| v == value)
}

/// The game's display gamma: its tone map encodes and decodes with it.
const GAMMA: f32 = 2.2;

/// Whether light is summed in display values, as the game's shaders do
/// (`M2_LIGHTING=linear` for linear light).
fn gamma_light() -> bool {
    static GAMMA_LIGHT: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *GAMMA_LIGHT.get_or_init(|| std::env::var("M2_LIGHTING").map_or(true, |v| v != "linear"))
}

/// A record colour word's red, green, blue and alpha, 0 to 1. The light's
/// colours (ambient and rim) are read as `0xAARRGGBB`, the water's and
/// post pass's as the bytes red, green, blue, alpha (red the low byte):
/// read all as `0xAARRGGBB` the islands' night water and grade are warm
/// against their blue cloud tone and footage and the daytime grades blue;
/// read all the other way the daytime ambient light is sky blue, the town
/// pink-grey and the jungle grey against footage. Read this way the town
/// and jungle are warm and the night blue (PC round F). Inferred;
/// `M2_COLOURS=argb` and `M2_COLOURS=rgba` read them all one way, for
/// comparison.
fn channels(v: u32, lit: Lit) -> [f32; 4] {
    static MODE: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    let mode = MODE.get_or_init(|| std::env::var("M2_COLOURS").unwrap_or_default());
    let argb = mode == "argb" || (mode != "rgba" && lit == Lit::Light);
    let byte = |shift: u32| ((v >> shift) & 0xff) as f32 / 255.0;
    if argb {
        [byte(16), byte(8), byte(0), byte(24)]
    } else {
        [byte(0), byte(8), byte(16), byte(24)]
    }
}

/// A gamma-encoded (sRGB) channel in 0..1 as linear light.
fn srgb_to_linear(c: f32) -> f32 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain(light_time: f32, interpolate_time: f32) -> Atmosphere {
        Atmosphere {
            ambient_cube: [Vec3::splat(0.2); 6],
            ambient_color: Vec3::ONE,
            light_intensity: 1.0,
            rim: Vec3::ZERO,
            rim_threshold: 0.0,
            inscattering: 0.0,
            extinction: 0.0,
            mie: 0.0,
            hg_g: 0.0,
            tone: Vec3::ONE,
            bloom: [0.0; 4],
            light_time: [light_time, 0.0],
            coverage: 0.0,
            cloud_scales: [1.0; 3],
            wind: [0.0; 2],
            light_color: Vec3::ONE,
            interpolate_time,
            water_ramp: None,
            tone_map: None,
            monochrome: None,
        }
    }

    fn square(
        entity: u32,
        x: f32,
        z: f32,
        half: f32,
        settings: Vec<(u32, Atmosphere)>,
    ) -> AtmosphereRegion {
        let outline = [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)]
            .map(|(a, b)| Vec2::new(x + a * half, z + b * half))
            .to_vec();
        AtmosphereRegion::new(entity, format!("r{entity}"), outline, settings)
    }

    #[test]
    fn the_sun_rises_with_light_time() {
        let low = plain(0.1, 0.0).sun_direction();
        let high = plain(0.5, 0.0).sun_direction();
        // Light travels down, from a sun over SUN_SETS when it is low.
        assert!(low.y < 0.0 && high.y < 0.0);
        assert!(low.normalize().dot(-SUN_SETS.normalize()) > 0.9);
        assert!(high.normalize().y < -0.99);
    }

    #[test]
    fn the_smallest_region_holding_the_player_lights_them() {
        let default = plain(0.2, 0.0);
        let night = name_hash("night");
        let regions = vec![
            square(
                1,
                0.0,
                0.0,
                100.0,
                vec![(name_hash("Default"), plain(0.9, 0.0))],
            ),
            square(
                2,
                10.0,
                10.0,
                20.0,
                vec![
                    (name_hash("Default"), plain(0.1, 0.0)),
                    (night, plain(0.1, 4.0)),
                ],
            ),
        ];
        assert_eq!(regions[0].area, 40_000.0);
        let mut mix = AtmosphereMix::default();
        mix.update(&default, &regions, Vec3::new(500.0, 0.0, 0.0), 0.1);
        assert_eq!(mix.current(&default).light_time[0], 0.2);
        mix.update(&default, &regions, Vec3::new(-50.0, 0.0, 0.0), 0.1);
        assert_eq!(mix.current(&default).light_time[0], 0.9);
        mix.update(&default, &regions, Vec3::new(15.0, 0.0, 15.0), 0.1);
        assert_eq!(mix.current(&default).light_time[0], 0.1);
        // A script's pick is eased into over its record's time.
        assert!(mix.choose(&regions, 2, "night"));
        assert!(!mix.choose(&regions, 2, "warzone"));
        mix.update(&default, &regions, Vec3::new(15.0, 0.0, 15.0), 0.1);
        mix.update(&default, &regions, Vec3::new(15.0, 0.0, 15.0), 2.0);
        assert_eq!(mix.over(&regions, Vec3::new(15.0, 0.0, 15.0)), Some((1, 1)));
        assert_eq!(mix.current(&default).interpolate_time, 4.0);
    }
}
