//! The device's motion sensors behind UserInputService (AccelerometerEnabled,
//! GyroscopeEnabled, GetDeviceAcceleration / GetDeviceGravity /
//! GetDeviceRotation, DeviceAccelerationChanged / DeviceGravityChanged /
//! DeviceRotationChanged): the device's through a [`Source`] the app
//! installs at start (Android's is src/android/sensors.rs, through
//! `install`); nothing on a desktop, where the service reports no sensors.
//! The sensors run from the first time a script asks about them (`latest`)
//! and the samples reach the client each frame (`__rs_device_sensors`,
//! library.luau), in Roblox's units: the acceleration and gravity in g
//! (device axes: X right, Y up the screen, Z out of it), the rotation as
//! the device's orientation (a CFrame) with the gyroscope's rates (rad/s)
//! as the delta.

use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};

/// The device's sensors in their own units: m/s² and rad/s.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Reading {
    /// The accelerometer without gravity (TYPE_LINEAR_ACCELERATION), m/s².
    pub acceleration: [f32; 3],
    /// Gravity, m/s² (about 9.81 long, pointing down in device axes).
    pub gravity: [f32; 3],
    /// The gyroscope, rad/s about the device axes.
    pub gyro: [f32; 3],
    /// The device's orientation as a unit quaternion (x, y, z, w).
    pub rotation: [f32; 4],
    /// Sample counters (a changed count is a new sample).
    pub acceleration_seq: u32,
    pub gravity_seq: u32,
    pub gyro_seq: u32,
    pub rotation_seq: u32,
}

/// What a system with sensors provides.
pub struct Source {
    /// (accelerometer, gyroscope) present.
    pub available: fn() -> (bool, bool),
    /// Turns the sensors on or off (accelerometer, gyroscope); called before
    /// every `latest`, so it must be cheap when nothing changes.
    pub set_enabled: fn(bool, bool),
    /// The latest reading.
    pub latest: fn() -> Reading,
}

static SOURCE: OnceLock<Source> = OnceLock::new();

/// Installs the device's sensors (once, at start; the Android entry point).
pub fn install(source: Source) {
    let _ = SOURCE.set(source);
}

/// One reading, in Roblox's units (the module doc).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Sample {
    pub acceleration: [f32; 3],
    pub gravity: [f32; 3],
    /// Gyroscope rates, rad/s.
    pub gyro: [f32; 3],
    /// Orientation quaternion (x, y, z, w).
    pub rotation: [f32; 4],
    /// Sample counters: a changed count is a new sample (the *Changed events).
    pub acceleration_seq: u32,
    pub gravity_seq: u32,
    pub rotation_seq: u32,
}

/// A script asked about the sensors (UserInputService.AccelerometerEnabled
/// / GyroscopeEnabled read, a GetDevice* call): they run from then on.
static WANTED: AtomicBool = AtomicBool::new(false);

/// (accelerometer, gyroscope) present. Reading it is the sign a script
/// is interested: the sensors start (a game that never asks costs no
/// power).
pub fn available() -> (bool, bool) {
    let Some(src) = SOURCE.get() else {
        return (false, false);
    };
    let (acc, gyro) = (src.available)();
    if acc || gyro {
        WANTED.store(true, Ordering::Relaxed);
    }
    (acc, gyro)
}

/// The latest reading (the sensors on from the first call); None where
/// there are none, or while no script has asked about them.
pub fn latest() -> Option<Sample> {
    let src = SOURCE.get()?;
    if !WANTED.load(Ordering::Relaxed) {
        return None;
    }
    let (acc, gyro) = available();
    if !acc && !gyro {
        return None;
    }
    (src.set_enabled)(acc, gyro);
    let r = (src.latest)();
    const G: f32 = 9.80665;
    let g = |v: [f32; 3]| [v[0] / G, v[1] / G, v[2] / G];
    Some(Sample {
        acceleration: g(r.acceleration),
        gravity: g(r.gravity),
        gyro: r.gyro,
        rotation: r.rotation,
        acceleration_seq: r.acceleration_seq,
        gravity_seq: r.gravity_seq,
        rotation_seq: r.rotation_seq.max(r.gyro_seq),
    })
}
