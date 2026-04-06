//! Serial protocol for ESP display communication
//!
//! The protocol sends newline-terminated key=value pairs over USB CDC serial.
//! Data is sent in rotating frames to keep each message size reasonable.

/// Snapshot of all host metrics at a point in time
pub struct MetricsSnapshot {
    pub hostname: String,
    
    // Core metrics (Frame 1)
    pub cpu_pct: f32,
    pub mem_pct: f32,
    pub uptime_secs: u64,
    pub cpu_temp: f32,
    pub cpu_temp_available: bool,
    pub rx_kbps: f32,
    pub tx_kbps: f32,
    pub iface: String,
    
    // Disk metrics (Frame 2)
    pub disk_pct: f32,
    pub disk_r_kbs: f32,
    pub disk_w_kbs: f32,
    pub disk_temp: f32,
    pub disk_temp_available: bool,
    pub fan_rpm: f32,
    pub fan_available: bool,
    
    // GPU metrics (Frame 3)
    pub gpu_temp: f32,
    pub gpu_util: f32,
    pub gpu_mem: f32,
    pub gpu_available: bool,
    pub gpu_enabled: bool,
    
    // Docker metrics (Frame 4)
    pub docker_enabled: bool,
    pub docker_running: u32,
    pub docker_stopped: u32,
    pub docker_unhealthy: u32,
    pub docker_compact: String,
    
    // VM metrics (Frame 5)
    pub vms_enabled: bool,
    pub vms_running: u32,
    pub vms_stopped: u32,
    pub vms_paused: u32,
    pub vms_other: u32,
    pub vms_compact: String,
}

impl MetricsSnapshot {
    /// Build the 5 rotating serial frames
    pub fn build_frames(&self) -> [String; 5] {
        [
            self.frame1_core(),
            self.frame2_disk(),
            self.frame3_gpu(),
            self.frame4_docker(),
            self.frame5_vms(),
        ]
    }

    /// Frame 1: Core system metrics
    fn frame1_core(&self) -> String {
        format!(
            "CPU={:.1},TEMP={},MEM={:.1},UP={},RX={:.1},TX={:.1},IFACE={},TEMPAV={},GPUEN={},DOCKEREN={},VMSEN={},POWER=RUNNING\n",
            self.cpu_pct,
            if self.cpu_temp_available { format!("{:.1}", self.cpu_temp) } else { String::new() },
            self.mem_pct,
            self.uptime_secs,
            self.rx_kbps,
            self.tx_kbps,
            self.iface,
            if self.cpu_temp_available { "1" } else { "0" },
            if self.gpu_enabled { "1" } else { "0" },
            if self.docker_enabled { "1" } else { "0" },
            if self.vms_enabled { "1" } else { "0" },
        )
    }

    /// Frame 2: Disk and fan metrics
    fn frame2_disk(&self) -> String {
        format!(
            "NVMET={},DISKPCT={:.1},DISKR={:.1},DISKW={:.1},FAN={},NVMTAV={},FANAV={},POWER=RUNNING\n",
            if self.disk_temp_available { format!("{:.1}", self.disk_temp) } else { String::new() },
            self.disk_pct,
            self.disk_r_kbs,
            self.disk_w_kbs,
            if self.fan_available { format!("{:.0}", self.fan_rpm) } else { String::new() },
            if self.disk_temp_available { "1" } else { "0" },
            if self.fan_available { "1" } else { "0" },
        )
    }

    /// Frame 3: GPU metrics
    fn frame3_gpu(&self) -> String {
        format!(
            "GPUT={},GPUU={},GPUVM={},GPUAV={},POWER=RUNNING\n",
            if self.gpu_available { format!("{:.1}", self.gpu_temp) } else { String::new() },
            if self.gpu_available { format!("{:.1}", self.gpu_util) } else { String::new() },
            if self.gpu_available { format!("{:.1}", self.gpu_mem) } else { String::new() },
            if self.gpu_available { "1" } else { "0" },
        )
    }

    /// Frame 4: Docker container metrics
    fn frame4_docker(&self) -> String {
        format!(
            "DOCKRUN={},DOCKSTOP={},DOCKUNH={},DOCKER={},POWER=RUNNING\n",
            if self.docker_enabled { self.docker_running.to_string() } else { String::new() },
            if self.docker_enabled { self.docker_stopped.to_string() } else { String::new() },
            if self.docker_enabled { self.docker_unhealthy.to_string() } else { String::new() },
            if self.docker_enabled { &self.docker_compact } else { "" },
        )
    }

    /// Frame 5: Virtual machine metrics
    fn frame5_vms(&self) -> String {
        format!(
            "VMSRUN={},VMSSTOP={},VMSPAUSE={},VMSOTHER={},VMS={},POWER=RUNNING\n",
            if self.vms_enabled { self.vms_running.to_string() } else { String::new() },
            if self.vms_enabled { self.vms_stopped.to_string() } else { String::new() },
            if self.vms_enabled { self.vms_paused.to_string() } else { String::new() },
            if self.vms_enabled { self.vms_other.to_string() } else { String::new() },
            if self.vms_enabled { &self.vms_compact } else { "" },
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_frame_format() {
        let snapshot = MetricsSnapshot {
            hostname: "testhost".to_string(),
            cpu_pct: 45.5,
            mem_pct: 62.3,
            uptime_secs: 12345,
            cpu_temp: 55.0,
            cpu_temp_available: true,
            rx_kbps: 100.5,
            tx_kbps: 50.2,
            iface: "eth0".to_string(),
            disk_pct: 75.0,
            disk_r_kbs: 10.0,
            disk_w_kbs: 5.0,
            disk_temp: 40.0,
            disk_temp_available: true,
            fan_rpm: 0.0,
            fan_available: false,
            gpu_temp: 0.0,
            gpu_util: 0.0,
            gpu_mem: 0.0,
            gpu_available: false,
            gpu_enabled: false,
            docker_enabled: false,
            docker_running: 0,
            docker_stopped: 0,
            docker_unhealthy: 0,
            docker_compact: String::new(),
            vms_enabled: false,
            vms_running: 0,
            vms_stopped: 0,
            vms_paused: 0,
            vms_other: 0,
            vms_compact: String::new(),
        };

        let frames = snapshot.build_frames();
        assert!(frames[0].starts_with("CPU=45.5,"));
        assert!(frames[0].ends_with("\n"));
        assert!(frames[0].contains("POWER=RUNNING"));
    }
}
