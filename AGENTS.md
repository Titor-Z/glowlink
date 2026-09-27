## 编码规范：
- 使用MVC开发模式，高内聚低耦合，使用设计模式，如单例模式、工厂模式、适配器模式等等来分模块。
- 使用抽象类、接口、子类等等手段来设计整体架构
- 无用的死代码直接删除
- 每次修复代码、改BUG等等，不要在代码外包裹，而是重写
- 任何时候，都不要偷懒
- 使用中文和用户交流
- 测试文件全部拆分，每个测试文件都针对单个模块、单个文件进行编写，不要全部汇入一个文件
- 一个问题如果重复3次没有解决，立即停止，告诉用户遇到的问题，并和用户讨论接下来的做法
- 认知纠正模块，用来存放我们在开发中已经遇到过的问题都总结
- TAOLUN 存放我们每次对话的重心的总结


## 测试与 CI 运行规范：
- 测试一源文件一份（`tests/<源文件名>.rs`），一个文件只测一个源文件，不合并。
- **本地一律走单文件/单函数入口，禁止 `cargo test --workspace`**（它会编译并链接所有测试目标，
  在低配机器上很重；全量只留给发版 CI）。
  - 类型检查：`cargo check --all-targets`（不链接，秒级）
  - 单个文件：`./scripts/t.sh <测试文件名>`
  - 单个用例：`./scripts/t.sh <测试文件名> <名字子串>`；精确匹配加 `--exact <用例名>`；只列名加 `--list`
  - **涉及并发、时序、网络协议的改动：必须把那一个测试文件跑完**——
    `cargo check` 看的是类型，看不见运行时会死锁（本次已栽过）。
- CI 只在打 `v*` 标签与手动触发时运行：仓库私有，Actions 按量计费（Linux ×1 / Windows ×2 / macOS ×10）。
  日常提交不跑 CI；**不要为了「顺手验一下」推 tag**（一次发版约 15 分钟额度）。
- 未经确认不推送远端（尤其额度见底时）；推送前先问。

## TAOLUN
> 这里存放讨论记录

### 2026-09-27 Phase 1 Rust CLI 概念验证

- 决定：AGENTS.md 的编码规范是通用 Web 口径，本项目用 Rust 落地同一套思想，映射如下：
  - 接口 / 抽象类 → `trait`（带默认实现的 trait 等同抽象基类）
  - 子类 → `struct` + `impl Trait`
  - 工厂模式 → `patterns/factory.rs`，控制器只持有 `Box<dyn Trait>`
  - 适配器模式 → `patterns/adapter.rs`，包装 `UdpSocket` / `TcpStream` 实现控制器接口
  - 单例模式 → `patterns/singleton.rs`，`OnceLock<DeviceIdentity>`
  - MVC → Model 放纯数据结构与算法（零 IO），Controller 放业务编排（只依赖 trait），View 放终端/界面表现
- 分层边界：`model` 不依赖 `controller`；`controller` 不依赖 `patterns`；只有 `patterns` 与 CLI 层做装配（依赖注入）。
- 零第三方依赖：协议、CRC32、环形缓冲、CLI 解析全部手写，符合“闭源私有协议”定位。
- 测试按模块拆文件：`tests/{crc,packet,device,ring,discovery,transfer,adapter}.rs`，一个文件只针对一个源文件（`adapter.rs` 针对真实套接字）；CLI 侧 `glowlink-cli/tests/args.rs` 针对参数解析模块。

### 2026-09-27 【网络发现决策 - v1.0】查询-单播应答

- 背景：`std::net::UdpSocket` 不暴露 `SO_REUSEPORT`，本机多实例绑定 8888 会报 `Address already in use`。
- 决策：**放弃 `libc` / 多播**（避开各平台防火墙与沙盒差异），全面改为**查询 - 单播应答**：
  - `QUERY` 帧广播到发现端口，携带自身 id / 名称 / TCP 传输端口；
  - 收到 `QUERY` 的节点向**来源地址单播**回 `HELLO`，不做二次广播；
  - 因此一次交互双方互相认识，无中心节点，也没有应答风暴（应答不再被广播）。
- 降级策略：绑定发现端口失败（`AddrInUse`）时自动改用临时端口，进入“仅主动查询模式”，只发查询、收单播应答，不再监听广播。
- 本机联调配套：`UdpDiscoveryAdapter::query` 在广播之外额外单发一份到 `127.0.0.1`（广播不保证回环），并对 ICMP 端口不可达（`ConnectionRefused`）静默续收。
- 开发者妥协：`glowlink send --peer ip:端口` 绕过发现层直连；`--discovery-port` 可改本地发现端口（局域网内需统一）。
- 结论：本机 `serve` + `list` + `send` 已实测互通（见下），不再需要 `SO_REUSEPORT`。

### 2026-09-27 【配置加载决策 - v1.0】零依赖工程配置文件

- 需求：提供一个“工程默认配置文件”，方便用户改默认端口等工程参数，不必重新编译。
- 决策：**不引入 `toml` / `serde` 等依赖**，手写扁平 `key = value` 解析器；格式如下：
  - `#` 起注释（双引号内的 `#` 不算），字符串可加双引号；
  - 不支持 `[分节]`，写了就报错并提示改用扁平键名。
- 优先级（低 → 高）：内置默认 → `./glowlink.toml`（随工程分发）→ `~/.config/glowlink/config.toml`（可由 `$GLOWLINK_CONFIG` 指定）→ 命令行开关。
- 合并语义：`Config::apply` 以现有配置为底，只覆盖文本中**显式出现**的键，所以才能逐层叠加而不会被上层默认值洗掉。
- 分层：纯解析 + 默认值放 `glowlink-core/src/model/config.rs`（零 IO）；文件定位与读取放 `glowlink-cli/src/config.rs`（IO 层）。
- 容错原则：未知键名、非法数值、缺 `=`、分节一律**报错并指出行号**，绝不静默忽略（静默忽略会让用户以为改了却没生效）。
- 防漂移：`glowlink-core/tests/config.rs` 用 `include_str!("../../glowlink.toml")` 断言分发的配置与内置默认完全一致。
### 2026-09-27 【默认端口决策 - v1.0】发现端口 8888 → 9267

- 背景：发现端口需要一个局域网内固定、且不易与其他服务撞车的值。
- 依据：
  - `8888` 在系统服务表里已登记（macOS `/etc/services`：`ddi-udp-1 8888/udp # NewsEDGE server`），且被大量开发工具（各类本地服务器、代理面板）默认占用，撞车概率高；
  - `9267` 未登记于 `/etc/services`，实测 `lsof -nP -iUDP:9267` 无占用，属于无归属的安静区域；
  - 只需担心 UDP；TCP 传输端口默认取 `transfer_port = 0`（系统随机分配），不占固定端口。
- 单一来源：端口值只在 `model/packet.rs` 的 `DISCOVERY_PORT` 定义一次，`Config::default()` 引用它，不再另写一份。
- 兵底：即使日后 9267 被占，发现层会自动降级为临时端口（仍可互通），也可用配置文件或 `--discovery-port` 改。
- 实测：默认端口 9267 下 `serve` 绑定成功（`UDP *:9267`），`list` + `send` 互通，1 MB 文件 SHA-1 一致。
- 文档约定：`why.md` 是产品设计书，**冻结不再修改**；所有进展、决策、踩坑一律记在本文件。

### 2026-09-27 【构建与发包决策 - v1.0】GitHub Actions 发布二进制

- 目标：不动另一台机器上的 Rust 环境，直接从 GitHub 下载可执行文件；另一台机器是 Windows 11。
- 触发：push `main`、push `v*` 标签、PR、`workflow_dispatch`；只有标签触发才产出 Release，其余只上传 Artifact（保留 30 天）。
- 平台矩阵：`macos-14`（一个 job 交叉编译 aarch64 + x86_64）、`windows-latest`（x86_64-pc-windows-msvc）。Linux 暂不加，将来需要时再增一个 job 即可。
- **按平台拆 job，而不是“一个矩阵 + `if runner.os`”，也不拆成独立 workflow 文件**：
  - 拆 job：每个 job 的步骤全用该平台原生 shell（bash / pwsh），无分支判断，日志清晰；
  - 不拆 workflow 文件：release 需要 `needs: [macos, windows]`，拆开会变成跨 workflow 抢同一个 Release，出现竞态。
- **不建 `scripts/package.*`，打包逻辑内联在各自 job 的 YAML 里**：runner 镜像已经提供打包工具（Windows 自带 PowerShell + `Compress-Archive`，macOS/Ubuntu 自带 `zip`），额外脚本只会把同一套规则（拷哪些文件、包叫什么）再拄到第三个地方，改一处忘一处。
  - 去了重：版本号提到 workflow 级 `env.VERSION`（标签用标签名，其余用 `github.run_number`），不再在 bash/pwsh 里各算一遍。
- 平台特有部分已压到最小：只有 shell 与打包命令各写各的，其他（checkout、Rust 安装、测试、上传）全部共用。
- 私有仓库的两个坑（必读）：
  - **计费**：私有仓库 Actions 分钟数受限，macOS 按 **×10**、Windows 按 **×2** 折算，所以 macOS 只起一个 job 编两个架构；
  - **权限**：release job 需要 `contents: write`，且仓库 Settings → Actions → General → Workflow permissions 必须设为 **Read and write**，否则 `gh release create` 会 403。
- 发布流程：`git push origin main` → `git tag v0.1.4 && git push origin v0.1.4` → 各平台 job 直接建 Release。
- **全程不碰 Artifact**：账号 Actions 存储配额（私有仓库共 500MB）已满，`upload-artifact` 必然 412/失败。Release 资产不计入该额度，所以各平台 job 并行 `gh release upload --clobber`（先建 Release 的那个可能撞 422，退回只传资产）。
  - 副作用（正面）：日常 push / PR 不再产出任何 Artifact，不再占用额度；需要临时产物时另行处理。
- 实测（本地拉回来验）：三个压缩包均可下载，Windows 那份 `file` 识别为 `PE32+ executable x86-64`，macOS x86_64 那份解压后可直接运行 `help` / `config`。
- 版本号以 workspace `version` 为准（`glowlink help` 会打印），需与标签一致；产物名用标签名，手动触发时用 `dev-<短 sha>`。

### 2026-09-27 【守护进程决策 - v1.0】on/off + 本地 IPC + 人工确认

- 目标：`glowlink on` / `off` 无感常驻，`list` / `send` 不再是独立进程抢端口。
- 守护进程以**分离进程**运行：Unix `setsid`（直接声明 libc 符号，不引依赖）、
  Windows `DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP`；stdin 关掉，输出进 `~/.glowlink/daemon.log`。
- 状态文件 `~/.glowlink/state.json` 是 CLI 找到守护进程的**唯一入口**：
  记录版本/PID/IPC 端口/传输端口/**实际**发现端口/接收目录；
  **临时文件 + rename 原子写**，`off` 与自退都删除，避免残留被误判成还在跑。
  判定「在跑」的标准是「能连上并 PING 通」，不是「文件存在」。
- IPC：回环 TCP，首选端口默认 10000（被占顺延 50）。选**面向行文本协议**而非 JSON：
  零依赖、`nc` 可手调、日志可读；每条响应以 `END` 收尾，单行与多行响应客户端走同一条路径。
  指令：`PING` / `STATUS` / `DEVICES` / `RESOLVE` / `PENDING` / `ACCEPT|REJECT [编号]` / `STOP`。
  模块划分：`state.rs`（状态文件）/ `ipc.rs`（协议）/ `pending.rs`（待确认队列）/ `daemon.rs`（平台层），
  Step 3 的加密插在 `ipc.rs` 的连接建立之后、指令分发之前。
- **人工确认**：分离进程没有终端，问不了人。改为「登记待确认 + 阻塞等待」，
  用户用 `glowlink accept <编号>` / `reject <编号>` 经 IPC 落决定，超时（60s）自动拒绝。
- **多槽位而非单槽**：单槽下第二个发送方会静默挂在 TCP backlog 里（既不提示也不拒绝），体验是灾难。
  所以每个 OFFER 各占一槽、各拿一个短编号（`001`/`002`），接收侧改为「每个连接一个线程」，
  等待确认互不阻塞；待确认多于 1 个时 `accept` 必须带编号，避免点错对象。
- `on --yes` 在加密与配对（Step 3）完成前**直接拒绝**，守住自动接收这条线。
- `list` / `send` 优先走 IPC，守护进程没跑时退回直连发现（保留 Phase 1 工作流）。

### 2026-09-27 【CI 降用量 - v1.0】只在打 tag 时跑

- 背景：私有仓库的 Actions **按量计费**，Free 计划每月 2000 分钟，倍率 **Linux ×1 / Windows ×2 / macOS ×10**；
  额度用尽且 `spending limit = 0` 时作业直接不启动（报 `recent account payments have failed or your spending limit needs to be increased`）。
- 本次实际烧掉的量：一次死锁的 macOS job 挂了 34 分 12 秒，×10 ≈ **342 分钟额度**（当月 1/6）；
  加上 3 次 push（macOS+Windows）≈ 40 分钟，本次会话合计约 **380 分钟 ≈ 月额度 19%**。
- 决策：`build.yml` 改为**只在 `v*` 标签与手动触发时运行**，删掉 `push: main` 与 `pull_request`；
  加 `concurrency.cancel-in-progress`、`timeout-minutes: 15`、`actions/cache`（缓存 `~/.cargo` + `target`）。
  一次发版成本 ≈ macOS(1.2分×10) + Windows(1.2分×2) ≈ **15 分钟额度**，月额度约可发版 130 次。

### 2026-09-27 【测试运行决策 - v1.0】单文件 / 单函数，不跑全量

- 问题：`cargo test --workspace` 会编译并链接**所有**测试目标，在低配机器上很重；
  而 cargo 本来就有 Jest 那种粒度（`--test <文件>` / 子串过滤 / `--exact <用例>`），是**调用方式**用错了。
- **本地与 CI 走同一套单文件入口**（本地也只用 `scripts/t.sh`，不再全量跑）。
- 三档流程：
  1. 平时改代码 → `cargo check --all-targets`（不链接、秒级，只拦类型/借用错误）；
  2. 改了某模块逻辑 → `./scripts/t.sh <测试文件> [过滤词]`（只编一个目标，再直接执行二进制）；
  3. 涉及并发/时序/网络协议 → 同上，但跑完整个那一个测试文件；
  4. 发版 → 打 tag，CI 在 macOS + Windows 上跑全量。
- 铁律：**碰并发/时序就必须本地跑那一个测试文件**。`cargo check` 看的是类型，看不见运行时会死锁。

## CHANGELOG
> 这里存放项目更新历史记录

### 2026-09-27 v0.1.0-step2 — 守护进程 + 本地 IPC + on/off（tag `v0.1.0-step2`）

- 新增 `glowlink on` / `off` / `accept [编号]` / `reject [编号]`；守护进程以分离进程常驻。
- 新增 `state.rs`（状态文件，临时文件 + rename 原子写）、`ipc.rs`（回环行协议）、
  `pending.rs`（token + 队列 + 超时自动拒绝）、`daemon.rs`（平台层）、`view/daemon.rs`（无终端的人工确认）。
- 新增内部子命令 `__daemon`（由 `on` 拉起，不面向用户）。
- `list` / `send` 优先走 IPC，守护进程没跑时退回直连发现；顺带解决多进程抢发现端口。
- 新增 `scripts/t.sh`：单文件/单函数测试入口；本地测试一律用它（不进二进制，不违反零依赖）。
- CI 触发改为**只在打 tag / 手动触发**（私有仓库 Actions 按量计费），并加 `timeout-minutes: 15`、
  `concurrency` 取消重复运行、`actions/cache` 缓存 Cargo 与 `target`。
- 测试：`tests/{state,ipc,pending,daemon_view}.rs` 共 41 个用例（本地逐文件跑过），
  另有 Phase 1 遗留用例；测试用例数过多，为控制私有仓库 CI 额度，全量只在打 tag 时跑。
- 真机验收：两个发送方同时挂起 → `PENDING` 列表（`#001`/`#002`）→ `accept 001` / `reject 002`
  → 甲文件 sha1 收发一致、乙收到「传输被拒绝」；无编号/乱编号均被拒。
- 修复：IPC 与发现循环共用锁导致查询被饿死；状态文件写了首选端口而非实际端口；
  远程错误叠三层前缀；待确认落决定后未立即从视图消失。

### 2026-09-27 v0.1.5 — CI 发布链路打通（仓库私有）

- 仓库 `Titor-Z/glowlink` 已建（private），用 `gh` 创建并推送。
- Workflow 改为**单一 `build` job + 平台矩阵**：macOS 双架构与 Windows 各跑一份，各自直接 `gh release upload`，彻底弃用 `upload-artifact`（账号存储配额已满，且 Release 资产不计额度）。
- 实测结果：
  - Windows 与 macOS 各 56 个测试全过（含 UDP 双绑定降级、真实 TCP 传输、临时目录相关用例，无平台差异问题）；
  - `v0.1.4` Release 产出三个资产：`windows-x86_64` 210KB、`macos-aarch64` 286KB、`macos-x86_64` 293KB；
  - 本地拉回验证：Windows 包内 `glowlink.exe` 为 `PE32+ executable x86-64`，macOS 包解压后可直接运行。
- 修了两个 workflow 语法/机制问题（见认知纠正）。

### 2026-09-27 v0.1.4 — CI 与 GitHub 发包

- 新增 `.github/workflows/build.yml`：macOS（arm64 + x86_64，同一 job）与 Windows（x86_64）两个独立 job，打包内联在各自 job（用镜像自带的 `zip` / `Compress-Archive`），PR/main 上传 Artifact，`v*` 标签创建 GitHub Release。
- 新增 `README.md`（同时作为压缩包内使用说明）：下载、运行、macOS 去隔离属性、防火墙、配置文件优先级、当前阶段。
- workspace 版本号 0.1.0 → 0.1.4，与发布标签对齐；同步 `Cargo.lock`。
- `.gitignore` 增加 `dist/`。
- 推 CI 前本地先验：`cargo check --locked --target x86_64-pc-windows-msvc --all-targets` 与 `--target aarch64-apple-darwin` 均通过（`cargo check` 不需要链接器）。
- 本地实跑验证：把 YAML 里的 macOS 打包段原样在本地 bash 跑一遍，两份 zip 生成正常，解压后 `glowlink` 可执行位保留、`help`/`config` 正常运行。

### 2026-09-27 v0.1.3 — 默认发现端口改为 9267

- `packet.rs` 的 `DISCOVERY_PORT` 改为 `9267`（避开已登记的 8888/NewsEDGE 及其他开发工具常用端口）。
- 去重：`Config::default()` 不再自写端口字面量，改为引用 `DISCOVERY_PORT`，消除“同一端口两处定义”。
- `glowlink-core/tests/config.rs` 的默认值断言改为对比 `glowlink_core::DISCOVERY_PORT` 常量，不再硬编码数字。
- 实测：默认 9267 下 `serve` 绑定成功，`list` + `send` 互通，1 MB 文件 SHA-1 一致。

### 2026-09-27 v0.1.2 — 工程配置文件

- 新增 `glowlink.toml`（工程默认配置）与 `glowlink-core/src/model/config.rs`（零依赖扁平解析器 + 内置默认）。
- 新增 `glowlink-cli/src/config.rs`：按 `./glowlink.toml` → `$GLOWLINK_CONFIG`（默认 `~/.config/glowlink/config.toml`）逐层叠加。
- 新增 `glowlink config` 子命令：打印生效配置与每个配置文件是否加载。
- CLI 所有硬编码默认值收归配置：发现端口、传输端口、接收目录、查询周期、设备超时、搜索时长、连接超时、分块大小、环容量；命令行开关仍优先。
- 新增 `GlowError::Config`，配置错误带文件名与行号。
- `.gitignore` 增加 `received/`。
- 实测：用 `$GLOWLINK_CONFIG` 把发现端口改为 8999、接收目录改为 `/tmp/glowlink-recv2`，`serve` + `list` + `send` 互通，4 MB 文件 SHA-1 一致。
- 测试增至 56 个用例，新增 `glowlink-core/tests/config.rs`（解析/叠加/错报/防漂移）与 `glowlink-cli/tests/config.rs`（多层加载）。

### 2026-09-27 v0.1.1 — 发现层改用查询-单播应答

- `model/packet.rs`：新增 `QUERY` 帧（KIND=7），与 `HELLO` 同负载；`Frame::Query` / `Frame::Hello` 语义分离。
- `controller/discovery.rs`：接口改为 `local_port` / `query` / `reply` / `poll` / `degraded`，新增 `DiscoveryEvent::{Query, Reply}`；`tick` 收到查询时单播应答并登记对方。
- `patterns/adapter.rs`：`UdpDiscoveryAdapter` 支持端口被占用时降级为临时端口；查询同时发广播与回环；忽略非协议报文与 ICMP 端口不可达噪声。
- `glowlink-cli`：拆成 lib + bin 以便测试；新增 `--discovery-port` 与 `send --peer ip:端口` 直连模式；端口降级时打印提示。
- 实测（单机双进程）：`serve`（占 8888）+ `list`（降级）能互相发现，按名称发送与 `--peer` 直连各传 5 MB 随机文件，SHA-1 与源文件一致。
- 测试增至 45 个用例，新增 `glowlink-cli/tests/args.rs` 参数解析用例与发现/适配器的真实 UDP 用例。

### 2026-09-27 v0.1.0 — Phase 1：Rust CLI 概念验证

- 新增 workspace：`glowlink-core`（库）+ `glowlink-cli`（二进制 `glowlink`）。
- `model/packet.rs`：私有二进制协议（魔数 `GLWK` + 版本 + 帧类型），帧类型 QUERY / HELLO / OFFER / ACCEPT / REJECT / CHUNK / DONE。
- `model/crc.rs`：增量 CRC32，用于分块与整文件校验。
- `model/ring.rs`：`RingBuffer`（纯结构）+ `SharedRing`（`Mutex` + `Condvar` 阻塞封装），实现背压控制。
- `model/device.rs`：`Device` / `DeviceBook`，广播合并、超时下线、按名称或 ID 查找。
- `controller/discovery.rs`：UDP 发现控制器（查询-单播应答），只依赖 `DiscoveryTransport` 接口。
- `controller/transfer.rs`：发送侧独立读盘线程 + 环形缓冲池（背压），接收侧逐帧校验偏移与 CRC32，拒绝接收时回 REJECT，失败清除残片。
- `patterns/`：工厂 / 适配器 / 单例三件套，装配点集中在 CLI `main.rs`。
- CLI 三个命令：`list`（扫描设备）、`serve`（常驻接收，默认询问确认，`--yes` 自动接收）、`send`（发现并发送，或 `--peer` 直连）。
- 安全：接收文件名剥离上级路径（防 `../` 路径穿越）；单帧上限 1 MiB（防超长帧内存爆炸）；非协议报文静默忽略。
- 测试：33 个用例，覆盖协议往返、截断/坏魔数/坏版本、环形缓冲回绕与背压、设备上下线、分块乱序与篡改检测、拒绝流程、路径穿越、真实 TCP 往返与真实 UDP 收发。

## 认知纠正
> 这里存放踩坑记录

- **开关取值被当成位置参数（已修复）**：`positional_args` 用 `strip_prefix("--")` 之后的名字去 `VALUE_FLAGS.contains(&name)`，而表里存的是带 `--` 的完整开关名，比较永远为假。后果：`send --peer <文件>` 会把 ip:端口当成文件，`send 设备 文件 --timeout 4` 会额外尝试发送一个名为 `4` 的文件，报 `IO 错误：No such file or directory`。
  - 纠正：开关比对一律用**完整开关名**；并为每个取值开关补一条“必须被登记”的测试（新增开关忘登记就会被测出来）。
  - 教训：CLI 胶水层也需要单测；只靠端到端跑一次不会发现“多发了不存在的文件”这种偏差，它只在报错信息里暴露。
- **UDP 发现端口 8888 同机只能一个进程绑定**：`std::net::UdpSocket` 不暴露 `SO_REUSEADDR` / `SO_REUSEPORT`，第二个实例 `bind` 报 `Address already in use`。
  - 已按【网络发现决策 - v1.0】解决：降级为临时端口 + 查询-单播应答，本机双进程已实测互通。
- **`&mut self` 方法返回内部引用时不能同时再借用集合**：`DeviceBook::observe` 先 `iter_mut().find()` 再 `push` 会触发 E0499/E0502，改为先求下标、再统一借用。
- **`Condvar::wait` 的守卫生命周期必须显式命名**：封装 `fn wait<'a>(&self, condvar: &Condvar, state: MutexGuard<'a, State>) -> MutexGuard<'a, State>`，写成 `'_, '_` 会报“lifetime may not live long enough”。
- **中文标识符/测试名不用**：Rust 虽允许，但工具链与日志对中文标识符不友好；代码内标识符用英文，注释、输出、文档用中文。
- **配置文件千万不要静默忽略未知键**：用户把 `discovery_port` 拼错成 `discovery_prot` 时，若静默忽略，现象是“改了没生效”，排查成本极高。已按【配置加载决策 - v1.0】改为报错 + 行号 + 可用键名列表。
- **分发出去的配置容易与代码里的默认值漂移**：同一份数值写在两个地方，改一处忘一处就会出现“配置文件加载了但值与默认不一致”的幽灵问题。用 `include_str!` 直接拿仓库里的配置文件做断言即可长期防漂移。
  - 实例：把 `glowlink.toml` 的端口改成 9267 后，该用例立刻报出 `glowlink.toml 与内置默认不一致 (9267 vs 8888)`。
- **同一个常量不要在两处定义**：端口默认值曾同时在 `model/packet.rs`（`DISCOVERY_PORT`）与 `model/config.rs`（`DEFAULT_DISCOVERY_PORT`）各写一份，改一处就会出现两套行为。
  - 纠正：以协议常量为唯一来源，配置层引用它；测试也改成对比常量而非硬编码数字（硬编码的数字本身就是第三份拷贝）。
- **改 workspace 版本号后 `--locked` 构建会失败**：`Cargo.lock` 里记着各 workspace 成员的版本，改了 `Cargo.toml` 必须重生成。
  - 纠正：先无 `--locked` 构建一次重新生成 lock 并提交，之后 CI 一律用 `--locked` 保证可复现。
- **zip 的目录结构与可执行位很容易搞错**：
  - 必须在 `dist` 里用相对路径打包（`cd dist && zip -r x.zip <目录>`），否则 Windows 用户解压出一堆散文件；
  - 先 `chmod +x` 再打包，`unzip` 会还原可执行位，免得用户拿到不能运行的二进制。
- **`cargo check --target` 不需要链接器**，是推 CI 前验证跨平台编译的最便宜手段（本地已用它先拦下 Windows/ARM 目标的编译问题）；真正链接与产物验证交给 CI 与 `scripts/package.sh`。
- **`permissions` 字段不接受表达式**：写 `contents: ${{ ... && 'write' || 'read' }}` 会让整个 workflow 直接不启动（run 显示 “likely failed because of a workflow file issue”，日志为空，极难定位）。
  - 纠正：`permissions` 只能写固定值；需要按条件赋权的，改为在 job 级写固定 `write`（非目标场景不会执行写操作），或用 `if` 拆 job。
  - 工具：本地装 `actionlint` 先跑一遍，一眼看出这类问题，不要靠盲推 CI 试错。
- **Artifact 存储配额是账号级共享的硬限**：私有仓库共 500MB，超了就 `Failed to CreateArtifact: Artifact storage quota has been hit`（6-12 小时才重算）。
  - 影响：`upload-artifact` 全线不可用，连“中转给 release job”都不行（中转也得先上传）。
  - 规避：让各平台 job **直接**发布 Release 资产（不计入该额度），彻底绕开 Artifact。

- **`cargo check` 全绿 ≠ 能跑**：本次 `cargo check --all-targets` 全过，CI 上 `cargo test` 却挂死 34 分钟。
  原因是我在回环用例里 `server.join()` 之前没关客户端，而服务端是「读到对端关闭才退出」的循环。
  类型系统没有任何理由拦它。
- **测试死锁在 CI 上会被平台倍率放大**：macOS 10× ⇒ 34 分 12 秒 = 342 分钟额度。
  `timeout-minutes` 是必加项，不能指望作业自己退出。
- **待确认队列「落决定」必须立即从对外视图消失**：只把条目标记成已决定、等接收线程醒来再摘除，
  会让 `STATUS`/`PENDING` 在几毫秒内仍把它算作待确认，甚至误报「有 2 个待确认请求」。
  对外视图（`list`/`count`/`decide`）一律只看**未落决定**的条目。
- **同一把锁不能跨「阻塞等收包」**：守护进程最初让 IPC 线程与发现主循环共用一把锁，
  而 `tick()` 持锁在 `poll` 里阻塞等收包 500ms ⇒ IPC 查询被彻底饿死（`STATUS`/`DEVICES` 直接超时）。
  改为发现循环独占控制器、只把**设备快照**发到共享锁里。用 `sample <pid>` 抓线程栈才定位到。
- **状态文件里的端口要写“实际绑定的”，不是“首选的”**：顺延后写首选端口，客户端会连到错误的端口。
- **`ERR` 回原文，不要回 `GlowError` 的 Display**：服务端回 Display、客户端再包一层，
  用户会看到 `错误：协议错误：用法错误：有 2 个待确认请求…` 三层前缀。已加 `GlowError::Remote` 原样展示。
- **测试改 `HOME` 必须串行化**：`HOME` 是进程级环境变量，而测试默认并行跑，
  不串行就会互相把 `HOME` 改到别的目录，造出「写 A 读 B」的假故障。
- **后台子进程会拖住调用方的输出管道**：真机验收里后台跑的 CLI 会继承管道，
  让调用方一直等它（表现为「命令超时」而不是「代码挂住」）。
  改用 python `start_new_session=True` 起独立会话，结论落文件。

- **GitHub Actions 免费额度只对 public 仓库**：私有仓库是**按分钟计费**的
  （Free 计划 2000 分钟/月，倍率 Linux ×1 / Windows ×2 / **macOS ×10**）。
  额度用尽且 `spending limit = 0` 时，作业**不会启动**，报
  `recent account payments have failed or your spending limit needs to be increased`。
  这不是「排队」，别再等它。
- **读计费数字需要带 billing 权限的 token**：只有 `repo` 权限时 `/user/settings/billing/actions` 直接 404，
  想知道确切用量只能去网页 Settings → Billing and plans。
- **`git checkout <路径>` 恢复的是「已提交」的版本，不是「我改之前」的版本**：
  我把 AGENTS.md 改坏后先提交、再 `git checkout AGENTS.md`，结果把坏版本又恢复了一遍。
  要回到某个好状态必须显式给提交：`git checkout <好提交> -- <路径>`。
- **分区结构的文档不能直接 `>>` 追加**：内容会落到最后一个分区（`## 其他`）后面，
  还会造出重复的 `## 认知纠正` 标题。必须用分区标题当锚点插入，插入后用
  `grep -n "^## "` 自检每个分区只出现一次。

## 其他
[why.md](./why.md) 存放的是项目的产品设计书，**冻结不改**；开发进展、决策、踩坑一律写在本文件（TAOLUN / CHANGELOG / 认知纠正）。
