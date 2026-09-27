//! `model/connect.rs` 的测试：拖到一起之后的连接态。
//!
//! 位置规则全在纯函数里，所以能钉住——渲染层（缩放、连线）测不动。

use glowlink_desktop::model::connect::{
    connected_radius, max_slots, min_angle_gap, pinned_points, point_at, ring_radius, slot_angles,
    Connection, CONNECTED_SCALE, FIRST_SLOT,
};
use glowlink_desktop::model::layout::{self, Point};

fn distance(a: Point, b: Point) -> f32 {
    ((a.x - b.x).powi(2) + (a.y - b.y).powi(2)).sqrt()
}

#[test]
fn the_ring_is_exactly_the_two_shrunk_discs_plus_a_dozen_pixels() {
    // 「拼合」的手感就在这个数上：缝大了像吊在外面，缝小了会挤在一起
    let expected = 84.0 / 2.0 * CONNECTED_SCALE + connected_radius() + 12.0;
    assert!((ring_radius() - expected).abs() < 0.001);
    assert!((connected_radius() - 52.0 / 2.0 * CONNECTED_SCALE).abs() < 0.001);
}

#[test]
fn the_first_device_always_lands_on_top() {
    // 槽位由中心点接管：第 1 台固定在正上方，**不看松手方向**。
    // 这样「我的 iPad 在哪边」是能记住的（以前按松手方向摆，每次拖都可能换位置）
    let center = layout::self_center();
    let slots = slot_angles(1);
    assert_eq!(slots.len(), 1);
    assert_eq!(slots[0], FIRST_SLOT);
    let landed = point_at(center, slots[0]);
    assert!((landed.x - center.x).abs() < 0.001, "正上方：x 与圆心相同");
    assert!(landed.y < center.y, "在上半边");
    assert!((distance(landed, center) - ring_radius()).abs() < 0.001, "距离必须是环半径");
}

#[test]
fn slots_go_clockwise_and_stay_evenly_spread() {
    let slots = slot_angles(4);
    assert_eq!(slots.len(), 4);
    // 顺时针：屏幕坐标 y 向下，角度递增就是顺时针
    for pair in slots.windows(2) {
        assert!(pair[1] > pair[0], "槽位必须单调递增：{pair:?}");
    }
    // 均匀：相邻间隔全等于 2π/n
    let step = std::f32::consts::TAU / 4.0;
    for pair in slots.windows(2) {
        assert!((pair[1] - pair[0] - step).abs() < 0.001, "间隔不均：{pair:?}");
    }
}

#[test]
fn every_slot_really_fits_a_shrunk_disc() {
    // 拍脑袋定 40° 时：环半径 57px 上的弦长只有 40px，而两个缩小盘要 42.7px —— 会叠。
    // 所以间隔必须**从几何算**，并且相邻槽位的弦长真的放得下两个盘。
    let chord = 2.0 * ring_radius() * (min_angle_gap() / 2.0).sin();
    let need = 2.0 * connected_radius() + layout::GAP;
    assert!(chord >= need, "弦长 {chord:.1}px 放不下两个盘（需要 {need:.1}px）");

    let limit = max_slots();
    assert!(limit >= 5, "按几何算至少该放得下 5 台：{limit}");
    for count in 1..=limit {
        let step = std::f32::consts::TAU / count as f32;
        assert!(step >= min_angle_gap() - 0.001, "{count} 台时相邻槽位挤到一起了");
    }
}

#[test]
fn more_devices_than_slots_are_still_ring_shaped() {
    // 超过几何上限只能硬塞：位置仍在环上（重叠交给物理推开，不在这里编造规则）
    let count = max_slots() + 3;
    let slots = slot_angles(count);
    assert_eq!(slots.len(), count);
    let center = layout::self_center();
    for angle in slots {
        let point = point_at(center, angle);
        assert!((distance(point, center) - ring_radius()).abs() < 0.001);
    }
}

#[test]
fn disconnecting_compacts_the_remaining_slots() {
    // 断开一台之后剩下的重新压实：2 台时是「正上 + 正下」，3 台时是 120° 三等分。
    // 位置完全确定 —— 这就是「中心点接管排布」的意思。
    let two = slot_angles(2);
    assert_eq!(two[0], FIRST_SLOT);
    assert!((two[1] - (FIRST_SLOT + std::f32::consts::PI)).abs() < 0.001, "第二台在正下方");
    let three = slot_angles(3);
    let step = std::f32::consts::TAU / 3.0;
    assert!((three[1] - three[0] - step).abs() < 0.001);
    assert!((three[2] - three[1] - step).abs() < 0.001);
}

#[test]
fn every_connection_gets_its_own_ring_point() {
    let center = layout::self_center();
    let connections: Vec<Connection> = ["a", "b", "c"]
        .iter()
        .zip(slot_angles(3))
        .map(|(id, angle)| Connection { id: (*id).to_string(), angle })
        .collect();
    let pinned = pinned_points(&connections);
    assert_eq!(pinned.len(), 3);
    for (connection, pin) in connections.iter().zip(pinned.iter()) {
        let expected = point_at(center, connection.angle);
        assert!((pin.point.x - expected.x).abs() < 0.001);
        assert!((pin.radius - connected_radius()).abs() < 0.001, "钉子要带缩小后的半径");
        // 环上的点离中心就是一个环半径，且离本机盘够近才叫「拼合」
        assert!(distance(pin.point, center) < 92.0, "落在中心净空之外就不是拼合了");
    }
}
