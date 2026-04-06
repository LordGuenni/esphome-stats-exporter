mod metrics;
mod serial_proto;

use anyhow::{Context, Result};
use clap::Parser;
use log::{error, info, warn};
use std::time::{Duration, Instant};
use std::thread;

use metrics::HostMetrics;

#[derive(Parser, Debug)]
#[command(name = "esp-host-bridge")]
#[command(about = "Lightweight host metrics agent for ESP displays")]
struct Args {
    /// Serial port device (e.g., /dev/ttyACM0). Use "debug" to print without sending.
    #[arg(short, long)]
    port: Option<String>,

    /// Baud rate
    #[arg(short, long, default_value = "115200")]
    baud: u32,

    /// Poll interval in seconds
    #[arg(short, long, default_value = "1.0")]
    interval: f64,

    /// Enable VM polling via virsh
    #[arg(long)]
    enable_vms: bool,

    /// Virsh URI (e.g., qemu:///system)
    #[arg(long)]
    virsh_uri: Option<String>,

    /// Enable Docker polling
    #[arg(long)]
    enable_docker: bool,

    /// Docker socket path
    #[arg(long, default_value = "/var/run/docker.sock")]
    docker_socket: String,

    /// Disable GPU polling
    #[arg(long)]
    disable_gpu: bool,

    /// Debug mode: print frames without opening serial port
    #[arg(long)]
    debug: bool,
}

// ESPHome USB device identifiers
const ESPHOME_VID: u16 = 0x303a;
const ESPHOME_PID: u16 = 0x4001;

fn find_serial_port() -> Option<String> {
    use serialport::SerialPortType;
    
    let ports = serialport::available_ports().ok()?;
    
    // First priority: ESPHome device by VID:PID (303a:4001)
    for port in &ports {
        if let SerialPortType::UsbPort(usb_info) = &port.port_type {
            if usb_info.vid == ESPHOME_VID && usb_info.pid == ESPHOME_PID {
                info!(
                    "Auto-detected ESPHome device: {} (VID:{:04x} PID:{:04x})",
                    port.port_name, usb_info.vid, usb_info.pid
                );
                return Some(port.port_name.clone());
            }
        }
    }
    
    // Second: any USB device with "ESPHome" in product name
    for port in &ports {
        if let SerialPortType::UsbPort(usb_info) = &port.port_type {
            if let Some(ref product) = usb_info.product {
                if product.to_lowercase().contains("esphome") {
                    info!(
                        "Auto-detected ESPHome device by name: {} ({})",
                        port.port_name, product
                    );
                    return Some(port.port_name.clone());
                }
            }
        }
    }
    
    // Fallback: /dev/ttyACM* devices (USB CDC)
    for port in &ports {
        if port.port_name.contains("ttyACM") {
            info!("Fallback to ttyACM device: {}", port.port_name);
            return Some(port.port_name.clone());
        }
    }
    
    // Then /dev/ttyUSB*
    for port in &ports {
        if port.port_name.contains("ttyUSB") {
            info!("Fallback to ttyUSB device: {}", port.port_name);
            return Some(port.port_name.clone());
        }
    }
    
    // Any available port
    ports.first().map(|p| p.port_name.clone())
}

fn open_serial(port: &str, baud: u32) -> Result<Box<dyn serialport::SerialPort>> {
    serialport::new(port, baud)
        .timeout(Duration::from_millis(100))
        .open()
        .with_context(|| format!("Failed to open serial port {}", port))
}

fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    
    let args = Args::parse();
    
    let debug_mode = args.debug || args.port.as_deref() == Some("debug") || args.port.as_deref() == Some("none");
    
    let port_name = if debug_mode {
        info!("Debug mode: serial output disabled");
        None
    } else {
        Some(args.port.clone().or_else(find_serial_port)
            .context("No serial port specified and none auto-detected. Use --debug for testing.")?)
    };
    
    info!("ESP Host Bridge (Rust) starting");
    if let Some(ref port) = port_name {
        info!("Serial port: {} @ {} baud", port, args.baud);
    }
    
    let interval = Duration::from_secs_f64(args.interval);
    let mut metrics = HostMetrics::new();
    let mut frame_index: usize = 0;
    let mut serial: Option<Box<dyn serialport::SerialPort>> = None;
    let mut hostname_sent = false;
    
    loop {
        let loop_start = Instant::now();
        
        // Try to open serial if not connected (skip in debug mode)
        if !debug_mode && serial.is_none() {
            if let Some(ref port) = port_name {
                match open_serial(port, args.baud) {
                    Ok(s) => {
                        info!("Serial connected: {}", port);
                        serial = Some(s);
                        hostname_sent = false;
                    }
                    Err(e) => {
                        warn!("Serial open failed: {}, retrying...", e);
                        thread::sleep(Duration::from_secs(2));
                        continue;
                    }
                }
            }
        }
        
        // Collect metrics
        let snapshot = metrics.collect(&args);
        
        // Build and send frame
        let frames = snapshot.build_frames();
        let frame = &frames[frame_index % frames.len()];
        frame_index = (frame_index + 1) % frames.len();
        
        info!("{}", frame.trim());
        
        if let Some(ref mut ser) = serial {
            // Send hostname once after connect
            if !hostname_sent {
                let hostname_line = format!("HOSTNAME={}\n", snapshot.hostname);
                if let Err(e) = ser.write_all(hostname_line.as_bytes()) {
                    error!("Serial write failed: {}", e);
                    serial = None;
                    continue;
                }
                hostname_sent = true;
            }
            
            // Send frame
            if let Err(e) = ser.write_all(frame.as_bytes()) {
                error!("Serial write failed: {}", e);
                serial = None;
                continue;
            }
            
            if let Err(e) = ser.flush() {
                warn!("Serial flush failed: {}", e);
            }
        }
        
        // Sleep for remaining interval
        let elapsed = loop_start.elapsed();
        if elapsed < interval {
            thread::sleep(interval - elapsed);
        }
    }
}
