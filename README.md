English | [中文](README.zh-CN.md)

# UPS Monitor

`ups-monitor` is a small Rust daemon that polls a UPS Network Management Card (NMC) via SNMP and shuts down the host when the UPS has been running on battery for too long or when battery capacity falls too low.

SNMP access is implemented directly using the pure Rust `async-snmp` crate, with no dependency on net-snmp command-line tools. You can verify network connectivity and credentials with your NMC using the following command:

```sh
snmpwalk -v3 -l noAuthNoPriv -u local3 192.168.0.255 1.3.6.1.2.1.1.1.0
```

## Features

- Periodic SNMP polling.
- Pure Rust SNMP client supporting SNMPv3 `noAuthNoPriv`, `authNoPriv`, and `authPriv`.
- Annotated YAML configuration file; see `examples/ups-monitor.yaml` for an example.
- Shutdown trigger modes:
  - `battery_capacity`: shut down only when capacity falls below the configured percentage.
  - `on_battery_duration`: shut down only when battery runtime exceeds the configured duration.
  - `either`: monitor both conditions; shut down when either is met.
- Cancel on-battery-duration countdown when AC power returns.
- Log UPS state changes.
- Extensible UPS adapter boundary for future non-standard NMC cards.
- `init` command to install systemd service and sample configuration on Linux.

## Build & Test

```sh
cargo test
cargo build --release
```

No additional SNMP command-line dependencies are required at runtime; only network access to the UPS NMC is needed.

## Versioning & Releases

This project follows [Semantic Versioning](https://semver.org/). The version is defined in `Cargo.toml`.

### Download Prebuilt Binaries

GitHub Releases provides prebuilt binaries for Linux x86_64, macOS Intel, and macOS Apple Silicon:

<https://github.com/woestler/ups-monitor/releases>

### Releasing a New Version

Maintainer release process:

1. Update the `version` field in `Cargo.toml`:
   ```toml
   [package]
   version = "0.2.0"
   ```
2. Commit the version bump:
   ```sh
   git add Cargo.toml && git commit -m "Bump version to 0.2.0"
   ```
3. Create and push a git tag (CI will verify the tag matches `Cargo.toml`):
   ```sh
   git tag v0.2.0
   git push origin main && git push origin v0.2.0
   ```
4. GitHub Actions automatically triggers the build, validates the version, and publishes the release for that tag.

## Configuration

Create the configuration file:

```sh
sudo cp examples/ups-monitor.yaml /etc/ups-monitor.yaml
sudo editor /etc/ups-monitor.yaml
```

The most important setting to check on first run:

```yaml
shutdown:
  dry_run: true
```

Keep `dry_run: true` while you verify SNMP values and logs. Only change it to `false` when you are ready to enable actual automatic shutdown.

The default shutdown command uses a graceful shutdown sequence similar to NUT:

```yaml
shutdown:
  command: ["/sbin/shutdown", "-h", "+0"]
```

`systemctl poweroff` can also be used on systemd Linux, but `systemctl poweroff --force --force` is not recommended as the default. Double `--force` skips many normal shutdown steps and carries higher risk; it is better suited as a manual last resort in extreme situations rather than the default automatic action of UPS monitoring software.

Validate the configuration file:

```sh
ups-monitor --config /etc/ups-monitor.yaml check-config
```

Run a single poll:

```sh
ups-monitor --config /etc/ups-monitor.yaml run --once
```

Run in the foreground:

```sh
ups-monitor --config /etc/ups-monitor.yaml run
```

Print the annotated default configuration:

```sh
ups-monitor print-default-config
```

## SANTAK / UPS-MIB Notes

The default adapter uses standard UPS-MIB OIDs:

- `1.3.6.1.2.1.33.1.4.1.0`: `upsOutputSource.0`
- `1.3.6.1.2.1.33.1.2.4.0`: `upsEstimatedChargeRemaining.0`
- `1.3.6.1.2.1.33.1.2.1.0`: `upsBatteryStatus.0`
- `1.3.6.1.2.1.33.1.2.2.0`: `upsSecondsOnBattery.0`
- `1.3.6.1.2.1.33.1.2.3.0`: `upsEstimatedMinutesRemaining.0`

When the UPS/NMC supports `upsSecondsOnBattery.0`, `on_battery_duration` uses the real battery runtime reported by the UPS. This means even if `ups-monitor` starts after the power failure, the countdown begins from the actual time the UPS switched to battery. If this OID is unconfigured or returns an invalid value, the program falls back to counting from the first local poll that detected battery power.

If your SANTAK NMC returns different values, you can override `ups.oids` or `ups.mapping` in the YAML. If you need to support a UPS using a different MIB in the future, you can add a new adapter implementing `UpsAdapter` in `src/ups.rs` and select it via `ups.adapter`.

## SNMP

The SNMP client supports SNMP v1, v2c, and v3. For v3, `security_level` can be set to `noAuthNoPriv`, `authNoPriv`, or `authPriv`. For v1/v2c, `username` is used as the community string.

## Linux systemd

After building, run the init command:

```sh
sudo ./target/release/ups-monitor init
```

`init` copies the current binary to `/usr/local/bin/ups-monitor`, installs `/etc/systemd/system/ups-monitor.service`, and creates an annotated sample configuration at `/etc/ups-monitor.yaml` if it does not already exist. Existing configuration and service files are not overwritten by default; use `--force` if you need to overwrite.

`/etc/systemd/system` is the standard directory for locally-administered custom services on systemd distributions, covering Debian/Ubuntu, RHEL/CentOS/Rocky/Alma, Fedora, Arch, openSUSE, and other common systemd systems. If your distribution has special conventions for systemd unit directories, you can override the path:

```sh
sudo ./target/release/ups-monitor init \
  --service-path /etc/systemd/system/ups-monitor.service
```

`init` detects `systemctl`. For Linux distributions that do not use systemd, such as OpenRC, runit, or s6, you will need to install the daemon using the corresponding service manager; the program itself can still be run in the foreground:

```sh
sudo /usr/local/bin/ups-monitor --config /etc/ups-monitor.yaml run
```

Edit `/etc/ups-monitor.yaml`, then enable and start the service:

```sh
sudo systemctl enable --now ups-monitor.service
```

You can also have `init` enable and start the service:

```sh
sudo ./target/release/ups-monitor init --enable --start
```

Check status and logs:

```sh
systemctl status ups-monitor.service
journalctl -u ups-monitor.service -f
```

Stop or restart:

```sh
sudo systemctl stop ups-monitor.service
sudo systemctl restart ups-monitor.service
```

## macOS

On macOS you can run directly in the foreground:

```sh
ups-monitor --config ./examples/ups-monitor.yaml run
```

Use the following shutdown command in your YAML:

```yaml
shutdown:
  command: ["/sbin/shutdown", "-h", "+0"]
```
