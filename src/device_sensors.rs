//! The device's motion sensors behind UserInputService (AccelerometerEnabled,
//! GyroscopeEnabled, GetDeviceAcceleration / GetDeviceGravity /
//! GetDeviceRotation, DeviceAccelerationChanged / DeviceGravityChanged /
//! DeviceRotationChanged): Android's through src/android/sensors.rs;
//! nothing on a desktop, where the service reports no sensors. The sensors
//! run from the first time a script asks about them (`latest`) and the
//! samples reach the client each frame (`__rs_device_sensors`,
//! src/library.luau), in Roblox's units: the acceleration and gravity in
//! g (device axes: X right, Y up the screen, Z out of it), the rotation
//! as the device's orientation (a CFrame) with the gyroscope's rates
//! (rad/s) as the delta.

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
static WANTED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// (accelerometer, gyroscope) present. Reading it is the sign a script
/// is interested: the sensors start (a game that never asks costs no
/// power).
pub fn available() -> (bool, bool) {
    #[cfg(target_os = "android")]
    {
        let a = crate::android::sensors::available();
        if a.accelerometer || a.gyroscope {
            WANTED.store(true, std::sync::atomic::Ordering::Relaxed);
        }
        return (a.accelerometer, a.gyroscope);
    }
    #[allow(unreachable_code)]
    (false, false)
}

/// The latest reading (the sensors on from the first call); None where
/// there are none, or while no script has asked about them.
pub fn latest() -> Option<Sample> {
    #[cfg(target_os = "android")]
    {
        if !WANTED.load(std::sync::atomic::Ordering::Relaxed) {
            return None;
        }
        let (acc, gyro) = available();
        if !acc && !gyro {
            return None;
        }
        crate::android::sensors::set_enabled(acc, gyro);
        let r = crate::android::sensors::latest();
        const G: f32 = 9.80665;
        let g = |v: [f32; 3]| [v[0] / G, v[1] / G, v[2] / G];
        return Some(Sample {
            acceleration: g(r.acceleration),
            gravity: g(r.gravity),
            gyro: r.gyro,
            rotation: r.rotation,
            acceleration_seq: r.acceleration_seq,
            gravity_seq: r.gravity_seq,
            rotation_seq: r.rotation_seq.max(r.gyro_seq),
        });
    }
    #[allow(unreachable_code)]
    None
}
