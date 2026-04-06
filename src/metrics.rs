//! Host metrics collection for Linux systems

use crate::serial_proto::MetricsSnapshot;
use crate::Args;
use gethostname::gethostname;
use std::collections::HashMap;
use std::fs;
use std::io::{BufRead, BufReader};
use std::process::Command;
use std::time::Instant;
use sysinfo::{CpuRefreshKind, Disks, MemoryRefreshKind, Networks, RefreshKind, System};

pub struct HostMetrics {
    sys: System,
    networks: Networks,
    disks: Disks,
    prev_net_rx: u64,
    prev_net_tx: u64,
    prev_net_time: Option<Instant>,
    prev_disk_read: u64,
    prev_disk_write: u64,
    prev_disk_time: Option<Instant>,
    active_iface: Option<String>,
    active_disk: Option<String>,
    hostname: String,
}

impl HostMetrics {
    pub fn new() -> Self {
        let sys = System::new_with_specifics(
            RefreshKind::nothing()
                .with_cpu(CpuRefreshKind::everything())
                .with_memory(MemoryRefreshKind::everything()),
        );
        let networks = Networks::new_with_refreshed_list();
        let disks = Disks::new_with_refreshed_list();
        let hostname = gethostname().to_string_lossy().to_string();

        Self {
            sys,
            networks,
            disks,
            prev_net_rx: 0,
            prev_net_tx: 0,
            prev_net_time: None,
            prev_disk_read: 0,
            prev_disk_write: 0,
            prev_disk_time: None,
            active_iface: None,
            active_disk: None,
            hostname,
        }
    }

    pub fn collect(&mut self, args: &Args) -> MetricsSnapshot {
        // Refresh system info
        self.sys.refresh_cpu_all();
        self.sys.refresh_memory();
        self.networks.refresh(true);
        self.disks.refresh(true);

        let cpu_pct = self.sys.global_cpu_usage();
        let mem_pct = if self.sys.total_memory() > 0 {
            (self.sys.used_memory() as f64 / self.sys.total_memory() as f64) * 100.0
        } else {
            0.0
        };
        let uptime_secs = System::uptime();

        // CPU temperature
        let (cpu_temp, cpu_temp_available) = self.get_cpu_temp();

        // Network rates
        let (rx_kbps, tx_kbps, iface) = self.get_network_rates();
        if iface.is_some() {
            self.active_iface = iface;
        }

        // Disk usage
        let disk_pct = self.get_disk_usage_pct();

        // Disk I/O rates
        let (disk_r_kbs, disk_w_kbs, disk_name) = self.get_disk_io_rates();
        if disk_name.is_some() {
            self.active_disk = disk_name;
        }

        // Disk temperature (expensive, could cache)
        let (disk_temp, disk_temp_available) = self.get_disk_temp();

        // GPU metrics
        let (gpu_temp, gpu_util, gpu_mem, gpu_available) = if args.disable_gpu {
            (0.0, 0.0, 0.0, false)
        } else {
            self.get_gpu_metrics()
        };

        // VM metrics
        let (vms_running, vms_stopped, vms_paused, vms_other, vms_compact) = if args.enable_vms {
            self.get_vm_metrics(args.virsh_uri.as_deref())
        } else {
            (0, 0, 0, 0, String::new())
        };

        // Docker metrics
        let (docker_running, docker_stopped, docker_unhealthy, docker_compact) = if args.enable_docker {
            self.get_docker_metrics(&args.docker_socket)
        } else {
            (0, 0, 0, String::new())
        };

        MetricsSnapshot {
            hostname: self.hostname.clone(),
            cpu_pct,
            mem_pct: mem_pct as f32,
            uptime_secs,
            cpu_temp,
            cpu_temp_available,
            rx_kbps,
            tx_kbps,
            iface: self.active_iface.clone().unwrap_or_default(),
            disk_pct,
            disk_r_kbs,
            disk_w_kbs,
            disk_temp,
            disk_temp_available,
            fan_rpm: 0.0, // TODO: implement fan detection
            fan_available: false,
            gpu_temp,
            gpu_util,
            gpu_mem,
            gpu_available,
            gpu_enabled: !args.disable_gpu,
            docker_enabled: args.enable_docker,
            docker_running,
            docker_stopped,
            docker_unhealthy,
            docker_compact,
            vms_enabled: args.enable_vms,
            vms_running,
            vms_stopped,
            vms_paused,
            vms_other,
            vms_compact,
        }
    }

    fn get_cpu_temp(&self) -> (f32, bool) {
        // Try reading from /sys/class/thermal/
        if let Ok(entries) = fs::read_dir("/sys/class/thermal") {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                if !name.starts_with("thermal_zone") {
                    continue;
                }
                let type_path = entry.path().join("type");
                let temp_path = entry.path().join("temp");

                let type_str = fs::read_to_string(&type_path).unwrap_or_default().to_lowercase();
                
                // Prefer CPU-related thermal zones
                if type_str.contains("cpu") || type_str.contains("pkg") || type_str.contains("x86") || type_str.contains("soc") {
                    if let Ok(temp_str) = fs::read_to_string(&temp_path) {
                        if let Ok(temp_millic) = temp_str.trim().parse::<f64>() {
                            let temp_c = if temp_millic > 1000.0 {
                                temp_millic / 1000.0
                            } else {
                                temp_millic
                            };
                            if (-20.0..=150.0).contains(&temp_c) {
                                return (temp_c as f32, true);
                            }
                        }
                    }
                }
            }

            // Fallback: any thermal zone
            for entry in fs::read_dir("/sys/class/thermal").into_iter().flatten().flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                if !name.starts_with("thermal_zone") {
                    continue;
                }
                let temp_path = entry.path().join("temp");
                if let Ok(temp_str) = fs::read_to_string(&temp_path) {
                    if let Ok(temp_millic) = temp_str.trim().parse::<f64>() {
                        let temp_c = if temp_millic > 1000.0 {
                            temp_millic / 1000.0
                        } else {
                            temp_millic
                        };
                        if (-20.0..=150.0).contains(&temp_c) {
                            return (temp_c as f32, true);
                        }
                    }
                }
            }
        }

        (0.0, false)
    }

    fn get_network_rates(&mut self) -> (f32, f32, Option<String>) {
        let now = Instant::now();
        
        // Find the best interface and sum traffic
        let mut best_iface: Option<String> = None;
        let mut total_rx: u64 = 0;
        let mut total_tx: u64 = 0;
        let mut best_traffic: u64 = 0;

        for (name, data) in self.networks.iter() {
            let name_lower = name.to_lowercase();
            
            // Skip loopback and virtual interfaces
            if name_lower == "lo" || name_lower.starts_with("veth") || 
               name_lower.starts_with("docker") || name_lower.starts_with("br-") ||
               name_lower.starts_with("virbr") {
                continue;
            }

            let rx = data.total_received();
            let tx = data.total_transmitted();
            let traffic = rx + tx;

            // Prefer eth/en/wl interfaces
            let is_preferred = name_lower.starts_with("eth") || 
                              name_lower.starts_with("en") || 
                              name_lower.starts_with("wl");

            if best_iface.is_none() || (is_preferred && traffic > 0) || traffic > best_traffic {
                if is_preferred || traffic > best_traffic {
                    best_iface = Some(name.clone());
                    best_traffic = traffic;
                }
            }

            total_rx += rx;
            total_tx += tx;
        }

        let (rx_kbps, tx_kbps) = if let Some(prev_time) = self.prev_net_time {
            let dt = now.duration_since(prev_time).as_secs_f64();
            if dt > 0.0 {
                let rx_diff = total_rx.saturating_sub(self.prev_net_rx) as f64;
                let tx_diff = total_tx.saturating_sub(self.prev_net_tx) as f64;
                ((rx_diff / dt / 1024.0) as f32, (tx_diff / dt / 1024.0) as f32)
            } else {
                (0.0, 0.0)
            }
        } else {
            (0.0, 0.0)
        };

        self.prev_net_rx = total_rx;
        self.prev_net_tx = total_tx;
        self.prev_net_time = Some(now);

        (rx_kbps.max(0.0), tx_kbps.max(0.0), best_iface)
    }

    fn get_disk_usage_pct(&self) -> f32 {
        // Find root or largest disk
        let mut root_pct: Option<f32> = None;
        let mut largest_pct: Option<f32> = None;
        let mut largest_size: u64 = 0;

        for disk in self.disks.iter() {
            let mount = disk.mount_point().to_string_lossy();
            let total = disk.total_space();
            let available = disk.available_space();
            
            if total == 0 {
                continue;
            }

            let used = total.saturating_sub(available);
            let pct = (used as f64 / total as f64 * 100.0) as f32;

            if mount == "/" {
                root_pct = Some(pct);
            }

            if total > largest_size {
                largest_size = total;
                largest_pct = Some(pct);
            }
        }

        root_pct.or(largest_pct).unwrap_or(0.0)
    }

    fn get_disk_io_rates(&mut self) -> (f32, f32, Option<String>) {
        let now = Instant::now();
        
        // Read /proc/diskstats
        let mut stats: HashMap<String, (u64, u64)> = HashMap::new();
        
        if let Ok(file) = fs::File::open("/proc/diskstats") {
            let reader = BufReader::new(file);
            for line in reader.lines().flatten() {
                let cols: Vec<&str> = line.split_whitespace().collect();
                if cols.len() < 14 {
                    continue;
                }
                
                let name = cols[2];
                
                // Skip partitions and virtual devices
                if name.starts_with("loop") || name.starts_with("ram") || 
                   name.starts_with("dm-") || name.starts_with("sr") ||
                   name.starts_with("zram") {
                    continue;
                }
                
                // Skip partition numbers (e.g., sda1, but keep nvme0n1)
                if !name.starts_with("nvme") && name.chars().last().map(|c| c.is_ascii_digit()).unwrap_or(false) {
                    continue;
                }
                
                // Skip nvme partitions (e.g., nvme0n1p1)
                if name.starts_with("nvme") && name.contains("p") && name.ends_with(char::is_numeric) {
                    continue;
                }

                if let (Ok(sectors_read), Ok(sectors_written)) = (
                    cols[5].parse::<u64>(),
                    cols[9].parse::<u64>()
                ) {
                    stats.insert(name.to_string(), (sectors_read * 512, sectors_written * 512));
                }
            }
        }

        // Find best disk
        let disk_name = self.active_disk.clone().or_else(|| {
            for prefix in ["nvme", "sd", "vd", "xvd", "mmcblk"] {
                for name in stats.keys() {
                    if name.starts_with(prefix) {
                        return Some(name.clone());
                    }
                }
            }
            stats.keys().next().cloned()
        });

        let (total_read, total_write) = disk_name.as_ref()
            .and_then(|n| stats.get(n))
            .copied()
            .unwrap_or((0, 0));

        let (r_kbs, w_kbs) = if let Some(prev_time) = self.prev_disk_time {
            let dt = now.duration_since(prev_time).as_secs_f64();
            if dt > 0.0 {
                let r_diff = total_read.saturating_sub(self.prev_disk_read) as f64;
                let w_diff = total_write.saturating_sub(self.prev_disk_write) as f64;
                ((r_diff / dt / 1024.0) as f32, (w_diff / dt / 1024.0) as f32)
            } else {
                (0.0, 0.0)
            }
        } else {
            (0.0, 0.0)
        };

        self.prev_disk_read = total_read;
        self.prev_disk_write = total_write;
        self.prev_disk_time = Some(now);

        (r_kbs.max(0.0), w_kbs.max(0.0), disk_name)
    }

    fn get_disk_temp(&self) -> (f32, bool) {
        // Try lm-sensors first (most reliable on modern Linux)
        if let Ok(output) = Command::new("sensors").output() {
            if output.status.success() {
                let text = String::from_utf8_lossy(&output.stdout);
                if let Some(temp) = extract_nvme_temp_from_sensors(&text) {
                    return (temp, true);
                }
            }
        }

        // Fallback: Try nvme smart-log
        if let Ok(output) = Command::new("nvme")
            .args(["smart-log", "/dev/nvme0"])
            .output()
        {
            let text = String::from_utf8_lossy(&output.stdout);
            if let Some(temp) = extract_temp_from_text(&text) {
                return (temp, true);
            }
        }

        // Fallback: Try smartctl
        for dev in ["/dev/nvme0", "/dev/sda"] {
            if let Ok(output) = Command::new("smartctl")
                .args(["-A", dev])
                .output()
            {
                let text = String::from_utf8_lossy(&output.stdout);
                if let Some(temp) = extract_temp_from_text(&text) {
                    return (temp, true);
                }
            }
        }

        (0.0, false)
    }

    fn get_gpu_metrics(&self) -> (f32, f32, f32, bool) {
        // Try NVIDIA GPU via nvidia-smi
        if let Some(metrics) = self.get_nvidia_gpu() {
            return metrics;
        }

        // Try AMD GPU via sensors + sysfs
        if let Some(metrics) = self.get_amd_gpu() {
            return metrics;
        }

        // Try Intel Arc GPU via sysfs
        if let Some(metrics) = self.get_intel_gpu() {
            return metrics;
        }

        (0.0, 0.0, 0.0, false)
    }

    fn get_nvidia_gpu(&self) -> Option<(f32, f32, f32, bool)> {
        let output = Command::new("nvidia-smi")
            .args([
                "--query-gpu=temperature.gpu,utilization.gpu,memory.used,memory.total",
                "--format=csv,noheader,nounits",
            ])
            .output()
            .ok()?;

        if !output.status.success() {
            return None;
        }

        let text = String::from_utf8_lossy(&output.stdout);
        for line in text.lines() {
            let parts: Vec<&str> = line.split(',').map(|s| s.trim()).collect();
            if parts.len() >= 4 {
                let temp = parts[0].parse::<f32>().unwrap_or(0.0);
                let util = parts[1].parse::<f32>().unwrap_or(0.0);
                let mem_used = parts[2].parse::<f64>().unwrap_or(0.0);
                let mem_total = parts[3].parse::<f64>().unwrap_or(1.0);
                let mem_pct = if mem_total > 0.0 {
                    (mem_used / mem_total * 100.0) as f32
                } else {
                    0.0
                };
                return Some((temp, util, mem_pct, true));
            }
        }
        None
    }

    fn get_amd_gpu(&self) -> Option<(f32, f32, f32, bool)> {
        // Get temperature from sensors (amdgpu)
        let temp = self.get_amd_gpu_temp()?;

        // Get utilization from sysfs
        let util = self.get_amd_gpu_util().unwrap_or(0.0);

        // Get VRAM usage from sysfs
        let mem_pct = self.get_amd_gpu_vram_pct().unwrap_or(0.0);

        Some((temp, util, mem_pct, true))
    }

    fn get_amd_gpu_temp(&self) -> Option<f32> {
        // Try sensors command first
        if let Ok(output) = Command::new("sensors").output() {
            if output.status.success() {
                let text = String::from_utf8_lossy(&output.stdout);
                let mut in_amd_section = false;

                for line in text.lines() {
                    if line.starts_with("amdgpu-") {
                        in_amd_section = true;
                        continue;
                    }

                    if in_amd_section && !line.is_empty() && !line.starts_with(' ') && !line.starts_with('\t') 
                        && !line.starts_with("Adapter:") 
                    {
                        in_amd_section = false;
                    }

                    // Look for edge temperature (main GPU temp)
                    if in_amd_section && line.trim_start().starts_with("edge:") {
                        if let Some(temp) = extract_celsius_value(line) {
                            return Some(temp);
                        }
                    }
                }
            }
        }

        // Fallback: try hwmon sysfs
        if let Ok(entries) = fs::read_dir("/sys/class/hwmon") {
            for entry in entries.flatten() {
                let path = entry.path();
                let name_path = path.join("name");
                if let Ok(name) = fs::read_to_string(&name_path) {
                    if name.trim() == "amdgpu" {
                        // Read temp1_input (millidegrees)
                        let temp_path = path.join("temp1_input");
                        if let Ok(temp_str) = fs::read_to_string(&temp_path) {
                            if let Ok(temp_mdeg) = temp_str.trim().parse::<f32>() {
                                return Some(temp_mdeg / 1000.0);
                            }
                        }
                    }
                }
            }
        }
        None
    }

    fn get_amd_gpu_util(&self) -> Option<f32> {
        // Try reading GPU busy percentage from sysfs
        // Path: /sys/class/drm/card*/device/gpu_busy_percent
        if let Ok(entries) = fs::read_dir("/sys/class/drm") {
            for entry in entries.flatten() {
                let name = entry.file_name();
                let name_str = name.to_string_lossy();
                if name_str.starts_with("card") && !name_str.contains('-') {
                    let gpu_busy_path = entry.path().join("device/gpu_busy_percent");
                    if let Ok(val) = fs::read_to_string(&gpu_busy_path) {
                        if let Ok(util) = val.trim().parse::<f32>() {
                            return Some(util);
                        }
                    }
                }
            }
        }
        None
    }

    fn get_amd_gpu_vram_pct(&self) -> Option<f32> {
        // Read VRAM usage from sysfs
        // /sys/class/drm/card*/device/mem_info_vram_used
        // /sys/class/drm/card*/device/mem_info_vram_total
        if let Ok(entries) = fs::read_dir("/sys/class/drm") {
            for entry in entries.flatten() {
                let name = entry.file_name();
                let name_str = name.to_string_lossy();
                if name_str.starts_with("card") && !name_str.contains('-') {
                    let device_path = entry.path().join("device");
                    let used_path = device_path.join("mem_info_vram_used");
                    let total_path = device_path.join("mem_info_vram_total");

                    if let (Ok(used_str), Ok(total_str)) = (
                        fs::read_to_string(&used_path),
                        fs::read_to_string(&total_path),
                    ) {
                        if let (Ok(used), Ok(total)) = (
                            used_str.trim().parse::<f64>(),
                            total_str.trim().parse::<f64>(),
                        ) {
                            if total > 0.0 {
                                return Some((used / total * 100.0) as f32);
                            }
                        }
                    }
                }
            }
        }
        None
    }

    fn get_intel_gpu(&self) -> Option<(f32, f32, f32, bool)> {
        // Intel Arc GPU support via sysfs and intel_gpu_top
        
        // Get temperature from hwmon
        let temp = self.get_intel_gpu_temp().unwrap_or(0.0);
        
        // Get utilization - try intel_gpu_top or sysfs
        let util = self.get_intel_gpu_util().unwrap_or(0.0);

        // If we found at least temperature, report as available
        if temp > 0.0 || util > 0.0 {
            return Some((temp, util, 0.0, true)); // VRAM% not easily available for Intel
        }
        
        None
    }

    fn get_intel_gpu_temp(&self) -> Option<f32> {
        // Check hwmon for i915 or xe driver
        if let Ok(entries) = fs::read_dir("/sys/class/hwmon") {
            for entry in entries.flatten() {
                let path = entry.path();
                let name_path = path.join("name");
                if let Ok(name) = fs::read_to_string(&name_path) {
                    let name = name.trim();
                    if name == "i915" || name == "xe" {
                        let temp_path = path.join("temp1_input");
                        if let Ok(temp_str) = fs::read_to_string(&temp_path) {
                            if let Ok(temp_mdeg) = temp_str.trim().parse::<f32>() {
                                return Some(temp_mdeg / 1000.0);
                            }
                        }
                    }
                }
            }
        }
        None
    }

    fn get_intel_gpu_util(&self) -> Option<f32> {
        // Try reading from sysfs for Intel discrete GPUs
        // /sys/class/drm/card*/gt/gt0/rps_cur_freq_mhz vs rps_max_freq_mhz as approximation
        // Or use intel_gpu_top if available (requires root typically)
        
        if let Ok(entries) = fs::read_dir("/sys/class/drm") {
            for entry in entries.flatten() {
                let name = entry.file_name();
                let name_str = name.to_string_lossy();
                if name_str.starts_with("card") && !name_str.contains('-') {
                    // Check if this is an Intel GPU
                    let device_path = entry.path().join("device");
                    let vendor_path = device_path.join("vendor");
                    if let Ok(vendor) = fs::read_to_string(&vendor_path) {
                        if vendor.trim() == "0x8086" {
                            // Intel device - try to get frequency-based utilization
                            let gt_path = entry.path().join("gt/gt0");
                            let cur_freq = gt_path.join("rps_cur_freq_mhz");
                            let max_freq = gt_path.join("rps_max_freq_mhz");
                            
                            if let (Ok(cur_str), Ok(max_str)) = (
                                fs::read_to_string(&cur_freq),
                                fs::read_to_string(&max_freq),
                            ) {
                                if let (Ok(cur), Ok(max)) = (
                                    cur_str.trim().parse::<f32>(),
                                    max_str.trim().parse::<f32>(),
                                ) {
                                    if max > 0.0 {
                                        return Some((cur / max * 100.0).min(100.0));
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        None
    }

    fn get_vm_metrics(&self, virsh_uri: Option<&str>) -> (u32, u32, u32, u32, String) {
        let mut args = vec!["list", "--all", "--name"];
        let uri_arg;
        if let Some(uri) = virsh_uri {
            uri_arg = format!("-c{}", uri);
            args.insert(0, &uri_arg);
        }

        let output = match Command::new("virsh").args(&args).output() {
            Ok(o) if o.status.success() => o,
            _ => return (0, 0, 0, 0, String::new()),
        };

        let names: Vec<String> = String::from_utf8_lossy(&output.stdout)
            .lines()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();

        if names.is_empty() {
            return (0, 0, 0, 0, String::new());
        }

        let mut running = 0u32;
        let mut stopped = 0u32;
        let mut paused = 0u32;
        let mut other = 0u32;
        let mut compact_parts: Vec<String> = Vec::new();

        for name in names.iter().take(10) {
            let mut dominfo_args = vec!["dominfo", name.as_str()];
            if let Some(uri) = virsh_uri {
                dominfo_args.insert(0, "-c");
                dominfo_args.insert(1, uri);
            }

            if let Ok(info_out) = Command::new("virsh").args(&dominfo_args).output() {
                let info_text = String::from_utf8_lossy(&info_out.stdout);
                let (state_key, state_label, vcpus, mem_mib) = parse_dominfo(&info_text);
                
                match state_key.as_str() {
                    "running" => running += 1,
                    "stopped" => stopped += 1,
                    "paused" => paused += 1,
                    _ => other += 1,
                }

                let clean_name = name.replace(['|', ';', ','], "_");
                compact_parts.push(format!(
                    "{}|{}|{}|{}|{}",
                    &clean_name[..clean_name.len().min(24)],
                    state_key,
                    vcpus,
                    mem_mib,
                    state_label
                ));
            }
        }

        let compact = if compact_parts.is_empty() {
            "-".to_string()
        } else {
            compact_parts.join(";")
        };

        (running, stopped, paused, other, compact)
    }

    fn get_docker_metrics(&self, socket_path: &str) -> (u32, u32, u32, String) {
        // Use curl to query Docker socket (simpler than implementing Unix socket HTTP)
        let output = match Command::new("curl")
            .args([
                "--unix-socket", socket_path,
                "-s",
                "http://localhost/containers/json?all=1"
            ])
            .output()
        {
            Ok(o) if o.status.success() => o,
            _ => return (0, 0, 0, String::new()),
        };

        let text = String::from_utf8_lossy(&output.stdout);
        
        // Simple JSON parsing without serde
        let mut running = 0u32;
        let mut stopped = 0u32;
        let mut unhealthy = 0u32;
        let mut compact_parts: Vec<String> = Vec::new();

        // Very basic parsing - look for container patterns
        // This is simplified; for production, use serde_json
        for container in text.split(r#"{"Id":"#).skip(1).take(10) {
            let state = if container.to_lowercase().contains("\"running\"") {
                running += 1;
                "running"
            } else {
                stopped += 1;
                "stopped"
            };

            if container.to_lowercase().contains("unhealthy") {
                unhealthy += 1;
            }

            // Extract name (simplified)
            if let Some(name_start) = container.find("\"Names\":[\"") {
                let rest = &container[name_start + 10..];
                if let Some(name_end) = rest.find('"') {
                    let name = rest[..name_end].trim_start_matches('/');
                    let clean_name = name.replace(['|', ';', ','], "_");
                    compact_parts.push(format!(
                        "{}|{}",
                        &clean_name[..clean_name.len().min(24)],
                        state
                    ));
                }
            }
        }

        let compact = if compact_parts.is_empty() {
            "-".to_string()
        } else {
            compact_parts.join(";")
        };

        (running, stopped, unhealthy, compact)
    }
}

fn extract_temp_from_text(text: &str) -> Option<f32> {
    for line in text.lines() {
        let lower = line.to_lowercase();
        if !lower.contains("temperature") && !lower.contains("composite") {
            continue;
        }
        
        // Extract numbers from the line
        for word in line.split_whitespace() {
            if let Ok(temp) = word.trim_end_matches(['°', 'C', 'c']).parse::<f32>() {
                if (-20.0..=150.0).contains(&temp) {
                    return Some(temp);
                }
            }
        }
    }
    None
}

/// Extract NVMe temperature from `sensors` command output.
/// Looks for nvme-pci-* sections and extracts the Composite temperature.
fn extract_nvme_temp_from_sensors(text: &str) -> Option<f32> {
    let mut in_nvme_section = false;
    
    for line in text.lines() {
        // Check if we're entering an nvme section
        if line.starts_with("nvme-pci-") || line.starts_with("nvme-") {
            in_nvme_section = true;
            continue;
        }
        
        // Check if we're leaving the section (new sensor block starting)
        if in_nvme_section && !line.is_empty() && !line.starts_with(' ') && !line.starts_with('\t') 
            && !line.starts_with("Adapter:") && !line.contains("Composite")
        {
            in_nvme_section = false;
        }
        
        // Look for Composite temperature in nvme section
        if in_nvme_section {
            let lower = line.to_lowercase();
            if lower.contains("composite") {
                // Parse line like "Composite:    +43.9°C  (low  = ..."
                // Extract temperature using regex-like pattern: +XX.X or XX.X followed by °C
                if let Some(temp) = extract_celsius_value(line) {
                    return Some(temp);
                }
            }
        }
    }
    None
}

/// Extract a Celsius temperature value from a string like "+43.9°C" or "43.9°C"
fn extract_celsius_value(text: &str) -> Option<f32> {
    // Look for pattern like +43.9°C or 43.9°C
    for part in text.split_whitespace() {
        // Check if this part contains °C or just C after a number
        if part.contains('°') || part.ends_with('C') || part.ends_with('c') {
            // Remove the +, °, C suffix to get the number
            let num_str: String = part.chars()
                .filter(|c| c.is_ascii_digit() || *c == '.' || *c == '-')
                .collect();
            if let Ok(temp) = num_str.parse::<f32>() {
                if (-20.0..=150.0).contains(&temp) {
                    return Some(temp);
                }
            }
        }
    }
    None
}

fn parse_dominfo(text: &str) -> (String, String, u32, u32) {
    let mut state = String::new();
    let mut vcpus = 0u32;
    let mut max_mem_kib = 0u64;

    for line in text.lines() {
        let line = line.trim();
        if let Some((key, value)) = line.split_once(':') {
            let key = key.trim().to_lowercase();
            let value = value.trim();
            
            match key.as_str() {
                "state" => state = value.to_lowercase(),
                "cpu(s)" => vcpus = value.parse().unwrap_or(0),
                "max memory" => {
                    // Parse "1048576 KiB" or similar
                    if let Some(num_str) = value.split_whitespace().next() {
                        max_mem_kib = num_str.parse().unwrap_or(0);
                    }
                }
                _ => {}
            }
        }
    }

    let (state_key, state_label) = classify_vm_state(&state);
    let mem_mib = (max_mem_kib / 1024) as u32;

    (state_key, state_label, vcpus, mem_mib)
}

fn classify_vm_state(state: &str) -> (String, String) {
    let s = state.to_lowercase();
    
    if s.contains("running") || s.contains("idle") {
        ("running".to_string(), "Running".to_string())
    } else if s.contains("paused") || s.contains("suspended") {
        ("paused".to_string(), "Paused".to_string())
    } else if s.contains("shut off") || s.contains("shutoff") || s.contains("crashed") || s.is_empty() {
        ("stopped".to_string(), "Stopped".to_string())
    } else {
        ("other".to_string(), s.to_string())
    }
}
