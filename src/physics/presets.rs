//! Библиотека пресетов и генераторы стабильных конфигураций.
//!
//! Пресеты сгруппированы по физическому смыслу:
//! * **Хореографии** — точные периодические решения задачи трёх тел
//!   (восьмёрка Ченчинера–Монтгомери и её расширение с далёкой планетой);
//! * **Планетные системы** — кеплеровские системы, разнесённые по критерию
//!   взаимных радиусов Хилла (Gladman 1993; Chambers, Wetherill & Boss 1996):
//!   соседние орбиты должны отстоять больше чем на `2√3 ≈ 3.46` взаимных
//!   радиуса Хилла, в генераторах берётся запас `K = 4.8…6`;
//! * **Спутники** — луны внутри 0.3–0.4 радиуса Хилла планеты, движение
//!   прямое (ретроградные луны на таких расстояниях часто неустойчивы);
//! * **Кратные** — иерархические системы: отношение внешней и внутренней
//!   полуосей > 10 (критерий Мардлинга–Орсета для тройных), S- и P-типы
//!   планет в двойных;
//! * **Лагранж** — троянцы в L4/L5: линейно устойчивы при μ < 0.0385;
//! * **Облака** — демонстрационные диски.
//!
//! Все генераторы детерминированы (свой xorshift-сид), а в конце каждого
//! сценария система центрируется и её полный импульс обнуляется — камера
//! не уезжает, а относительная динамика не меняется.

use std::f64::consts::{FRAC_PI_3, TAU};

use super::sim::{Body, Integrator, Scenario};
use super::vec::{Rng, Vec3};

/// Категория пресета для галереи и меню.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PresetCategory {
    Choreography,
    Planetary,
    Satellites,
    Multiple,
    Lagrange,
    Cloud,
}

impl PresetCategory {
    pub fn name(self) -> &'static str {
        match self {
            PresetCategory::Choreography => "Хореографии",
            PresetCategory::Planetary => "Планетные системы",
            PresetCategory::Satellites => "Спутники и кольца",
            PresetCategory::Multiple => "Кратные системы",
            PresetCategory::Lagrange => "Точки Лагранжа",
            PresetCategory::Cloud => "Облака и диски",
        }
    }

    pub fn hint(self) -> &'static str {
        match self {
            PresetCategory::Choreography => "Точные периодические решения задачи трёх тел",
            PresetCategory::Planetary => "Кеплеровские системы, устойчивые по Хиллу",
            PresetCategory::Satellites => "Планеты с лунами и кольцами",
            PresetCategory::Multiple => "Двойные, тройные и кратные звёзды",
            PresetCategory::Lagrange => "Троянцы в L4/L5 и окрестности точек равновесия",
            PresetCategory::Cloud => "Протопланетные и звёздные диски",
        }
    }

    pub const ALL: [PresetCategory; 6] = [
        PresetCategory::Choreography,
        PresetCategory::Planetary,
        PresetCategory::Satellites,
        PresetCategory::Multiple,
        PresetCategory::Lagrange,
        PresetCategory::Cloud,
    ];
}

/// Насколько конфигурация устойчива (по литературе и численным критериям).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stability {
    /// Линейно/иерархически устойчива на длинных временах.
    Stable,
    /// Долгоживущая: устойчива по принятым критериям, но не доказана строго.
    LongLived,
    /// Периодическая, но слабо неустойчивая (уходит за несколько периодов).
    #[allow(dead_code)]
    WeaklyUnstable,
    /// Демонстрация без гарантий устойчивости.
    Demo,
}

impl Stability {
    pub fn label(self) -> &'static str {
        match self {
            Stability::Stable => "устойчива",
            Stability::LongLived => "долгоживущая",
            Stability::WeaklyUnstable => "слабо неустойчива",
            Stability::Demo => "демо",
        }
    }

    pub fn hint(self) -> &'static str {
        match self {
            Stability::Stable => "Сохраняется на длинных временах",
            Stability::LongLived => "Устойчива по критериям Хилла/иерархии, проверена численно",
            Stability::WeaklyUnstable => "Периодическая орбита; малые возмущения растут",
            Stability::Demo => "Наглядная, но не гарантированно устойчивая конфигурация",
        }
    }
}

/// Рекомендуемые параметры интегрирования для пресета.
#[derive(Clone, Copy, Debug)]
pub struct Recommended {
    pub integrator: Integrator,
    pub dt: f64,
    pub softening: f64,
    pub speed: f64,
}

/// Описание пресета для меню и галереи.
#[derive(Clone, Copy, Debug)]
pub struct PresetInfo {
    pub name: &'static str,
    pub category: PresetCategory,
    pub description: &'static str,
    pub stability: Stability,
    pub recommended: Option<Recommended>,
}

impl Scenario {
    /// Полное описание пресета.
    pub fn info(self) -> PresetInfo {
        use PresetCategory as C;
        use Stability as S;
        let info = |name, category, description, stability, recommended| PresetInfo {
            name,
            category,
            description,
            stability,
            recommended,
        };
        match self {
            Scenario::FigureEight => info(
                "Восьмёрка (3 тела)",
                C::Choreography,
                "Знаменитая фигура-восьмёрка Ченчинера–Монтгомери: три равные массы \
                 гонятся друг за другом по одной кривой. Линейно устойчива.",
                S::Stable,
                Some(Recommended {
                    integrator: Integrator::PolynomialRK4,
                    dt: 0.016,
                    softening: 1.0,
                    speed: 3.0,
                }),
            ),
            Scenario::FigureEightPlanet => info(
                "Восьмёрка + планета",
                C::Choreography,
                "Та же восьмёрка, но вокруг неё по широкой орбите идёт лёгкая планета. \
                 Приливное возмущение от планеты ничтожно, хореография сохраняется.",
                S::Stable,
                Some(Recommended {
                    integrator: Integrator::PolynomialRK4,
                    dt: 0.016,
                    softening: 1.0,
                    speed: 3.0,
                }),
            ),
            Scenario::SolarSystem => info(
                "Солнечная система",
                C::Planetary,
                "Звезда и 10 планет на почти круговых орбитах с малыми взаимными \
                 наклонениями. Классическая демонстрация кеплеровского движения.",
                S::LongLived,
                None,
            ),
            Scenario::StableSystem => info(
                "Стабильная система (8 планет)",
                C::Planetary,
                "Восемь планет, разнесённых на K = 5 взаимных радиусов Хилла. \
                 Критическое значение для круговых копланарных орбит — 2√3 ≈ 3.46.",
                S::Stable,
                None,
            ),
            Scenario::HillCompact => info(
                "Компактная система (6 планет)",
                C::Planetary,
                "Плотная, но хилл-устойчивая система: K = 4.8, массы 0.05–0.35 M⊕. \
                 Хороший тест на точность интегратора при близких орбитах.",
                S::Stable,
                None,
            ),
            Scenario::HillGrand => info(
                "Широкая система (9 планет)",
                C::Planetary,
                "Девять планет с большим запасом устойчивости K = 6 и заметными \
                 промежутками. Удобно для обзора и включения тепловой карты.",
                S::Stable,
                None,
            ),
            Scenario::KeplerLike => info(
                "Кеплер-11-подобная",
                C::Planetary,
                "Шесть компактных планет с периодами, растущими примерно как 1:1.5:2.3… \
                 Прообраз горячих многопланетных систем у солнцеподобной звезды.",
                S::LongLived,
                None,
            ),
            Scenario::TrappistLike => info(
                "TRAPPIST-1-подобная",
                C::Planetary,
                "Семь малых планет у холодного красного карлика. Орбиты разнесены \
                 по Хиллу, наклонения минимальны — система читается как «горошины».",
                S::LongLived,
                None,
            ),
            Scenario::PulsarPlanets => info(
                "Пульсарные планеты",
                C::Planetary,
                "Три лёгкие планеты вокруг компактной массивной звезды. \
                 Проверка устойчивости при больших отношениях масс.",
                S::Stable,
                None,
            ),
            Scenario::GasGiants => info(
                "Четыре газовых гиганта",
                C::Planetary,
                "Массивные планеты на широких орбитах с запасом по взаимному \
                 радиусу Хилла. Хорошо видны волны плотности на тепловой карте.",
                S::LongLived,
                None,
            ),
            Scenario::EarthMoon => info(
                "Земля–Луна",
                C::Satellites,
                "Планета с крупной луной на 0.42 радиуса Хилла. Прямое движение, \
                 наклонение совпадает с орбитальной плоскостью планеты.",
                S::Stable,
                None,
            ),
            Scenario::MoonSystem => info(
                "Планета с 4 лунами",
                C::Satellites,
                "Галилеевоподобная система: четыре луны на 0.08–0.30 радиуса Хилла. \
                 Соседние луны разнесены больше чем на 4 взаимных радиуса Хилла.",
                S::Stable,
                None,
            ),
            Scenario::StableRing => info(
                "Кольцо планеты",
                C::Satellites,
                "Планета-гигант и 96 ледяных частиц в кольце шириной 45–120. \
                 Частицы движутся по круговым орбитам внутри сферы Хилла.",
                S::LongLived,
                None,
            ),
            Scenario::BinaryStar => info(
                "Двойная звезда + планеты",
                C::Multiple,
                "Две звезды по 800 масс в тесной паре и шесть планет P-типа \
                 на широких орбитах (a > 2.5–3 ширины пары).",
                S::Stable,
                None,
            ),
            Scenario::BinaryUnequal => info(
                "Неравная двойная",
                C::Multiple,
                "Двойная с отношением масс 2.8 : 1. Барицентр смещён к тяжёлой \
                 звезде — удобно смотреть на кривые скоростей.",
                S::Stable,
                None,
            ),
            Scenario::HierarchicalTriple => info(
                "Иерархическая тройная",
                C::Multiple,
                "Тесная двойная (800+800) и далёкая третья звезда на расстоянии \
                 13 ширин внутренней пары — надёжно устойчивая иерархия.",
                S::Stable,
                None,
            ),
            Scenario::HierarchicalQuad => info(
                "Двойная двойная (4 звезды)",
                C::Multiple,
                "Две тесные пары обращаются друг вокруг друга; внешняя полуось \
                 в 18 раз больше внутренней. Плоскости пар слегка наклонены.",
                S::Stable,
                None,
            ),
            Scenario::CircumbinaryPlanets => info(
                "Планеты вокруг двойной",
                C::Multiple,
                "P-тип: три планеты обращаются вокруг всей двойной. Первая орбита \
                 в 3.7 раза шире пары, что выше критического радиуса Холмана–Вигерта.",
                S::LongLived,
                None,
            ),
            Scenario::StypePlanet => info(
                "S-тип в двойной",
                C::Multiple,
                "Планета обращается вокруг одной звезды тесной двойной на 0.18 \
                 радиуса Хилла этой звезды — классический S-тип.",
                S::Stable,
                None,
            ),
            Scenario::PlutoCharon => info(
                "Плутон–Харон + луны",
                C::Multiple,
                "Двойная планета с отношением масс 2:1 и две циркумбинарные луны \
                 на 3.2 и 7.9 ширины пары.",
                S::Stable,
                None,
            ),
            Scenario::JupiterTrojans => info(
                "Троянцы L4/L5",
                C::Lagrange,
                "Звезда, планета (μ = 0.002) и два роя троянцев в окрестности L4/L5. \
                 Точки Лагранжа устойчивы при μ < 0.0385; троянцы медленно либрируют.",
                S::Stable,
                None,
            ),
            Scenario::RandomCloud => info(
                "Протопланетное облако",
                C::Cloud,
                "200 частиц в толстом диске вокруг протозвезды. Диск не стабилен \
                 по построению — это демонстрация коллективной динамики и FMM.",
                S::Demo,
                None,
            ),
        }
    }

    /// Короткое имя (для меню и заголовков).
    pub fn name(self) -> &'static str {
        self.info().name
    }
}

// ──────────────────────────────────────────────────────────────────────────
// Генераторы
// ──────────────────────────────────────────────────────────────────────────

const STAR: [u8; 3] = [255, 205, 90];
const STAR2: [u8; 3] = [150, 200, 255];
const PLANET_COLORS: [[u8; 3]; 10] = [
    [110, 170, 255],
    [80, 220, 160],
    [240, 120, 110],
    [230, 180, 90],
    [200, 140, 255],
    [120, 230, 230],
    [255, 150, 190],
    [190, 210, 230],
    [170, 240, 120],
    [235, 160, 120],
];
const MOON_COLORS: [[u8; 3]; 6] = [
    [210, 220, 235],
    [190, 200, 220],
    [225, 210, 190],
    [180, 220, 230],
    [215, 195, 225],
    [200, 225, 200],
];

fn body(id: u64, mass: f64, pos: Vec3, vel: Vec3, radius: f64, color: [u8; 3]) -> Body {
    Body {
        id,
        mass,
        pos,
        vel,
        radius,
        color,
    }
}

fn push(
    bodies: &mut Vec<Body>,
    next_id: &mut u64,
    mass: f64,
    pos: Vec3,
    vel: Vec3,
    radius: f64,
    color: [u8; 3],
) {
    bodies.push(body(*next_id, mass, pos, vel, radius, color));
    *next_id += 1;
}

/// Радиус тела по массе — только для наглядности (физика точечная).
fn display_radius(mass: f64) -> f64 {
    (1.4 + mass.cbrt() * 2.0).clamp(1.4, 15.0)
}

/// Центр масс в начало координат, полный импульс — в ноль.
/// Относительные координаты и скорости при этом не меняются.
fn zero_momentum(bodies: &mut [Body]) {
    let mut m = 0.0;
    let mut com = Vec3::ZERO;
    let mut p = Vec3::ZERO;
    for b in bodies.iter() {
        if b.mass.is_finite() && b.mass > 0.0 {
            m += b.mass;
            com += b.pos * b.mass;
            p += b.vel * b.mass;
        }
    }
    if m <= 1e-12 {
        return;
    }
    com = com * (1.0 / m);
    let vcm = p * (1.0 / m);
    for b in bodies.iter_mut() {
        b.pos = b.pos - com;
        b.vel = b.vel - vcm;
    }
}

/// Наклонить всю систему как единое целое.
fn tilt(bodies: &mut [Body], rx: f64, ry: f64) {
    for b in bodies.iter_mut() {
        b.pos = b.pos.rotate_x(rx).rotate_y(ry);
        b.vel = b.vel.rotate_x(rx).rotate_y(ry);
    }
}

/// Ортонормированный базис плоскости с нормалью `normal` (u × v = n).
fn orbit_basis(normal: Vec3) -> (Vec3, Vec3) {
    let n = normal.normalized();
    let n = if n.len() < 1e-9 {
        Vec3::new(0.0, 0.0, 1.0)
    } else {
        n
    };
    let helper = if n.z.abs() < 0.9 {
        Vec3::new(0.0, 0.0, 1.0)
    } else {
        Vec3::new(1.0, 0.0, 0.0)
    };
    let u = helper.cross(n).normalized();
    let v = n.cross(u).normalized();
    (u, v)
}

/// Нормаль по наклонению и долготе узла.
fn normal_from(incl: f64, node: f64) -> Vec3 {
    Vec3::new(incl.sin() * node.cos(), incl.sin() * node.sin(), incl.cos())
}

/// Состояние на кеплеровской орбите из перицентра/апоцентра (vis-viva).
fn orbit_state(g: f64, m: f64, a: f64, e: f64, normal: Vec3, phase: f64) -> (Vec3, Vec3) {
    let (u, v) = orbit_basis(normal);
    let a = a.max(1e-6);
    let e = e.clamp(0.0, 0.9);
    let r_apo = a * (1.0 + e);
    let v_apo = (g * m * (1.0 - e) / (a * (1.0 + e))).sqrt();
    let (sp, cp) = phase.sin_cos();
    let pos = (u * cp + v * sp) * r_apo;
    let vel = (u * (-sp) + v * cp) * v_apo;
    (pos, vel)
}

/// Круговая орбита вокруг центра в начале координат.
fn circular(g: f64, m: f64, r: f64, normal: Vec3, phase: f64) -> (Vec3, Vec3) {
    orbit_state(g, m, r, 0.0, normal, phase)
}

/// Система «звезда + планеты»; орбиты кеплеровские, затем обнуляется импульс.
fn planetary_system(
    g: f64,
    star_mass: f64,
    star_radius: f64,
    base_normal: Vec3,
    specs: &[(f64, f64, f64, [u8; 3])],
    rng: &mut Rng,
) -> Vec<Body> {
    let mut bodies = Vec::new();
    let mut next_id = 1;
    push(
        &mut bodies,
        &mut next_id,
        star_mass,
        Vec3::ZERO,
        Vec3::ZERO,
        star_radius,
        STAR,
    );
    for (i, &(a, mass, e, color)) in specs.iter().enumerate() {
        // Небольшие взаимные наклонения: диск объёмный, но динамика почти плоская.
        let incl = (rng.f64() - 0.5) * 0.05;
        let node = rng.f64() * TAU;
        let normal = if base_normal.len() > 0.5 {
            // Наклоняем относительно базовой плоскости.
            let (u, v) = orbit_basis(base_normal);
            let n = (u * (incl.sin() * node.cos())
                + v * (incl.sin() * node.sin())
                + base_normal.normalized() * incl.cos())
            .normalized();
            n
        } else {
            normal_from(incl, node)
        };
        let phase = i as f64 * 2.399_963_229_728_653 + rng.f64() * 0.9;
        let (pos, vel) = orbit_state(g, star_mass, a, e, normal, phase);
        push(
            &mut bodies,
            &mut next_id,
            mass,
            pos,
            vel,
            display_radius(mass),
            color,
        );
    }
    zero_momentum(&mut bodies);
    bodies
}

/// Хилл-устойчивый набор планет: соседние полуоси отстоят на `k·R_H`.
fn hill_system(
    g: f64,
    star_mass: f64,
    star_radius: f64,
    start_a: f64,
    count: usize,
    k: f64,
    mass_lo: f64,
    mass_hi: f64,
    rng: &mut Rng,
) -> Vec<Body> {
    let mut bodies = Vec::new();
    let mut next_id = 1;
    push(
        &mut bodies,
        &mut next_id,
        star_mass,
        Vec3::ZERO,
        Vec3::ZERO,
        star_radius,
        STAR,
    );

    let mut a = start_a;
    let mut prev: Option<(f64, f64)> = None;
    for i in 0..count {
        let mass = mass_lo + (mass_hi - mass_lo) * rng.f64();
        if let Some((prev_a, prev_mass)) = prev {
            // Взаимный радиус Хилла соседней пары.
            let r_h = ((mass + prev_mass) / (3.0 * star_mass)).cbrt() * (a + prev_a) * 0.5;
            a = prev_a + k * r_h;
        }
        let e = rng.f64() * 0.015;
        let incl = (rng.f64() - 0.5) * 0.05;
        let node = rng.f64() * TAU;
        let phase = i as f64 * 2.399_963_229_728_653 + rng.f64();
        let normal = normal_from(incl, node);
        let (pos, vel) = orbit_state(g, star_mass, a, e, normal, phase);
        push(
            &mut bodies,
            &mut next_id,
            mass,
            pos,
            vel,
            display_radius(mass),
            PLANET_COLORS[i % PLANET_COLORS.len()],
        );
        prev = Some((a, mass));
    }
    zero_momentum(&mut bodies);
    bodies
}

/// Две гравитирующие массы на круговой орбите (барицентр в нуле, импульс 0).
#[allow(clippy::too_many_arguments)]
fn binary(
    g: f64,
    m1: f64,
    m2: f64,
    separation: f64,
    normal: Vec3,
    phase: f64,
    r1: f64,
    c1: [u8; 3],
    r2: f64,
    c2: [u8; 3],
) -> Vec<Body> {
    let (u, v) = orbit_basis(normal);
    let total = m1 + m2;
    let v_rel = (g * total / separation).sqrt();
    let (sp, cp) = phase.sin_cos();
    let dir = u * cp + v * sp;
    let tangent = u * (-sp) + v * cp;
    let p1 = dir * (-separation * m2 / total);
    let p2 = dir * (separation * m1 / total);
    let v1 = tangent * (-v_rel * m2 / total);
    let v2 = tangent * (v_rel * m1 / total);
    vec![body(1, m1, p1, v1, r1, c1), body(2, m2, p2, v2, r2, c2)]
}

/// Добавить луну к телу `host` в его орбитальной плоскости.
#[allow(clippy::too_many_arguments)]
fn add_moon(
    bodies: &mut Vec<Body>,
    next_id: &mut u64,
    g: f64,
    host: usize,
    r: f64,
    mass: f64,
    radius: f64,
    color: [u8; 3],
    normal: Vec3,
    phase: f64,
) {
    let (hp, hv, hm) = (bodies[host].pos, bodies[host].vel, bodies[host].mass);
    let (u, v) = orbit_basis(normal);
    let (sp, cp) = phase.sin_cos();
    let offset = (u * cp + v * sp) * r;
    let v_circ = (g * (hm + mass) / r).sqrt();
    let tangent = normal.normalized().cross(offset.normalized());
    push(
        bodies,
        next_id,
        mass,
        hp + offset,
        hv + tangent * v_circ,
        radius,
        color,
    );
}

/// Планета с лунами: луны задаются долей радиуса Хилла планеты.
fn planet_with_moons(
    g: f64,
    star_mass: f64,
    planet_mass: f64,
    planet_a: f64,
    normal: Vec3,
    moons: &[(f64, f64, f64, [u8; 3])],
    rng: &mut Rng,
) -> Vec<Body> {
    let mut bodies = planetary_system(
        g,
        star_mass,
        12.0,
        normal,
        &[(planet_a, planet_mass, 0.0, PLANET_COLORS[4])],
        rng,
    );
    let hill = planet_a * (planet_mass / (3.0 * star_mass)).cbrt();
    let mut next_id = bodies.iter().map(|b| b.id).max().unwrap_or(0) + 1;
    for (i, &(frac, mass, radius, color)) in moons.iter().enumerate() {
        let phase = i as f64 * 2.399_963_229_728_653 + 0.4;
        add_moon(
            &mut bodies,
            &mut next_id,
            g,
            1,
            hill * frac,
            mass,
            radius,
            color,
            normal,
            phase,
        );
    }
    zero_momentum(&mut bodies);
    bodies
}

/// Точная восьмёрка Ченчинера–Монтгомери в масштабе симуляции.
fn figure_eight(g: f64, with_planet: bool) -> Vec<Body> {
    // Канонические единицы: G = 1, массы 1, координаты порядка единицы.
    // Масштаб: mu — масса каждого тела, l — длина. Скорости умножаются на
    // sqrt(mu / l), период — на sqrt(l^3 / mu); динамика сохраняется.
    let mu = 3000.0_f64;
    let l = 80.0_f64;
    let vs = (mu / l).sqrt();
    let _ = g; // закон масштабирования уже учитывает G = 1; G в симуляции = 1
    let mut bodies = vec![
        body(
            1,
            mu,
            Vec3::new(0.970_004_36, -0.243_087_53, 0.0) * l,
            Vec3::new(0.466_203_685, 0.432_365_73, 0.0) * vs,
            5.2,
            [255, 130, 130],
        ),
        body(
            2,
            mu,
            Vec3::new(-0.970_004_36, 0.243_087_53, 0.0) * l,
            Vec3::new(0.466_203_685, 0.432_365_73, 0.0) * vs,
            5.2,
            [120, 185, 255],
        ),
        body(
            3,
            mu,
            Vec3::ZERO,
            Vec3::new(-0.932_407_37, -0.864_731_46, 0.0) * vs,
            5.2,
            [255, 220, 120],
        ),
    ];
    if with_planet {
        let r = 900.0;
        let v = (g * (3.0 * mu + 1.0) / r).sqrt();
        let mut next_id = 4;
        push(
            &mut bodies,
            &mut next_id,
            1.0,
            Vec3::new(r, 0.0, 0.0),
            Vec3::new(0.0, v, 0.0),
            2.0,
            [150, 255, 200],
        );
    }
    zero_momentum(&mut bodies);
    tilt(&mut bodies, 0.55, 0.18);
    bodies
}

fn solar_system(g: f64, rng: &mut Rng) -> Vec<Body> {
    let star_mass = 1000.0;
    let mut bodies = Vec::new();
    let mut next_id = 1;
    push(
        &mut bodies,
        &mut next_id,
        star_mass,
        Vec3::ZERO,
        Vec3::ZERO,
        14.0,
        STAR,
    );
    for i in 0..10 {
        let r = 70.0 + i as f64 * 42.0 + rng.f64() * 18.0;
        let mass = 0.5 + rng.f64() * 8.0;
        let incl = (rng.f64() - 0.5) * 0.14;
        let node = rng.f64() * TAU;
        let phase = rng.f64() * TAU;
        let e = rng.f64() * 0.02;
        let (pos, vel) = orbit_state(g, star_mass, r, e, normal_from(incl, node), phase);
        push(
            &mut bodies,
            &mut next_id,
            mass,
            pos,
            vel,
            1.6 + mass * 0.22,
            PLANET_COLORS[i % PLANET_COLORS.len()],
        );
    }
    zero_momentum(&mut bodies);
    tilt(&mut bodies, 0.30, 0.12);
    bodies
}

fn random_cloud(g: f64, rng: &mut Rng) -> Vec<Body> {
    let n = 200;
    let disk_r = 320.0;
    let mut bodies = Vec::new();
    let mut next_id = 1;
    for _ in 0..n {
        let r = (30.0f64 * 30.0 + (disk_r * disk_r - 30.0 * 30.0) * rng.f64()).sqrt();
        let a = rng.f64() * TAU;
        let z = (rng.f64() - 0.5) * r * 0.7;
        let pos = Vec3::new(r * a.cos(), r * a.sin(), z);
        let mass = 0.1 + rng.f64() * 2.0;
        push(
            &mut bodies,
            &mut next_id,
            mass,
            pos,
            Vec3::ZERO,
            0.9 + mass * 0.3,
            [
                120 + (rng.f64() * 100.0) as u8,
                160 + (rng.f64() * 80.0) as u8,
                255,
            ],
        );
    }
    push(
        &mut bodies,
        &mut next_id,
        900.0,
        Vec3::ZERO,
        Vec3::ZERO,
        12.0,
        [255, 210, 90],
    );
    let layout: Vec<(Vec3, f64)> = bodies.iter().map(|b| (b.pos, b.mass)).collect();
    for b in &mut bodies {
        if b.mass > 100.0 {
            continue;
        }
        let rho = b.pos.xy().len().max(1.0);
        let m_in: f64 = layout
            .iter()
            .filter(|(p, _)| p.xy().len() < rho)
            .map(|(_, m)| m)
            .sum();
        let v = (g * m_in / rho).sqrt();
        b.vel = b.pos.xy().perp().to3() * (v / rho);
        b.vel.z = (rng.f64() - 0.5) * v * 0.12;
    }
    zero_momentum(&mut bodies);
    bodies
}

fn binary_star(g: f64, rng: &mut Rng) -> Vec<Body> {
    let m = 800.0;
    let d = 140.0;
    let normal = normal_from(0.55, 0.9);
    let mut bodies = binary(g, m, m, d, normal, 0.0, 11.0, STAR, 10.0, STAR2);
    let mut next_id = 3;
    // P-тип: планеты заметно шире пары, иначе они были бы неустойчивы.
    let mut a = 430.0;
    let mut prev: Option<(f64, f64)> = None;
    for i in 0..6 {
        let mass = 0.5 + rng.f64() * 4.0;
        if let Some((pa, pm)) = prev {
            let r_h = ((mass + pm) / (3.0 * 2.0 * m)).cbrt() * (a + pa) * 0.5;
            a = pa + 4.5 * r_h;
        }
        let node = 0.9 + (rng.f64() - 0.5) * 0.12;
        let incl = 0.55 + (rng.f64() - 0.5) * 0.06;
        let phase = rng.f64() * TAU;
        let e = rng.f64() * 0.015;
        let (pos, vel) = orbit_state(g, 2.0 * m, a, e, normal_from(incl, node), phase);
        push(
            &mut bodies,
            &mut next_id,
            mass,
            pos,
            vel,
            display_radius(mass),
            PLANET_COLORS[i % PLANET_COLORS.len()],
        );
        prev = Some((a, mass));
    }
    zero_momentum(&mut bodies);
    bodies
}

fn circumbinary(g: f64, rng: &mut Rng) -> Vec<Body> {
    let m = 700.0;
    let d = 150.0;
    let normal = normal_from(0.42, 0.7);
    let mut bodies = binary(g, m, m, d, normal, 0.0, 11.0, STAR, 10.0, STAR2);
    let mut next_id = 3;
    let specs = [(550.0, 6.0), (1050.0, 4.0), (1750.0, 3.0)];
    for (i, (a, mass)) in specs.iter().copied().enumerate() {
        let node = 0.7 + (rng.f64() - 0.5) * 0.1;
        let incl = 0.42 + (rng.f64() - 0.5) * 0.05;
        let phase = i as f64 * 2.399_963 + rng.f64();
        let e = rng.f64() * 0.01;
        let (pos, vel) = orbit_state(g, 2.0 * m, a, e, normal_from(incl, node), phase);
        push(
            &mut bodies,
            &mut next_id,
            mass,
            pos,
            vel,
            display_radius(mass),
            PLANET_COLORS[(i + 1) % PLANET_COLORS.len()],
        );
    }
    zero_momentum(&mut bodies);
    bodies
}

fn stype(g: f64) -> Vec<Body> {
    let (m1, m2, d) = (1200.0, 500.0, 160.0);
    let normal = normal_from(0.5, 0.4);
    let mut bodies = binary(g, m1, m2, d, normal, 0.0, 11.0, STAR, 9.0, STAR2);
    let hill = d * (m1 / (3.0 * m2)).cbrt();
    let a = 0.18 * hill;
    let (p1, v1) = (bodies[0].pos, bodies[0].vel);
    let (dp, dv) = circular(g, m1, a, normal, 0.8);
    let mut next_id = 3;
    push(
        &mut bodies,
        &mut next_id,
        3.0,
        p1 + dp,
        v1 + dv,
        3.0,
        [120, 230, 180],
    );
    zero_momentum(&mut bodies);
    bodies
}

fn hierarchical_triple(g: f64) -> Vec<Body> {
    let (m1, m2, m3) = (800.0, 800.0, 1000.0);
    let (d, r_out) = (90.0, 1200.0);
    let total = m1 + m2 + m3;
    let inner = m1 + m2;
    let outer_n = normal_from(0.5, 0.4);
    let inner_n = normal_from(0.72, 0.9);
    let (u, v) = orbit_basis(outer_n);
    let (ui, vi) = orbit_basis(inner_n);
    let v_rel = (g * total / r_out).sqrt();
    let com_in = u * (-r_out * m3 / total);
    let com_out = u * (r_out * inner / total);
    let v3 = v * (v_rel * inner / total);
    let v_com_in = v * (-v_rel * m3 / total);
    let v_in = (g * inner / d).sqrt();
    vec![
        body(
            1,
            m1,
            com_in + ui * (-d * 0.5),
            v_com_in + vi * (-v_in * 0.5),
            11.0,
            STAR,
        ),
        body(
            2,
            m2,
            com_in + ui * (d * 0.5),
            v_com_in + vi * (v_in * 0.5),
            10.0,
            STAR2,
        ),
        body(3, m3, com_out, v3, 12.0, [255, 160, 100]),
    ]
}

fn hierarchical_quad(g: f64) -> Vec<Body> {
    let m = 600.0;
    let (d, r_out) = (75.0, 1400.0);
    let total = 4.0 * m;
    let v_rel = (g * total / r_out).sqrt();
    let (u, v) = orbit_basis(normal_from(0.45, 0.2));
    let com_a = u * (-r_out * 0.5);
    let com_b = u * (r_out * 0.5);
    let v_a = v * (-v_rel * 0.5);
    let v_b = v * (v_rel * 0.5);
    let v_in = (g * 2.0 * m / d).sqrt() * 0.5;
    let (ua, va) = orbit_basis(normal_from(0.7, 0.2));
    let (ub, vb) = orbit_basis(normal_from(0.35, 1.4));
    vec![
        body(
            1,
            m,
            com_a + ua * (-d * 0.5),
            v_a + va * (-v_in),
            10.0,
            STAR,
        ),
        body(2, m, com_a + ua * (d * 0.5), v_a + va * v_in, 9.5, STAR2),
        body(
            3,
            m,
            com_b + ub * (-d * 0.5),
            v_b + vb * (-v_in),
            10.0,
            [255, 150, 110],
        ),
        body(
            4,
            m,
            com_b + ub * (d * 0.5),
            v_b + vb * v_in,
            9.5,
            [180, 160, 255],
        ),
    ]
}

fn pluto_charon(g: f64, rng: &mut Rng) -> Vec<Body> {
    let normal = normal_from(0.6, 1.2);
    let mut bodies = binary(
        g,
        800.0,
        400.0,
        140.0,
        normal,
        0.0,
        9.0,
        [210, 200, 190],
        7.0,
        [170, 190, 220],
    );
    let mut next_id = 3;
    for (i, (r, mass)) in [(450.0, 4.0), (1100.0, 2.0)].into_iter().enumerate() {
        let phase = i as f64 * 2.399_963 + rng.f64();
        let (pos, vel) = circular(g, 1200.0, r, normal, phase);
        push(
            &mut bodies,
            &mut next_id,
            mass,
            pos,
            vel,
            2.0,
            MOON_COLORS[i],
        );
    }
    zero_momentum(&mut bodies);
    bodies
}

fn jupiter_trojans(g: f64) -> Vec<Body> {
    let (m_star, m_planet, d) = (2000.0, 4.0, 400.0);
    let normal = normal_from(0.35, 0.6);
    let mut bodies = binary(
        g,
        m_star,
        m_planet,
        d,
        normal,
        0.0,
        13.0,
        STAR,
        5.0,
        [255, 150, 90],
    );
    let (p1, v1) = (bodies[0].pos, bodies[0].vel);
    let (p2, v2) = (bodies[1].pos, bodies[1].vel);
    let rel = p2 - p1;
    let vrel = v2 - v1;
    let omega = rel.cross(vrel) * (1.0 / rel.len2());
    let n = omega.normalized();
    let bary = (p1 * m_star + p2 * m_planet) * (1.0 / (m_star + m_planet));
    let vcm = (v1 * m_star + v2 * m_planet) * (1.0 / (m_star + m_planet));
    let rotate = |v: Vec3, ang: f64| v * ang.cos() + n.cross(v) * ang.sin();
    let mut next_id = 3;
    for side in [1.0, -1.0] {
        let base = p1 + rotate(rel, side * FRAC_PI_3);
        let radial = (base - bary).normalized();
        let tangent = n.cross(base - bary).normalized();
        for k in -3i32..=3 {
            let pos = base + tangent * (k as f64 * 6.0) + radial * ((k.rem_euclid(2)) as f64 * 1.5);
            let vel = vcm + omega.cross(pos - bary);
            push(
                &mut bodies,
                &mut next_id,
                0.01,
                pos,
                vel,
                1.3,
                if side > 0.0 {
                    [130, 225, 180]
                } else {
                    [185, 235, 140]
                },
            );
        }
    }
    zero_momentum(&mut bodies);
    bodies
}

fn stable_ring(g: f64, rng: &mut Rng) -> Vec<Body> {
    let normal = normal_from(0.4, 0.7);
    let mut bodies = planetary_system(
        g,
        2000.0,
        12.0,
        normal,
        &[(600.0, 800.0, 0.0, [230, 190, 140])],
        rng,
    );
    let (hp, hv, hm) = (bodies[1].pos, bodies[1].vel, bodies[1].mass);
    let (u, v) = orbit_basis(normal);
    let mut next_id = 3;
    for k in 0..96 {
        let r = 45.0 + 75.0 * rng.f64();
        let theta = k as f64 * 2.399_963 + rng.f64() * 0.06;
        let z = (rng.f64() - 0.5) * 1.6;
        let offset = u * (r * theta.cos()) + v * (r * theta.sin()) + normal.normalized() * z;
        let v_circ = (g * hm / offset.len().max(1e-9)).sqrt();
        let tangent = normal.normalized().cross(offset.normalized());
        push(
            &mut bodies,
            &mut next_id,
            0.0015,
            hp + offset,
            hv + tangent * v_circ,
            0.8,
            [
                170 + (rng.f64() * 50.0) as u8,
                205 + (rng.f64() * 40.0) as u8,
                240,
            ],
        );
    }
    zero_momentum(&mut bodies);
    bodies
}

/// Сгенерировать тела пресета.
pub fn generate(scenario: Scenario, g: f64) -> Vec<Body> {
    let mut rng = Rng::new(0x9E37_79B9_7F4A_7C15);
    match scenario {
        Scenario::FigureEight => figure_eight(g, false),
        Scenario::FigureEightPlanet => figure_eight(g, true),
        Scenario::SolarSystem => solar_system(g, &mut rng),
        Scenario::StableSystem => {
            let mut bodies = hill_system(g, 2000.0, 14.0, 70.0, 8, 5.0, 0.05, 0.5, &mut rng);
            tilt(&mut bodies, 0.42, 0.22);
            bodies
        }
        Scenario::HillCompact => {
            let mut bodies = hill_system(g, 3000.0, 14.0, 70.0, 6, 4.8, 0.05, 0.35, &mut rng);
            tilt(&mut bodies, 0.26, 0.1);
            bodies
        }
        Scenario::HillGrand => {
            let mut bodies = hill_system(g, 3000.0, 14.0, 90.0, 9, 6.0, 0.05, 0.4, &mut rng);
            tilt(&mut bodies, 0.3, 0.12);
            bodies
        }
        Scenario::KeplerLike => {
            let specs = [
                (70.0, 0.15, 0.01, PLANET_COLORS[0]),
                (105.0, 0.10, 0.01, PLANET_COLORS[1]),
                (160.0, 0.25, 0.008, PLANET_COLORS[2]),
                (240.0, 0.30, 0.012, PLANET_COLORS[3]),
                (360.0, 0.20, 0.006, PLANET_COLORS[4]),
                (540.0, 0.15, 0.01, PLANET_COLORS[5]),
            ];
            let mut bodies = planetary_system(g, 2500.0, 13.0, Vec3::ZERO, &specs, &mut rng);
            tilt(&mut bodies, 0.28, 0.12);
            bodies
        }
        Scenario::TrappistLike => {
            let specs = [
                (45.0, 0.020, 0.004, PLANET_COLORS[1]),
                (58.0, 0.030, 0.003, PLANET_COLORS[2]),
                (75.0, 0.040, 0.005, PLANET_COLORS[3]),
                (96.0, 0.050, 0.003, PLANET_COLORS[4]),
                (123.0, 0.060, 0.004, PLANET_COLORS[5]),
                (157.0, 0.080, 0.003, PLANET_COLORS[6]),
                (200.0, 0.100, 0.004, PLANET_COLORS[7]),
            ];
            let mut bodies = planetary_system(g, 500.0, 9.0, Vec3::ZERO, &specs, &mut rng);
            tilt(&mut bodies, 0.25, 0.1);
            bodies
        }
        Scenario::PulsarPlanets => {
            let specs = [
                (80.0, 0.020, 0.02, PLANET_COLORS[1]),
                (140.0, 0.015, 0.02, PLANET_COLORS[3]),
                (230.0, 0.010, 0.02, PLANET_COLORS[5]),
            ];
            let mut bodies = planetary_system(g, 2500.0, 5.0, Vec3::ZERO, &specs, &mut rng);
            tilt(&mut bodies, 0.3, 0.1);
            bodies
        }
        Scenario::GasGiants => {
            // Газовые гиганты разнесены по взаимным радиусам Хилла, а не по
            // «красивым» круглым числам: иначе внешняя пара слишком тесная.
            let mut bodies = hill_system(g, 20_000.0, 15.0, 300.0, 4, 6.0, 10.0, 50.0, &mut rng);
            tilt(&mut bodies, 0.32, 0.14);
            bodies
        }
        Scenario::EarthMoon => planet_with_moons(
            g,
            3000.0,
            40.0,
            420.0,
            Vec3::ZERO,
            &[(0.42, 0.5, 2.6, MOON_COLORS[0])],
            &mut rng,
        ),
        Scenario::MoonSystem => planet_with_moons(
            g,
            2500.0,
            1200.0,
            650.0,
            Vec3::ZERO,
            &[
                (0.08, 3.0, 3.2, MOON_COLORS[0]),
                (0.14, 2.0, 2.6, MOON_COLORS[1]),
                (0.21, 1.5, 2.2, MOON_COLORS[2]),
                (0.30, 1.0, 1.9, MOON_COLORS[3]),
            ],
            &mut rng,
        ),
        Scenario::StableRing => stable_ring(g, &mut rng),
        Scenario::BinaryStar => binary_star(g, &mut rng),
        Scenario::BinaryUnequal => binary(
            g,
            1400.0,
            500.0,
            180.0,
            normal_from(0.6, 1.1),
            0.0,
            12.0,
            STAR,
            9.0,
            STAR2,
        ),
        Scenario::HierarchicalTriple => hierarchical_triple(g),
        Scenario::HierarchicalQuad => hierarchical_quad(g),
        Scenario::CircumbinaryPlanets => circumbinary(g, &mut rng),
        Scenario::StypePlanet => stype(g),
        Scenario::PlutoCharon => pluto_charon(g, &mut rng),
        Scenario::JupiterTrojans => jupiter_trojans(g),
        Scenario::RandomCloud => random_cloud(g, &mut rng),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::physics::sim::{Algorithm, SimParams, Simulation};

    #[test]
    fn all_presets_are_nonempty_and_centered() {
        for scenario in Scenario::ALL {
            let bodies = generate(scenario, 1.0);
            assert!(!bodies.is_empty(), "{}: пустой пресет", scenario.name());
            let mut total = 0.0;
            let mut com = Vec3::ZERO;
            let mut momentum = Vec3::ZERO;
            for b in &bodies {
                assert!(
                    b.mass.is_finite() && b.mass > 0.0,
                    "{}: масса",
                    scenario.name()
                );
                assert!(b.pos.is_finite(), "{}: позиция", scenario.name());
                assert!(b.vel.is_finite(), "{}: скорость", scenario.name());
                total += b.mass;
                com += b.pos * b.mass;
                momentum += b.vel * b.mass;
            }
            assert!(
                (com * (1.0 / total)).len() < 1e-6,
                "{}: центр масс не в нуле",
                scenario.name()
            );
            assert!(
                (momentum * (1.0 / total)).len() < 1e-6,
                "{}: ненулевой импульс",
                scenario.name()
            );
        }
    }

    #[test]
    fn figure_eight_survives_several_periods() {
        let params = SimParams {
            integrator: Integrator::PolynomialRK4,
            algorithm: Algorithm::BruteForce,
            dt: 0.016,
            softening: 1.0,
            speed: 3.0,
            ..Default::default()
        };
        let mut sim = Simulation::new(params, Scenario::FigureEight);
        let start = sim
            .bodies()
            .iter()
            .map(|b| b.pos.len())
            .fold(0.0_f64, f64::max);
        for _ in 0..4000 {
            sim.step();
        }
        let end = sim
            .bodies()
            .iter()
            .map(|b| b.pos.len())
            .fold(0.0_f64, f64::max);
        assert!(end < start * 2.0, "восьмёрка разлетелась: {start} -> {end}");
    }

    #[test]
    fn presets_do_not_explode_immediately() {
        for scenario in Scenario::ALL {
            let rec = scenario.info().recommended.unwrap_or(Recommended {
                integrator: Integrator::PolynomialRK4,
                dt: 0.008,
                softening: 4.0,
                speed: 1.0,
            });
            let params = SimParams {
                integrator: rec.integrator,
                dt: rec.dt.min(0.016),
                softening: rec.softening,
                algorithm: Algorithm::BruteForce,
                ..Default::default()
            };
            let mut sim = Simulation::new(params, scenario);
            let start = sim
                .bodies()
                .iter()
                .map(|b| b.pos.len())
                .fold(0.0_f64, f64::max)
                .max(1.0);
            for _ in 0..300 {
                sim.step();
            }
            for b in sim.bodies() {
                assert!(
                    b.pos.is_finite() && b.vel.is_finite(),
                    "{}: NaN за 300 шагов",
                    scenario.name()
                );
            }
            let end = sim
                .bodies()
                .iter()
                .map(|b| b.pos.len())
                .fold(0.0_f64, f64::max);
            assert!(
                end < start * 60.0,
                "{}: система разлетелась за 300 шагов ({start} -> {end})",
                scenario.name()
            );
        }
    }

    #[test]
    fn fmm_step_produces_regions_and_keeps_moon_orbit() {
        let params = SimParams {
            algorithm: Algorithm::FastMultipole,
            integrator: Integrator::SymplecticEuler,
            dt: 0.008,
            softening: 1.0,
            fmm_theta: 0.5,
            fmm_order: 2,
            ..Default::default()
        };
        let mut sim = Simulation::new(params, Scenario::EarthMoon);
        let planet = sim.bodies()[1].pos;
        let moon = sim.bodies()[2].pos;
        let r0 = (moon - planet).len();
        for _ in 0..600 {
            sim.step();
        }
        assert!(!sim.regions().is_empty(), "FMM не отдал области");
        assert!(sim.bodies().iter().all(|b| b.pos.is_finite()));
        let planet = sim.bodies()[1].pos;
        let moon = sim.bodies()[2].pos;
        let r1 = (moon - planet).len();
        assert!(
            (r1 / r0 - 1.0).abs() < 0.15,
            "луна ушла с орбиты: {r0} -> {r1}"
        );
    }

    #[test]
    fn quick_orbit_stays_inside_hill_sphere() {
        // Звезда и планета на круговой орбите; вокруг планеты строим орбиту.
        let bodies = vec![
            Body {
                id: 1,
                mass: 1000.0,
                pos: Vec3::ZERO,
                vel: Vec3::ZERO,
                radius: 10.0,
                color: [255, 255, 255],
            },
            Body {
                id: 2,
                mass: 1.0,
                pos: Vec3::new(100.0, 0.0, 0.0),
                vel: Vec3::new(0.0, (1001.0f64 / 100.0).sqrt(), 0.0),
                radius: 2.0,
                color: [200, 220, 255],
            },
        ];
        let plan =
            super::super::sim::orbit_plan(&bodies, 1, Vec3::new(160.0, 0.0, 0.0), 0.001, 1.0)
                .expect("орбита строится");
        let hill = 100.0 * (1.0f64 / 3000.0).cbrt();
        assert!(plan.radius <= hill * 0.45 + 1e-9, "радиус вне Хилла");
        let rel = plan.pos - bodies[1].pos;
        assert!((rel.len() - plan.radius).abs() < 1e-9);
        let vrel = plan.vel - bodies[1].vel;
        assert!(vrel.dot(rel).abs() / (vrel.len() * rel.len()) < 1e-9);
        let expected = (1.001f64 / plan.radius).sqrt();
        assert!((vrel.len() - expected).abs() < 1e-9);
    }
}
