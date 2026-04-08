# ESP Host Bridge (Rust)

A lightweight Rust implementation of the ESP Host Bridge agent. This version focuses on the core functionality: collecting host metrics and sending them to an ESP display over USB serial.

## Features

- **CPU**: Usage percentage, temperature
- **Memory**: Usage percentage  
- **Network**: RX/TX rates (KB/s)
- **Disk**: Usage, I/O rates, temperature (via nvme/smartctl)
- **GPU**: NVIDIA GPU metrics (via nvidia-smi)
- **VMs**: libvirt/virsh VM status
- **Docker**: Container status (via Docker socket)

## Building

```bash
cd rust
cargo build --release
```

The binary will be at `target/release/esp-host-bridge`.

## Usage

```bash
# Auto-detect serial port
./esp-host-bridge

# Specify port explicitly
./esp-host-bridge --port /dev/ttyACM0

# Enable VM monitoring (Proxmox/libvirt)
./esp-host-bridge --enable-vms --virsh-uri qemu:///system

# Enable Docker monitoring
./esp-host-bridge --enable-docker

# All options
./esp-host-bridge --help
```

## Command Line Options

| Option | Default | Description |
|--------|---------|-------------|
| `--port`, `-p` | auto-detect | Serial port device |
| `--baud`, `-b` | 115200 | Baud rate |
| `--interval`, `-i` | 1.0 | Poll interval (seconds) |
| `--enable-vms` | false | Enable VM polling via virsh |
| `--virsh-uri` | auto | Virsh connection URI |
| `--enable-docker` | false | Enable Docker polling |
| `--docker-socket` | /var/run/docker.sock | Docker socket path |
| `--disable-gpu` | false | Disable GPU polling |

## Installing as systemd Service

```bash
# Build release binary
cargo build --release

# Copy binary
sudo cp target/release/esp-host-bridge /usr/local/bin/

# Edit service file to match your setup
# (adjust --port, --virsh-uri, etc.)
sudo cp esp-host-bridge.service /etc/systemd/system/

# Enable and start
sudo systemctl daemon-reload
sudo systemctl enable esp-host-bridge
sudo systemctl start esp-host-bridge

# Check status
sudo systemctl status esp-host-bridge
journalctl -u esp-host-bridge -f
```

## Serial Protocol

The agent sends newline-terminated key=value pairs in 5 rotating frames:

**Frame 1 - Core:**
```
CPU=45.5,TEMP=55.0,MEM=62.3,UP=12345,RX=100.5,TX=50.2,IFACE=eth0,TEMPAV=1,GPUEN=0,DOCKEREN=0,VMSEN=1,POWER=RUNNING
```

**Frame 2 - Disk:**
```
DISKTEMP=40.0,DISKPCT=75.0,DISKR=10.0,DISKW=5.0,FAN=,DISKTAV=1,FANAV=0,POWER=RUNNING
```

**Frame 3 - GPU:**
```
GPUT=65.0,GPUU=80.0,GPUVM=50.0,GPUAV=1,POWER=RUNNING
```

**Frame 4 - Docker:**
```
DOCKRUN=5,DOCKSTOP=2,DOCKUNH=0,DOCKER=nginx|running;redis|running,POWER=RUNNING
```

**Frame 5 - VMs:**
```
VMSRUN=2,VMSSTOP=1,VMSPAUSE=0,VMSOTHER=0,VMS=ubuntu|running|4|8192|Running;win10|stopped|2|4096|Stopped,POWER=RUNNING
```
