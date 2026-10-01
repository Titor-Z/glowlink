# 萤光 GlowLink

跨平台近场文件传输。局域网内设备自动互相发现，点对点直传文件，不经云端。
传输已启用**端到端加密**（X25519 + ChaCha20-Poly1305），首次配对核对 6 位校验码，之后免确认。

除密码学依赖（RustCrypto 系）外零第三方依赖：协议帧、CRC32 校验、环形缓冲背压、配置解析全部手写。

## 下载与运行

从 [Releases](../../releases) 下载对应平台的压缩包（Windows 选 `windows-x86_64`），解压后：

```bash
# 后台常驻（推荐）：收到文件时等你确认；加 --yes 则对已信任设备免确认
./glowlink on --dir ./received

# 前台常驻：交互式，直接在终端里按 y 确认（Ctrl-C 退出）
./glowlink serve --dir ./received

# 看设备 / 发文件（终端里直接跑 ./glowlink 就是交互式界面）
./glowlink                 # 交互式界面：↑↓ 选择 · 回车（设备：输入文件路径 / 待确认：接受）
                           #   · d 拒绝待确认 · r 刷新 · q 退出（退出后后台服务继续跑）
./glowlink ui              # 同上，显式写法
./glowlink help send            # 单条命令的详细用法（也可 glowlink send --help）
./glowlink list            # 一次性列表（守护进程在跑时结果来自它）

# 发送：按设备名或 ID 发送（可多个文件）
./glowlink send 客厅的Mac 照片.jpg 报告.pdf

# 直连：绕过发现层，适合跨网段或调试
./glowlink send --peer 192.168.1.23:52878 文件.zip

# 查看/取消已配对设备
./glowlink trust
./glowlink trust forget 客厅的Mac

# 停止后台守护进程
./glowlink off
```

Windows 上二进制名为 `glowlink.exe`，用法相同。首次运行会弹出防火墙提示，**必须允许**（需要接收入站 TCP 连接与 UDP 发现报文）。

### 首次配对

第一次和某台设备互传时，两端屏幕都会出现同一个 **6 位校验码**，核对一致后确认即可：

```
[等待确认 #001] 192.168.1.23 想发来「报告.pdf」（12.4 MB）
             设备指纹 37ba779c450001b9  校验码 **482913**（请与发送方屏幕核对）
             确认：glowlink accept 001 / glowlink reject 001
```

确认后该设备进入信任列表（`~/.glowlink/trusted.txt`），此后不再核对校验码。
**免确认自动接收**需要守护进程带 `--yes` 启动，且只对已信任设备生效
（陌生设备即使开了 `--yes` 也必须人工确认）。

### macOS 解压后无法运行

下载的二进制没有代码签名，会被 Gatekeeper 隔离。执行一次即可：

```bash
chmod +x glowlink
xattr -dr com.apple.quarantine glowlink
```

## 两台机器怎么连

1. **接同一个 Wi-Fi / 路由器**（最简单）。
2. **没有路由器**：让 Windows 开「移动热点」，另一台加入该热点。
   注意 Windows 需要先连上某个网络（哪怕没有外网）才能开启移动热点。
3. 都不行时用 `--peer ip:端口` 直连，接收端的传输端口由 `on` / `serve` 启动时打印。

## 桌面版（玻璃小窗）

除了命令行，还有一个 320×500 的桌面小窗（预览版，尚未随压缩包分发）：

```bash
cargo build -p glowlink-desktop        # 产物 target/debug/glowlink-desktop
./target/debug/glowlink-desktop        # 打开即用；没在跑的服务会被自动拉起
```

怎么用：**点设备选中**（选中后底部出现「拖到此处 → 甲机」）→ 把文件拖进窗口
（或点底部那条虚线框选文件）→ 开始传。还没配对过的设备会弹一张卡片让你核对
**6 位校验码**（要跟对方屏幕上的一致）再接受；配对过就免确认、直接收。

- 标题栏右侧的开关是**服务开关**（等于 `glowlink on` / `glowlink off`）；`✕` 只关窗口。
- 一次拖多个文件会**按顺序**发：卡片上写 `2/5`，✕ = 取消这一批。
- 想在一台机器上看到第二台"设备"（真协议、不是假数据）：
  `./scripts/local-peer.sh`（独立 HOME + 同一发现端口），用完 `--stop`。

## 配置文件

分两层，职责不同：

| 层 | 在哪里 | 什么时候生效 | 给谁用 |
|---|---|---|---|
| **工程配置** | 仓库里的 `glowlink.toml` | **编译期**（`include_str!` 编进二进制） | 开发者：工程常量（端口、超时、分块大小） |
| **用户配置** | `~/.config/glowlink/config.toml`（可用 `$GLOWLINK_CONFIG` 指定） | 运行时 | 用户：改接收目录、临时换端口 |

```
内置默认 < 编译期嵌入的 glowlink.toml < ~/.config/glowlink/config.toml < 命令行开关
```

**工程配置改了要重新编译才生效**（这是刻意的）：它不随压缩包分发，也不会在运行目录里去找。
用户配置不存在就跳过，只当那一层没有覆盖。

可改的项：`discovery_port`、`transfer_port`、`receive_dir`、`query_interval_secs`、
`device_timeout_secs`、`scan_timeout_secs`、`connect_timeout_secs`、`pending_timeout_secs`、
`chunk_size_kib`、`ring_capacity_kib`、`discovery_port_span`（发现端口被占时的顺延跨度，
同时也是查询覆盖的候选端口数，默认 21；**局域网内要一致**）。用 `glowlink config` 查看当前生效值与各层是否加载。

## 注意事项

- **必须在同一子网**：发现依赖 UDP 广播，路由器的客户端隔离（AP isolation）、跨网段、VPN 都会让它失效，这种情况下请用 `--peer` 直连。
- **两端发现端口要一致**：默认 `9267`。
- **未配对设备无法传输**：传输全程加密，且接收端要人工确认或该设备已在信任列表里。
- **加密实现未经第三方审计**：用的是成熟库（RustCrypto），但整体协议没有经过专业审计，请自行判断风险。
- 设备名取自主机名，可用 `GLOWLINK_NAME=自定义名` 固定。
- 密钥与信任列表在 `~/.glowlink/`：`identity.txt`（设备身份私钥，Unix 上 0600）、
  `trusted.txt`（已配对设备，可直接手改，删一行即取消该设备的免确认）。
  删除 `identity.txt` 等于把这台机器变成一台新设备（需要重新配对）。

## 当前进度

| 阶段 | 内容 | 状态 |
| :--- | :--- | :--- |
| Phase 1 | Rust CLI：发现 + 传输 + 背压 + 校验 | 已完成 |
| Phase 2 | 守护进程 + 本地 IPC + `on`/`off` | 已完成 |
| Phase 3 | 端到端加密、配对信任、免确认自动接收 | 已完成 |
| Phase 4 | 桌面端 GPUI 小窗口 | 未开始 |
| Phase 5 | Android App 与分享菜单 | 未开始 |
| Phase 6 | Wi-Fi Direct 自组网 | 未开始 |

开发规范、决策记录与踩坑总结见 [AGENTS.md](./AGENTS.md)；产品设计书见 [why.md](./why.md)。
