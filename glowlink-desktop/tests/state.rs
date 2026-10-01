//! `model/state.rs` 的测试：界面状态机。
//!
//! 这套状态机是纯逻辑（零 IO、零 GPUI），所以能单测——而它出错的样子极其隐蔽：
//! 设备早就拿到了，屏幕上却只有中心那枚盘，日志里一切正常。

use glowlink_desktop::model::state::{Phase, UiState};

#[test]
fn starts_in_searching_and_hides_device_nodes() {
    // 启动先「搜索中」：发现要几百毫秒，直接进发现态会闪一下空图
    let state = UiState::new();
    assert_eq!(state.phase(), Phase::Searching);
    assert!(!state.phase().shows_devices(), "搜索中不该画设备节点");
    assert!(state.phase().shows_searching());
}

#[test]
fn seeing_devices_enters_discover() {
    // **这就是踩过的那个 bug**：状态不跟着设备数走时，
    // 「有设备」也没用——`Searching` 下节点一律不画，屏幕上永远只有自机。
    let mut state = UiState::new();
    state.observe_devices(2);
    assert_eq!(state.phase(), Phase::Discover);
    assert!(state.phase().shows_devices());
    assert_eq!(state.device_count(), 2);
}

#[test]
fn all_devices_gone_goes_back_to_searching() {
    // 设备全部掉线要回到搜索态（设计稿的空态 = 自机 + spinner），
    // 否则空图会挂着上一次的残留。
    let mut state = UiState::new();
    state.observe_devices(1);
    state.observe_devices(0);
    assert_eq!(state.phase(), Phase::Searching);
    assert!(!state.phase().shows_devices());
}

#[test]
fn a_refresh_never_steals_transfer_or_pairing() {
    // 传输/配对态由用户动作驱动：设备表刷新（比如目标中途掉线）不许把它掳走
    let mut state = UiState::new();
    state.observe_devices(1);
    state.select(0);
    assert!(state.begin_transfer(), "选中了目标才能进传输态");
    state.observe_devices(1);
    assert_eq!(state.phase(), Phase::Transfer, "刷新不该改状态");
    state.observe_devices(0);
    assert_eq!(state.phase(), Phase::Transfer, "设备掉线也不该在传输中途改状态");
    assert!(state.phase().shows_wire(), "传输态才画连线");
}

#[test]
fn a_device_that_goes_offline_drops_the_selection() {
    // 选中的设备下线了，选中必须跟着失效，否则界面指着空气发文件
    let mut state = UiState::new();
    state.observe_devices(3);
    state.select(2);
    assert_eq!(state.selected(), Some(2));
    state.observe_devices(2);
    assert_eq!(state.selected(), None);
}

#[test]
fn transfer_and_pairing_need_a_target() {
    // 没有目标就没有传输 / 配对：这两种状态都不许在未选中时进入
    let mut state = UiState::new();
    state.observe_devices(1);
    assert!(!state.begin_transfer());
    assert!(!state.begin_pairing());
    assert_eq!(state.phase(), Phase::Discover);

    state.select(0);
    assert!(state.begin_pairing());
    state.end_pairing();
    assert_eq!(state.phase(), Phase::Discover, "配对结束回到发现态");
}

#[test]
fn the_empty_note_reports_then_retires() {
    use glowlink_desktop::model::state::{
        empty_state, EmptyNote, ServiceState, RESULT_NOTE_HOLD, SEARCH_PATIENCE,
    };
    use std::time::Duration;
    let on = ServiceState::On;

    // 刚开：正在搜 + **转**
    let (note, spinning) = empty_state(on, 0, Duration::from_secs(0)).expect("应显示");
    assert_eq!(note, EmptyNote::Searching);
    assert!(spinning, "这时候转圈是对的：确实在做事");

    // 搜满耐心：出结论，**停转**
    let (note, spinning) =
        empty_state(on, 0, SEARCH_PATIENCE).expect("应显示结论");
    assert_eq!(note, EmptyNote::NoDevices);
    assert!(!spinning, "静止的结论不该还在转");

    // 结论留一小会儿之后**收起**：常驻说明在拖拽区，中央别一直唠叨
    assert_eq!(
        empty_state(on, 0, SEARCH_PATIENCE + RESULT_NOTE_HOLD),
        None
    );
    assert_eq!(empty_state(on, 0, Duration::from_secs(600)), None);

    // 有设备：图上就有东西，不需要提示挡在中间
    assert_eq!(empty_state(on, 3, Duration::from_secs(0)), None);
}

#[test]
fn the_empty_note_tells_the_truth_about_the_service() {
    use glowlink_desktop::model::state::{
        empty_state, EmptyNote, ServiceState, SERVICE_NOTE_HOLD,
    };
    use std::time::Duration;
    // 服务关着还说「正在搜索附近设备」是假话——没有任何东西在搜
    let (note, spinning) =
        empty_state(ServiceState::Off, 0, Duration::from_secs(1)).expect("应显示");
    assert_eq!(note, EmptyNote::ServiceOff);
    assert!(!spinning, "没在搜就不许转圈");
    // 「服务已关闭」也是**过渡报告**：看一眼就够，开关就在上面
    assert_eq!(
        empty_state(ServiceState::Off, 0, SERVICE_NOTE_HOLD),
        None
    );
    // 拉起服务那几百毫秒：说「正在启动」，不许说「已关闭」
    let (note, spinning) =
        empty_state(ServiceState::Starting, 0, Duration::from_secs(0)).expect("应显示");
    assert_eq!(note, EmptyNote::Starting);
    assert!(!spinning);
}

