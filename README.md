# 萤光 GlowLink

跨平台近场文件传输（Phase 1：CLI 概念验证）。局域网内设备自动互相发现，点对点直传文件，不经云端。

闭源、零第三方依赖：协议、CRC32 校验、环形缓冲背压、配置解析全部手写，只用 Rust 标准库。

## 下载与运行

从 [Releases](../../releases) 下载对应平台的压缩包（Windows 选 `windows-x86_64`），解压后：

```bash
# 接收方：常驻等待，默认需要按 y 确认，--yes 自动接收
./glowlink serve --dir ./received

# 任一台：扫描局域网内的设备
./glowlink list

# 发送：按设备名或 ID 发送（可多个文件）
./glowlink send 客厅的Mac 照片.jpg 报告.pdf

# 直连：绕过发现层，适合跨网段或调试
./glowlink send --peer 192.168.1.23:52878 文件.zip
```

Windows 上二进制名为 `glowlink.exe`，用法相同。首次运行会弹出防火墙提示，**必须允许**（需要接收入站 TCP 连接与 UDP 发现报文）。

### macOS 解压后无法运行

下载的二进制没有代码签名，会被 Gatekeeper 隔离。执行一次即可：

```bash
chmod +x glowlink
xattr -dr com.apple.quarantine glowlink
```

## 配置文件

同目录下的 `glowlink.toml` 是工程默认配置，可直接修改。优先级：

```
内置默认 < ./glowlink.toml < ~/.config/glowlink/config.toml（可用 $GLOWLINK_CONFIG 指定）< 命令行开关
```

可改的项：`discovery_port`、`transfer_port`、`receive_dir`、`query_interval_secs`、`device_timeout_secs`、`scan_timeout_secs`、`connect_timeout_secs`、`chunk_size_kib`、`ring_capacity_kib`。用 `glowlink config` 查看当前生效值与配置文件位置。

## 注意事项

- **必须在同一子网**：发现依赖 UDP 广播，路由器的客户端隔离（AP isolation）、跨网段、VPN 都会让它失效，这种情况下请用 `--peer` 直连。
- **两端发现端口要一致**：默认 `9267`。
- **尚无加密**：任何人都能向你的机器发起传输，唯一的闸门是接收端的确认提示。**不要用它传敏感文件**。
- 设备名取自主机名，可用 `GLOWLINK_NAME=自定义名` 固定。

## 当前进度

| 阶段 | 内容 | 状态 |
| :--- | :--- | :--- |
| Phase 1 | Rust CLI：发现 + 传输 + 背压 + 校验 | 已完成 |
| Phase 2 | 桌面端 GPUI 小窗口 | 未开始 |
| Phase 3 | Android App 与分享菜单 | 未开始 |
| Phase 4 | 端到端加密、配对信任、Wi-Fi Direct 自组网 | 未开始 |

开发规范、决策记录与踩坑总结见 [AGENTS.md](./AGENTS.md)；产品设计书见 [why.md](./why.md)。
