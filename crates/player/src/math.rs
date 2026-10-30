//! The float functions `core` lacks, from libm, for builds without `std`. With `std` the inherent
//! methods of the same names are used.

pub trait Float: Sized {
    fn sin(self) -> Self;
    fn cos(self) -> Self;
    fn sinh(self) -> Self;
    fn exp(self) -> Self;
    fn ln(self) -> Self;
    fn log10(self) -> Self;
    fn powf(self, n: Self) -> Self;
    fn powi(self, n: i32) -> Self;
    fn sqrt(self) -> Self;
    fn floor(self) -> Self;
    fn ceil(self) -> Self;
    fn round(self) -> Self;
}

macro_rules! float {
    ($t:ty, $sin:ident, $cos:ident, $sinh:ident, $exp:ident, $ln:ident, $log10:ident, $pow:ident, $sqrt:ident, $floor:ident, $ceil:ident, $round:ident) => {
        impl Float for $t {
            fn sin(self) -> Self { libm::$sin(self) }
            fn cos(self) -> Self { libm::$cos(self) }
            fn sinh(self) -> Self { libm::$sinh(self) }
            fn exp(self) -> Self { libm::$exp(self) }
            fn ln(self) -> Self { libm::$ln(self) }
            fn log10(self) -> Self { libm::$log10(self) }
            fn powf(self, n: Self) -> Self { libm::$pow(self, n) }
            fn powi(self, n: i32) -> Self { libm::$pow(self, n as $t) }
            fn sqrt(self) -> Self { libm::$sqrt(self) }
            fn floor(self) -> Self { libm::$floor(self) }
            fn ceil(self) -> Self { libm::$ceil(self) }
            fn round(self) -> Self { libm::$round(self) }
        }
    };
}

float!(f32, sinf, cosf, sinhf, expf, logf, log10f, powf, sqrtf, floorf, ceilf, roundf);
float!(f64, sin, cos, sinh, exp, log, log10, pow, sqrt, floor, ceil, round);
