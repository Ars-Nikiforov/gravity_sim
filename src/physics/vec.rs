//! Двойная точность для физики: egui-типы (f32) сюда не входят —
//! движок считается в f64 и ничего не знает об интерфейсе.

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Vec2 {
    pub x: f64,
    pub y: f64,
}

impl Vec2 {
    pub const ZERO: Self = Self { x: 0.0, y: 0.0 };

    pub const fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }

    pub fn len(self) -> f64 {
        self.len2().sqrt()
    }

    pub fn len2(self) -> f64 {
        self.x * self.x + self.y * self.y
    }

    /// Перпендикуляр против часовой стрелки.
    pub fn perp(self) -> Self {
        Self::new(-self.y, self.x)
    }

    /// Дополнить нулевой z-компонентой (переход 2D → 3D).
    pub fn to3(self) -> Vec3 {
        Vec3::new(self.x, self.y, 0.0)
    }

    pub fn lerp(self, other: Self, t: f64) -> Self {
        self + (other - self) * t
    }

    pub fn distance(self, other: Self) -> f64 {
        (self - other).len()
    }
}

/// Трёхмерный вектор. Орбитальная плоскость сценариев — xy (z = 0),
/// поэтому 2D-симуляция — частный случай 3D.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Vec3 {
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

impl Vec3 {
    pub const ZERO: Self = Self { x: 0.0, y: 0.0, z: 0.0 };

    pub const fn new(x: f64, y: f64, z: f64) -> Self {
        Self { x, y, z }
    }

    pub fn xy(self) -> Vec2 {
        Vec2::new(self.x, self.y)
    }

    pub fn len(self) -> f64 {
        self.len2().sqrt()
    }

    pub fn len2(self) -> f64 {
        self.x * self.x + self.y * self.y + self.z * self.z
    }

    pub fn dot(self, other: Self) -> f64 {
        self.x * other.x + self.y * other.y + self.z * other.z
    }

    pub fn cross(self, other: Self) -> Self {
        Self::new(
            self.y * other.z - self.z * other.y,
            self.z * other.x - self.x * other.z,
            self.x * other.y - self.y * other.x,
        )
    }

    /// Единичный вектор; для нулевого/вырожденного возвращает ZERO.
    pub fn normalized(self) -> Self {
        let len = self.len();
        if len > 1e-12 {
            self * (1.0 / len)
        } else {
            Self::ZERO
        }
    }

    pub fn lerp(self, other: Self, t: f64) -> Self {
        self + (other - self) * t
    }

    pub fn is_finite(self) -> bool {
        self.x.is_finite() && self.y.is_finite() && self.z.is_finite()
    }

    /// Поворот вокруг оси X на угол `a` (радианы).
    pub fn rotate_x(self, a: f64) -> Self {
        let (s, c) = a.sin_cos();
        Self::new(self.x, self.y * c - self.z * s, self.y * s + self.z * c)
    }

    /// Поворот вокруг оси Y на угол `a` (радианы).
    pub fn rotate_y(self, a: f64) -> Self {
        let (s, c) = a.sin_cos();
        Self::new(self.x * c + self.z * s, self.y, -self.x * s + self.z * c)
    }
}

impl std::ops::Add for Vec3 {
    type Output = Self;
    fn add(self, rhs: Self) -> Self {
        Self::new(self.x + rhs.x, self.y + rhs.y, self.z + rhs.z)
    }
}

impl std::ops::Sub for Vec3 {
    type Output = Self;
    fn sub(self, rhs: Self) -> Self {
        Self::new(self.x - rhs.x, self.y - rhs.y, self.z - rhs.z)
    }
}

impl std::ops::Mul<f64> for Vec3 {
    type Output = Self;
    fn mul(self, k: f64) -> Self {
        Self::new(self.x * k, self.y * k, self.z * k)
    }
}

impl std::ops::AddAssign for Vec3 {
    fn add_assign(&mut self, rhs: Self) {
        *self = *self + rhs;
    }
}

impl std::ops::Neg for Vec3 {
    type Output = Self;
    fn neg(self) -> Self {
        Self::new(-self.x, -self.y, -self.z)
    }
}

impl std::ops::Add for Vec2 {
    type Output = Self;
    fn add(self, rhs: Self) -> Self {
        Self::new(self.x + rhs.x, self.y + rhs.y)
    }
}

impl std::ops::Sub for Vec2 {
    type Output = Self;
    fn sub(self, rhs: Self) -> Self {
        Self::new(self.x - rhs.x, self.y - rhs.y)
    }
}

impl std::ops::Mul<f64> for Vec2 {
    type Output = Self;
    fn mul(self, k: f64) -> Self {
        Self::new(self.x * k, self.y * k)
    }
}

impl std::ops::Neg for Vec2 {
    type Output = Self;
    fn neg(self) -> Self {
        Self::new(-self.x, -self.y)
    }
}

impl std::ops::AddAssign for Vec2 {
    fn add_assign(&mut self, rhs: Self) {
        *self = *self + rhs;
    }
}

/// Детерминированный xorshift64* — сценарии генерируются без внешнего crate rand.
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Self(seed | 1)
    }

    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    pub fn f64(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }

    /// Случайная точка в кольце [r_min, r_max] вокруг нуля.
    #[allow(dead_code)]
    pub fn ring(&mut self, r_min: f64, r_max: f64) -> Vec2 {
        let r = (r_min * r_min + (r_max * r_max - r_min * r_min) * self.f64()).sqrt();
        let a = self.f64() * std::f64::consts::TAU;
        Vec2::new(r * a.cos(), r * a.sin())
    }
}
