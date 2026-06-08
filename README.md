# UPS Monitor

`ups-monitor` 是一个小型 Rust 守护进程，用于通过 SNMP 轮询 UPS 网络管理卡，并在 UPS
使用电池供电时间过长或电池电量过低时关闭主机。

SNMP 访问直接使用纯 Rust `async-snmp` crate，不依赖 net-snmp 命令行工具。你的 NMC
可以先用下面的命令确认网络和账号无误：

```sh
snmpwalk -v3 -l noAuthNoPriv -u local3 192.168.0.255 1.3.6.1.2.1.1.1.0
```

## 功能

- 定期进行 SNMP 轮询。
- 使用纯 Rust SNMP 客户端，支持 SNMPv3 `noAuthNoPriv`、`authNoPriv` 和 `authPriv`。
- 带注释的 YAML 配置文件，示例位于 `examples/ups-monitor.yaml`。
- 关机触发模式：
  - `battery_capacity`：仅当电量低于配置的百分比时关机。
  - `on_battery_duration`：仅当电池供电持续时间超过配置值后关机。
  - `either`：同时监控以上两个条件，任一条件满足即关机。
- 市电恢复后取消电池供电持续时间倒计时。
- 记录 UPS 状态变化日志。
- 为未来的非标准 NMC 卡预留 UPS 适配器边界。
- 提供 `init` 命令在 Linux 上安装 systemd 服务和样本配置。

## 构建与测试

```sh
cargo test
cargo build --release
```

运行时没有额外 SNMP 命令依赖，只需要能访问 UPS NMC 的网络。

## 配置

创建配置文件：

```sh
sudo cp examples/ups-monitor.yaml /etc/ups-monitor.yaml
sudo editor /etc/ups-monitor.yaml
```

首次运行时需要重点检查的设置：

```yaml
shutdown:
  dry_run: true
```

在验证 SNMP 数值和日志期间，请保持 `dry_run: true`。只有在你准备启用真实自动关机时，
才将它改为 `false`。

验证配置文件：

```sh
ups-monitor --config /etc/ups-monitor.yaml check-config
```

只轮询一次：

```sh
ups-monitor --config /etc/ups-monitor.yaml run --once
```

在前台运行：

```sh
ups-monitor --config /etc/ups-monitor.yaml run
```

打印带注释的默认配置：

```sh
ups-monitor print-default-config
```

## SANTAK / UPS-MIB 说明

默认适配器使用标准 UPS-MIB OID：

- `1.3.6.1.2.1.33.1.4.1.0`：`upsOutputSource.0`
- `1.3.6.1.2.1.33.1.2.4.0`：`upsEstimatedChargeRemaining.0`
- `1.3.6.1.2.1.33.1.2.1.0`：`upsBatteryStatus.0`
- `1.3.6.1.2.1.33.1.2.3.0`：`upsEstimatedMinutesRemaining.0`

如果你的 SANTAK NMC 返回不同的值，可以在 YAML 中覆盖 `ups.oids` 或 `ups.mapping`。
如果未来要支持使用不同 MIB 的 UPS，可以在 `src/ups.rs` 中新增一个实现 `UpsAdapter`
的适配器，然后通过 `ups.adapter` 选择它。

## SNMP

SNMP 客户端支持 SNMP v1、v2c 和 v3。对于 v3，`security_level` 可设置为
`noAuthNoPriv`、`authNoPriv` 或 `authPriv`。对于 v1/v2c，`username` 会作为
community string 使用。

## Linux systemd

构建后执行初始化命令：

```sh
sudo ./target/release/ups-monitor init
```

`init` 会复制当前二进制到 `/usr/local/bin/ups-monitor`，安装
`/etc/systemd/system/ups-monitor.service`，并在 `/etc/ups-monitor.yaml` 不存在时创建
带注释的样本配置文件。已有配置文件和服务文件默认不会被覆盖；需要覆盖时加 `--force`。

`/etc/systemd/system` 是 systemd 发行版中用于本机管理员自定义服务的通用目录，适用于
Debian/Ubuntu、RHEL/CentOS/Rocky/Alma、Fedora、Arch、openSUSE 等常见 systemd 系统。
如果你的发行版对 systemd 单元目录有特殊约定，可以覆盖路径：

```sh
sudo ./target/release/ups-monitor init \
  --service-path /etc/systemd/system/ups-monitor.service
```

`init` 会检测 `systemctl`。不使用 systemd 的 Linux 发行版，例如 OpenRC、runit 或 s6
系统，需要用对应的服务管理器安装守护进程；程序本身仍可用前台方式运行：

```sh
sudo /usr/local/bin/ups-monitor --config /etc/ups-monitor.yaml run
```

编辑 `/etc/ups-monitor.yaml`，然后启用并启动服务：

```sh
sudo systemctl enable --now ups-monitor.service
```

也可以让 `init` 完成启用和启动：

```sh
sudo ./target/release/ups-monitor init --enable --start
```

查看状态和日志：

```sh
systemctl status ups-monitor.service
journalctl -u ups-monitor.service -f
```

停止或重启：

```sh
sudo systemctl stop ups-monitor.service
sudo systemctl restart ups-monitor.service
```

## macOS

macOS 可以直接以前台方式运行：

```sh
ups-monitor --config ./examples/ups-monitor.yaml run
```

在 YAML 中使用以下关机命令：

```yaml
shutdown:
  command: ["shutdown", "-h", "now"]
```
