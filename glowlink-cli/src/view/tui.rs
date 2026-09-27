//! 交互式界面（ratatui）。
//!
//! 依赖边界：**只有这个模块与 CLI 命令层用 TUI 库**，核心协议层（帧 / 发现 / 传输 / 加密）
//! 依然是零依赖（见 AGENTS.md「信息暴露与输出规范」的依赖政策）。
//!
//! 职责边界：界面是**守护进程的前端**，不自己保管任何状态。
//! - 设备表与待确认表都来自守护进程（IPC `DEVICES` / `PENDING`，或界面自己起的发现线程）；
//! - 接受 / 拒绝通过 IPC `ACCEPT <编号>` / `REJECT <编号>` 交给守护进程执行；
//! - 因此界面退出后守护进程照常工作，新请求也不会丢。
//!
//! 「选设备发文件」仍是**选完退出界面**再走原发送流程：不重写进度渲染，界面崩溃也不牵连传输。
//!
//! 文件路径的来源：终端里可以直接把文件**拖进来**（等于粘贴路径），所以底部就是一个输入行。

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind};
use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Modifier, Style as TuiStyle};
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders, Paragraph, Row, Table, TableState};
use ratatui::Frame;

use crate::view::cli::human_size;
use glowlink_core::model::error::Result;
use glowlink_core::Device;

/// 界面刷新间隔（发现线程/守护进程查询在后台跑，这里只重绘）。
const TICK: Duration = Duration::from_millis(200);

/// 用户做出的、需要界面之外去执行的动作。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TuiAction {
    /// 退出界面（守护进程继续跑）。
    Quit,
    /// 选中了设备并给了文件路径。
    Send { addr: std::net::SocketAddr, name: String, path: PathBuf },
    /// 接受某个待确认请求（编号见界面）。
    Accept { token: String },
    /// 拒绝某个待确认请求。
    Reject { token: String },
}

/// 一条待确认的接收请求。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PendingItem {
    pub token: String,
    pub peer: String,
    pub name: String,
    pub size: u64,
    /// 对端设备指纹。
    pub fingerprint: String,
    /// 6 位校验码：两端相同，**必须由人对面比对**后才接受。
    pub code: String,
}

/// 界面用到的静态信息（本机身份与后台服务状态）。
pub struct Header {
    pub version: String,
    pub local_name: String,
    pub local_id: String,
    /// 后台服务的 PID；没跑时为 `None`。
    pub daemon: Option<u32>,
}

/// 一份完整的界面数据快照。
#[derive(Clone, Debug, Default)]
pub struct Snapshot {
    pub devices: Vec<Device>,
    pub pending: Vec<PendingItem>,
}

/// 光标当前落点。
pub enum Focus<'a> {
    Device(&'a Device),
    Pending(&'a PendingItem),
    None,
}

/// 底部输入行的状态。
#[derive(Default)]
struct Input {
    text: String,
    /// 是否处于「输入文件路径」模式（回车进入）。
    active: bool,
}

/// 界面状态。**这部分是纯逻辑，可单测**（渲染与事件循环在外面）。
///
/// 设备与待确认共用**一个光标**：先走设备、再走待确认，回车在不同项上含义不同
/// （设备 → 输入文件路径；待确认 → 接受）。这样 ↑↓ 一路走到底，
/// 不必先学「怎么在两个列表之间切焦点」。
#[derive(Default)]
pub struct AppState {
    devices: Vec<Device>,
    pending: Vec<PendingItem>,
    cursor: usize,
    /// 上一次选中的设备 ID / 待确认编号：刷新后尽量停在原来那一项上。
    remembered_device: Option<String>,
    remembered_pending: Option<String>,
    input: Input,
}

impl AppState {
    pub fn new() -> Self {
        AppState::default()
    }

    /// 用新快照刷新（保持光标尽量停在原来那一项上）。
    pub fn set_snapshot(&mut self, snapshot: Snapshot) {
        let was_device = self.remembered_device.clone();
        let was_pending = self.remembered_pending.clone();
        self.devices = snapshot.devices;
        self.pending = snapshot.pending;

        // 优先找回「原来那一项」；找不到再把下标夹回有效范围
        let device_at = was_device
            .as_deref()
            .and_then(|id| self.devices.iter().position(|d| d.id.as_str() == id));
        let pending_at = was_pending
            .as_deref()
            .and_then(|token| self.pending.iter().position(|p| p.token == token));
        self.cursor = match (device_at, pending_at) {
            (Some(index), _) => index,
            (None, Some(index)) => self.devices.len() + index,
            (None, None) => self.cursor.min(self.total().saturating_sub(1)),
        };
        self.sync_remembered();
    }

    pub fn devices(&self) -> &[Device] {
        &self.devices
    }

    pub fn pending(&self) -> &[PendingItem] {
        &self.pending
    }

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    pub fn total(&self) -> usize {
        self.devices.len() + self.pending.len()
    }

    pub fn focus(&self) -> Focus<'_> {
        if self.cursor < self.devices.len() {
            Focus::Device(&self.devices[self.cursor])
        } else if let Some(item) = self.pending.get(self.cursor - self.devices.len()) {
            Focus::Pending(item)
        } else {
            Focus::None
        }
    }

    /// 下移一格；到底就停住（不循环，免得手一抖从第一台跳到最后一台）。
    pub fn select_next(&mut self) {
        if self.cursor + 1 < self.total() {
            self.cursor += 1;
        }
        self.sync_remembered();
    }

    pub fn select_previous(&mut self) {
        self.cursor = self.cursor.saturating_sub(1);
        self.sync_remembered();
    }

    /// 回车：设备进输入模式；待确认则接受（**校验码必须已与对方核对过**）。
    pub fn activate(&mut self) -> Option<TuiAction> {
        match self.focus() {
            Focus::Device(_) => {
                self.begin_input();
                None
            }
            Focus::Pending(item) => Some(TuiAction::Accept { token: item.token.clone() }),
            Focus::None => None,
        }
    }

    /// 拒绝当前项；对设备无效（拒绝对设备没有意义）。
    pub fn reject_current(&mut self) -> Option<TuiAction> {
        match self.focus() {
            Focus::Pending(item) => Some(TuiAction::Reject { token: item.token.clone() }),
            _ => None,
        }
    }

    pub fn input_active(&self) -> bool {
        self.input.active
    }

    pub fn input(&self) -> &str {
        &self.input.text
    }

    /// 进入输入模式（回车）。只有在设备上才有意义。
    pub fn begin_input(&mut self) -> bool {
        if !matches!(self.focus(), Focus::Device(_)) {
            return false;
        }
        self.input.active = true;
        self.input.text.clear();
        true
    }

    pub fn cancel_input(&mut self) {
        self.input.active = false;
        self.input.text.clear();
    }

    pub fn push_char(&mut self, ch: char) {
        if self.input.active {
            self.input.text.push(ch);
        }
    }

    pub fn pop_char(&mut self) {
        if self.input.active {
            self.input.text.pop();
        }
    }

    /// 确认输入：路径为空或焦点已不在设备上时返回 `None`（由调用方决定提示什么）。
    pub fn confirm_input(&mut self) -> Option<TuiAction> {
        if !self.input.active {
            return None;
        }
        let path = parse_path_input(&self.input.text);
        let Focus::Device(device) = self.focus() else {
            return None;
        };
        if path.as_os_str().is_empty() {
            return None;
        }
        let action = TuiAction::Send {
            addr: device.addr,
            name: device.name.clone(),
            path,
        };
        self.input.active = false;
        self.input.text.clear();
        Some(action)
    }

    /// 把「当前焦点是谁」记下来，供下一次刷新找回。
    fn sync_remembered(&mut self) {
        // 先把「是谁」抄出来，再去改字段：否则 focus() 的借用与赋值会打架
        let anchor = match self.focus() {
            Focus::Device(device) => Some((Some(device.id.as_str().to_string()), None)),
            Focus::Pending(item) => Some((None, Some(item.token.clone()))),
            Focus::None => Some((None, None)),
        };
        if let Some((device, pending)) = anchor {
            self.remembered_device = device;
            self.remembered_pending = pending;
        }
    }
}

/// 把输入行里的文本解析成路径。
///
/// 拖拽文件到终端时，shell 给的是一串**带引号或反斜杠转义**的文本
/// （`'/tmp/有 空格.pdf'`、`/tmp/有\ 空格.pdf`），所以要剥掉一层引号、
/// 还原转义的空格，否则会把引号当文件名的一部分（文件肯定找不到）。
pub fn parse_path_input(text: &str) -> PathBuf {
    let trimmed = text.trim();
    // 只剥一对**匹配的**引号：`"a'` 这种半截的不动
    let unquoted = if trimmed.len() >= 2 {
        let bytes = trimmed.as_bytes();
        let (first, last) = (bytes[0], bytes[bytes.len() - 1]);
        if (first == b'\'' && last == b'\'') || (first == b'"' && last == b'"') {
            &trimmed[1..trimmed.len() - 1]
        } else {
            trimmed
        }
    } else {
        trimmed
    };
    // 反斜杠转义的空格（`\ `）还原成普通空格；Windows 路径是 `\` 后跟字母，不受影响
    PathBuf::from(unquoted.replace("\\ ", " "))
}

/// 启动界面并跑到用户做出**需要退出界面**的动作（发文件或退出）。
///
/// 接受 / 拒绝不退出界面：由 `decide` 回调交给调用方执行 IPC，然后继续跑。
pub fn run<F>(
    snapshot: Arc<Mutex<Snapshot>>,
    header: Header,
    decisions_allowed: bool,
    mut decide: F,
) -> Result<TuiAction>
where
    F: FnMut(&str, &str) -> std::result::Result<String, String>,
{
    let mut terminal = ratatui::init();
    let mut state = AppState::new();
    let result = event_loop(
        &mut terminal,
        &mut state,
        &snapshot,
        &header,
        decisions_allowed,
        &mut decide,
    );
    // 无论正常退出还是出错都要还原终端，否则用户的 shell 会留在备用屏里
    ratatui::restore();
    result
}

#[allow(clippy::too_many_arguments)]
fn event_loop<F>(
    terminal: &mut ratatui::DefaultTerminal,
    state: &mut AppState,
    snapshot: &Arc<Mutex<Snapshot>>,
    header: &Header,
    decisions_allowed: bool,
    decide: &mut F,
) -> Result<TuiAction>
where
    F: FnMut(&str, &str) -> std::result::Result<String, String>,
{
    let mut last_refresh = Instant::now();
    // 上一次操作的反馈（成功为 false，失败为 true）；显示在底部
    let mut status: Option<(bool, String)> = None;

    loop {
        // 定期把后台线程的结果搬进来（收包与 IPC 查询都会阻塞，不能放界面线程）
        if last_refresh.elapsed() >= TICK {
            state.set_snapshot(snapshot.lock().unwrap_or_else(|e| e.into_inner()).clone());
            last_refresh = Instant::now();
        }

        terminal
            .draw(|frame| draw(frame, state, header, decisions_allowed, status.as_ref()))
            .map_err(glowlink_core::GlowError::Io)?;

        // 事件轮询也用 TICK 做上限：没有按键时到点就重绘一次
        if !event::poll(TICK).map_err(glowlink_core::GlowError::Io)? {
            continue;
        }
        let Event::Key(key) = event::read().map_err(glowlink_core::GlowError::Io)? else {
            continue;
        };
        // Windows 上会同时收到按下与抬起，只处理按下（否则一次按键算两次）
        if key.kind != KeyEventKind::Press {
            continue;
        }

        let mut refresh = || snapshot.lock().unwrap_or_else(|e| e.into_inner()).clone();
        match apply_key(state, key, &mut refresh) {
            Some(TuiAction::Quit) => return Ok(TuiAction::Quit),
            // 发文件要离开界面：交给调用方走原发送流程
            Some(action @ TuiAction::Send { .. }) => return Ok(action),
            Some(TuiAction::Accept { token }) => {
                status = Some(decide_show(decide, "ACCEPT", &token, snapshot));
            }
            Some(TuiAction::Reject { token }) => {
                status = Some(decide_show(decide, "REJECT", &token, snapshot));
            }
            None => {}
        }
        // 操作过后马上刷新一次，别让用户对着旧列表发呆
        state.set_snapshot(snapshot.lock().unwrap_or_else(|e| e.into_inner()).clone());
    }
}

/// 执行一次接受/拒绝，并把结果整理成一行反馈。
fn decide_show<F>(
    decide: &mut F,
    verb: &str,
    token: &str,
    snapshot: &Arc<Mutex<Snapshot>>,
) -> (bool, String)
where
    F: FnMut(&str, &str) -> std::result::Result<String, String>,
{
    let done = if verb == "ACCEPT" { "已接受" } else { "已拒绝" };
    let outcome = decide(verb, token);
    // 后端状态可能已经变了，立刻重取
    let _ = snapshot;
    match outcome {
        Ok(_) => (false, format!("{done} #{token}")),
        Err(e) => (true, format!("操作失败：{e}")),
    }
}

/// 把一个按键喂给状态机，返回需要执行的动作。
///
/// 界面事件循环与单元测试都走这一个函数——按键语义只有一处实现，
/// 不会出现「界面上能用、测试里不是同一套」的漂移。
pub fn apply_key(
    state: &mut AppState,
    key: KeyEvent,
    refresh: &mut dyn FnMut() -> Snapshot,
) -> Option<TuiAction> {
    if state.input_active() {
        match key.code {
            KeyCode::Esc => state.cancel_input(),
            KeyCode::Backspace => state.pop_char(),
            KeyCode::Enter => match state.confirm_input() {
                // 路径为空就当作取消，别把空路径当文件去发
                Some(action) => return Some(action),
                None => state.cancel_input(),
            },
            KeyCode::Char(ch) => state.push_char(ch),
            _ => {}
        }
        return None;
    }
    match key.code {
        KeyCode::Char('q') | KeyCode::Esc => return Some(TuiAction::Quit),
        KeyCode::Down | KeyCode::Char('j') => state.select_next(),
        KeyCode::Up | KeyCode::Char('k') => state.select_previous(),
        KeyCode::Enter => return state.activate(),
        KeyCode::Char('d') | KeyCode::Delete => return state.reject_current(),
        KeyCode::Char('r') => state.set_snapshot(refresh()),
        _ => {}
    }
    None
}

/// 绘制界面。
fn draw(
    frame: &mut Frame,
    state: &AppState,
    header: &Header,
    decisions_allowed: bool,
    status: Option<&(bool, String)>,
) {
    let area = frame.area();
    let pending_rows = state.pending().len();
    // 待确认区：有请求时按条数给高度（含表头与边框），没有时只留一行提示
    let pending_height = if pending_rows == 0 {
        3
    } else {
        (pending_rows as u16 + 4).min(9)
    };
    // 顶部 4 行 = 上下边框 + 2 行内容（身份一行、后台服务状态一行）；
    // 写成 3 行会把第二行内容裁掉（界面上看起来像「状态没显示」）
    let chunks = Layout::vertical([
        Constraint::Length(4),
        Constraint::Min(3),
        Constraint::Length(pending_height),
        Constraint::Length(3),
    ])
    .split(area);

    // 顶部：本机身份 + 后台服务状态
    let daemon = match (&header.daemon, pending_rows) {
        (None, _) => "后台服务：未启动（glowlink on 可常驻）".to_string(),
        (Some(pid), 0) => format!("后台服务：运行中（PID {pid}）"),
        (Some(pid), n) => format!("后台服务：运行中（PID {pid}，待确认 {n}）"),
    };
    frame.render_widget(
        Paragraph::new(vec![
            Line::from(format!(
                "萤光 GlowLink {}    本机：{}（{}）",
                header.version, header.local_name, header.local_id
            )),
            Line::from(daemon),
        ])
        .block(Block::default().borders(Borders::ALL).title(" 设备 ")),
        chunks[0],
    );

    // 中部：设备表
    let rows: Vec<Row> = state
        .devices()
        .iter()
        .map(|device| {
            Row::new(vec![
                device.name.clone(),
                device.short_id().to_string(),
                device.addr.to_string(),
                format!("{}s", device.last_seen.elapsed().as_secs()),
            ])
        })
        .collect();
    let table = Table::new(
        rows,
        [
            Constraint::Min(16),
            Constraint::Length(10),
            Constraint::Length(22),
            Constraint::Length(8),
        ],
    )
    .header(
        Row::new(vec!["名称", "ID", "地址", "最后出现"]).style(TuiStyle::new().add_modifier(Modifier::BOLD)),
    )
    .row_highlight_style(TuiStyle::new().add_modifier(Modifier::REVERSED))
    .highlight_symbol("▶ ");

    let mut device_state = TableState::default();
    if !state.devices().is_empty() && state.cursor() < state.devices().len() {
        device_state.select(Some(state.cursor()));
    }
    frame.render_stateful_widget(table, chunks[1], &mut device_state);

    // 待确认区：接受与否取决于是否已与对方核对过 6 位校验码，所以校验码必须在同一行可见
    if pending_rows == 0 {
        let hint = if decisions_allowed {
            "暂无待确认的接收请求。"
        } else {
            "暂无待确认的接收请求（需要后台服务才能接受/拒绝：glowlink on）。"
        };
        frame.render_widget(
            Paragraph::new(hint).block(Block::default().borders(Borders::ALL).title(" 待确认 ")),
            chunks[2],
        );
    } else {
        let rows: Vec<Row> = state
            .pending()
            .iter()
            .map(|item| {
                Row::new(vec![
                    format!("#{}", item.token),
                    item.peer.clone(),
                    item.name.clone(),
                    human_size(item.size),
                    item.code.clone(),
                ])
            })
            .collect();
        let table = Table::new(
            rows,
            [
                Constraint::Length(6),
                Constraint::Min(12),
                Constraint::Min(16),
                Constraint::Length(10),
                Constraint::Length(8),
            ],
        )
        .header(
            Row::new(vec!["编号", "来自", "文件", "大小", "校验码"])
                .style(TuiStyle::new().add_modifier(Modifier::BOLD)),
        )
        .row_highlight_style(TuiStyle::new().add_modifier(Modifier::REVERSED))
        .highlight_symbol("▶ ");
        let mut pending_state = TableState::default();
        if state.cursor() >= state.devices().len() {
            pending_state.select(Some(state.cursor() - state.devices().len()));
        }
        frame.render_stateful_widget(table, chunks[2], &mut pending_state);
    }

    // 底部：输入行 / 操作提示 / 上次操作反馈
    let hint: String = if state.input_active() {
        let target = match state.focus() {
            Focus::Device(device) => device.name.clone(),
            _ => String::new(),
        };
        format!(
            "发送到「{target}」，文件路径（可直接把文件拖进来）：{}",
            state.input()
        )
    } else if let Some((failed, text)) = status {
        if *failed {
            format!("[警告] {text}")
        } else {
            text.clone()
        }
    } else {
        match state.focus() {
            Focus::Pending(_) => {
                "待确认：Enter 接受 · d 拒绝 · ↑↓ 选择 · r 刷新 · q 退出（接受前请与对方核对校验码）"
                    .to_string()
            }
            _ if state.devices().is_empty() && pending_rows == 0 => {
                "正在搜索附近设备…（r 刷新 · q 退出）".to_string()
            }
            _ => "↑↓ 选择 · Enter 输入文件路径 · r 刷新 · q 退出".to_string(),
        }
    };
    let title = if state.input_active() { " 输入 " } else { " 操作 " };
    frame.render_widget(
        Paragraph::new(hint).block(Block::default().borders(Borders::ALL).title(title)),
        chunks[3],
    );
}
