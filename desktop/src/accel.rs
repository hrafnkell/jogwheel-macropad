//! Acceleration model - mirrors accelFactor() in src/main.cpp
use crate::model::{Settings, Wheel};

#[derive(Clone, Copy, Debug)]
pub struct Curve {
    pub start: f64,
    pub full: f64,
    pub curve: f64,
    pub fast: f64,
}
pub fn curve_params(w: Option<&Wheel>, s: &Settings) -> Curve {
    let pick = |v: Option<f64>, d: f64| v.filter(|x| *x >= 0.0).unwrap_or(d);
    Curve {
        start: pick(w.and_then(|w| w.accel_start), s.accel_start),
        full: pick(w.and_then(|w| w.accel_full), s.accel_full),
        curve: w.and_then(|w| w.accel_curve).filter(|c| *c > 0.0).unwrap_or(s.accel_curve),
        fast: pick(w.and_then(|w| w.fast_speed), s.fast_speed),
    }
}
pub fn accel_at(max: f64, p: &Curve, speed: f64) -> f64 {
    let full = if p.full <= p.start { p.start + 0.1 } else { p.full };
    let mut f = ((speed - p.start) / (full - p.start)).clamp(0.0, 1.0);
    if p.curve != 1.0 {
        f = f.powf(p.curve);
    }
    1.0 + f * (max - 1.0)
}
