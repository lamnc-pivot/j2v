use cpal::traits::{DeviceTrait, HostTrait};
use serde::{Deserialize, Serialize};

/// Audio device information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudioDevice {
    /// Device identifier (usually same as name on cpal)
    pub id: String,
    /// Human-readable device name
    pub name: String,
    /// Whether this device can be used as input
    pub is_input: bool,
    /// Whether this device can be used as output
    pub is_output: bool,
    /// Sample rate in Hz (if available)
    pub sample_rate: Option<u32>,
    /// Number of channels (if available)
    pub channels: Option<u32>,
}

/// Device manager for listing available audio devices
pub struct DeviceManager;

impl DeviceManager {
    /// Lists all available audio input devices on the system.
    pub fn list_input_devices() -> Result<Vec<AudioDevice>, String> {
        let host = cpal::default_host();
        let mut devices = Vec::new();

        // Get input devices
        for device in host.input_devices().map_err(|e| e.to_string())? {
            if let Ok(name) = device.name() {
                let mut sample_rate = None;
                let mut channels = None;

                if let Ok(config) = device.default_input_config() {
                    sample_rate = Some(config.sample_rate().0);
                    channels = Some(config.channels() as u32);
                }

                let device_info = AudioDevice {
                    id: name.clone(),
                    name,
                    is_input: true,
                    is_output: false,
                    sample_rate,
                    channels,
                };
                devices.push(device_info);
            }
        }

        if devices.is_empty() {
            return Err("No input devices found".to_string());
        }

        log::info!("Found {} input device(s)", devices.len());
        for device in &devices {
            log::debug!(
                "  Device: {} ({}Hz, {}ch)",
                device.name,
                device.sample_rate.unwrap_or(0),
                device.channels.unwrap_or(0)
            );
        }

        Ok(devices)
    }

}

