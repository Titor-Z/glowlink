//! 交互式设备列表（ratatui）。
//!
//! 依赖边界：**只有这个模块和 CLI 命令层用 TUI 库**，核心协议层（帧 / 发现 / 传输 / 加密）
//! 依然是零依赖（见 AGENTS.md「信息暴露与输出规范」的依赖政策）。
//!
//! 职责边界：TUI 只负责「选设备 + 收一个文件路径」，**传输本身仍走原来的 CLI 流程**——
//! 选择完成后退出界面、恢复终端，再执行普通发送。这样不必在 TUI 里重写一遍进度渲染，
//! 也让「TUI 崩溃不会牵连传输」。
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

use glowlink_core::model::error::Result;
use glowlink_core::Device;

/// 界面刷新间隔（发现线程在后台跑，这里只重绘）。
const TICK: Duration = Duration::from_millis(200);

/// 用户退出界面时要交给 CLI 去做的动作。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TuiAction {
    /// 直接退出，什么也不做。
    Quit,
    /// 选中了设备并给了文件路径。
    Send { addr: std::net::SocketAddr, name: String, path: PathBuf },
}

/// 界面用到的静态信息（本机身份与后台服务状态）。
pub struct Header {
    pub version: String,
    pub local_name: String,
    pub local_id: String,
    /// `(PID, 待确认数)`；后台服务没跑时为 `None`。
    pub daemon: Option<(u32, usize)>,
}

/// 底部输入行的状态。
#[derive(Default)]
struct Input {
    text: String,
    /// 是否处于「输入文件路径」模式（回车进入）。
    active: bool,
}

/// 界面状态。**这部分是纯逻辑，可单测**（渲染与事件循环在外面）。
pub struct DeviceList {
    devices: Vec<Device>,
    selected: usize,
    /// 上一次选中的设备 ID：刷新后尽量保持选中同一台，而不是跳回第一行。
    remembered: Option<String>,
    input: Input,
}

impl Default for DeviceList {
    fn default() -> Self {
        Self::new()
    }
}

impl DeviceList {
    pub fn new() -> Self {
        DeviceList {
            devices: Vec::new(),
            selected: 0,
            remembered: None,
            input: Input::default(),
        }
    }

    /// 用新发现到的设备列表刷新（保持选中项）。
    pub fn set_devices(&mut self, devices: Vec<Device>) {
        // 优先按上次选中的 ID 找回同一台设备
        let anchored = self
            .remembered
            .as_deref()
            .and_then(|id| devices.iter().position(|d| d.id.as_str() == id));
        self.devices = devices;
        self.selected = match anchored {
            Some(index) => index,
            // 设备消失了：把下标夹回有效范围，而不是让选中项凭空跳走
            None => self.selected.min(self.devices.len().saturating_sub(1)),
        };
        self.remembered = self.selected().map(|d| d.id.as_str().to_string());
    }

    pub fn devices(&self) -> &[Device] {
        &self.devices
    }

    pub fn selected_index(&self) -> usize {
        self.selected
    }

    pub fn selected(&self) -> Option<&Device> {
        self.devices.get(self.selected)
    }

    /// 下移一格；到底就停住（不循环，免得手一抖从第一台跳到最后一台）。
    pub fn select_next(&mut self) {
        if self.selected + 1 < self.devices.len() {
            self.selected += 1;
        }
        self.remembered = self.selected().map(|d| d.id.as_str().to_string());
    }

    pub fn select_previous(&mut self) {
        self.selected = self.selected.saturating_sub(1);
        self.remembered = self.selected().map(|d| d.id.as_str().to_string());
    }

    pub fn input_active(&self) -> bool {
        self.input.active
    }

    pub fn input(&self) -> &str {
        &self.input.text
    }

    /// 进入输入模式（回车）。
    pub fn begin_input(&mut self) -> bool {
        if self.selected().is_none() {
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

    /// 确认输入：路径为空或设备已消失都返回 `None`（由调用方决定提示什么）。
    pub fn confirm_input(&mut self) -> Option<TuiAction> {
        if !self.input.active {
            return None;
        }
        let path = parse_path_input(&self.input.text);
        let device = self.selected()?.clone();
        if path.as_os_str().is_empty() {
            return None;
        }
        self.input.active = false;
        self.input.text.clear();
        Some(TuiAction::Send { addr: device.addr, name: device.name, path })
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

/// 启动界面并跑到用户做出选择。`snapshot` 由后台发现线程持续更新。
pub fn run(snapshot: Arc<Mutex<Vec<Device>>>, header: Header) -> Result<TuiAction> {
    let mut terminal = ratatui::init();
    let mut list = DeviceList::new();
    let result = event_loop(&mut terminal, &mut list, &snapshot, &header);
    // 无论正常退出还是出错都要还原终端，否则用户的 shell 会留在备用屏里
    ratatui::restore();
    result
}

fn event_loop(
    terminal: &mut ratatui::DefaultTerminal,
    list: &mut DeviceList,
    snapshot: &Arc<Mutex<Vec<Device>>>,
    header: &Header,
) -> Result<TuiAction> {
    let mut last_refresh = Instant::now();
    loop {
        // 定期把后台发现线程的结果搬进来（发现线程的 UDP 收包会阻塞，不能放 UI 线程）
        if last_refresh.elapsed() >= TICK {
            list.set_devices(snapshot.lock().unwrap_or_else(|e| e.into_inner()).clone());
            last_refresh = Instant::now();
        }

        terminal
            .draw(|frame| draw(frame, list, header))
            .map_err(|e| glowlink_core::GlowError::Io(e))?;

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

        // 按键处理只有一处实现（apply_key），界面与测试共用同一套语义。
        // 刷新闭包只负责「取一份最新设备」，不碰 list，避免与 apply_key 争可变借用
        let mut refresh = || snapshot.lock().unwrap_or_else(|e| e.into_inner()).clone();
        if let Some(action) = apply_key(list, key, &mut refresh) {
            return Ok(action);
        }
    }
}

/// 绘制界面。
fn draw(frame: &mut Frame, list: &DeviceList, header: &Header) {
    let area = frame.area();
    // 顶部 4 行 = 上下边框 + 2 行内容（身份一行、后台服务状态一行）；
    // 写成 3 行会把第二行内容裁掉（界面上看起来像「状态没显示」）
    let chunks = Layout::vertical([Constraint::Length(4), Constraint::Min(3), Constraint::Length(3)])
        .split(area);

    // 顶部：本机身份 + 后台服务状态
    let daemon = match &header.daemon {
        None => "后台服务：未启动（glowlink on 可常驻）".to_string(),
        Some((pid, 0)) => format!("后台服务：运行中（PID {pid}）"),
        Some((pid, pending)) => format!("后台服务：运行中（PID {pid}，待确认 {pending}）"),
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
    let rows: Vec<Row> = list
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
    .header(Row::new(vec!["名称", "ID", "地址", "最后出现"]).style(TuiStyle::new().add_modifier(Modifier::BOLD)))
    .row_highlight_style(TuiStyle::new().add_modifier(Modifier::REVERSED))
    .highlight_symbol("▶ ");

    let mut state = TableState::default();
    if !list.devices().is_empty() {
        state.select(Some(list.selected_index()));
    }
    frame.render_stateful_widget(table, chunks[1], &mut state);

    // 底部：输入行 / 操作提示
    let bottom = if list.input_active() {
        let target = list.selected().map(|d| d.name.clone()).unwrap_or_default();
        Paragraph::new(format!("发送到「{target}」，文件路径（可直接把文件拖进来）：{}", list.input()))
            .block(Block::default().borders(Borders::ALL).title(" 输入 "))
    } else {
        let hint = if list.devices().is_empty() {
            "正在搜索附近设备…（r 刷新 · q 退出）"
        } else {
            "↑↓ 选择 · Enter 输入文件路径 · r 刷新 · q 退出"
        };
        Paragraph::new(hint).block(Block::default().borders(Borders::ALL))
    };
    frame.render_widget(bottom, chunks[2]);
}

/// 把一个按键喂给状态机，返回需要执行的动作。
///
/// 界面事件循环与单元测试都走这一个函数——按键语义只有一处实现，
/// 不会出现「界面上能用、测试里不是同一套」的漂移。
pub fn apply_key(
    list: &mut DeviceList,
    key: KeyEvent,
    refresh: &mut dyn FnMut() -> Vec<Device>,
) -> Option<TuiAction> {
    if list.input_active() {
        match key.code {
            KeyCode::Esc => list.cancel_input(),
            KeyCode::Backspace => list.pop_char(),
            KeyCode::Enter => match list.confirm_input() {
                // 路径为空就当作取消，别把空路径当文件去发
                Some(action) => return Some(action),
                None => list.cancel_input(),
            },
            KeyCode::Char(ch) => list.push_char(ch),
            _ => {}
        }
        return None;
    }
    match key.code {
        KeyCode::Char('q') | KeyCode::Esc => return Some(TuiAction::Quit),
        KeyCode::Down | KeyCode::Char('j') => list.select_next(),
        KeyCode::Up | KeyCode::Char('k') => list.select_previous(),
        KeyCode::Enter => {
            list.begin_input();
        }
        KeyCode::Char('r') => list.set_devices(refresh()),
        _ => {}
    }
    None
}
