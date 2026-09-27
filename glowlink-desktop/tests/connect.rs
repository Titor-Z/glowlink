//! `model/connect.rs` 的测试：拖到一起之后的连接态。
//!
//! 位置规则全在纯函数里，所以能钉住——渲染层（缩放、连线）测不动。

use glowlink_desktop::model::connect::{
    angle_of, connected_radius, min_angle_gap, point_at, ring_radius, separate, Connection,
    CONNECTED_SCALE,
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
fn dragging_from_the_upper_right_lands_upper_right() {
    // 方向 = 拖来的方向：你往右上拖，它就停在右上（不是随机丢一颗）
    let center = layout::self_center();
    let upper_right = Point { x: center.x + 60.0, y: center.y - 60.0 };
    let angle = angle_of(center, upper_right);
    let landed = point_at(center, angle);
    assert!(landed.x > center.x, "应在右半边：{landed:?}");
    assert!(landed.y < center.y, "应在上半边：{landed:?}");
    assert!((distance(landed, center) - ring_radius()).abs() < 0.001, "距离必须是环半径");
    // 斜 45°：落点相对中心的 dx/dy 应大致相等
    assert!((landed.x - center.x - (center.y - landed.y)).abs() < 1.0);
}

#[test]
fn a_drop_exactly_on_the_center_still_gets_a_direction() {
    // 指针正好压在中心上：方向无从可选，但总得给一个（正上方）
    let center = layout::self_center();
    let angle = angle_of(center, center);
    let landed = point_at(center, angle);
    assert!((landed.x - center.x).abs() < 0.001, "正上方：x 不变");
    assert!(landed.y < center.y, "在上方");
}

#[test]
fn the_minimum_angle_really_fits_two_shrunk_discs() {
    // 拍脑袋定 40° 时：环半径 57px 上的弦长只有 40px，而两个缩小盘要 42.7px —— 会叠。
    // 所以这个间隔必须**从几何算**，测试就是拿弦长去核对。
    let chord = 2.0 * ring_radius() * (min_angle_gap() / 2.0).sin();
    let need = 2.0 * connected_radius() + glowlink_desktop::model::layout::GAP;
    assert!(chord >= need, "弦长 {chord:.1}px 放不下两个盘（需要 {need:.1}px）");
}

#[test]
fn cables_never_stack_on_one_side() {
    // 全都拖到右下时不能叠成一坨：推开到最小间隔
    let crowded = vec![0.2, 0.25, 0.3];
    let spread = separate(&crowded);
    assert_eq!(spread.len(), 3);
    for pair in spread.windows(2) {
        assert!(pair[1] - pair[0] >= min_angle_gap() - 0.001, "两台仍挤在一起：{pair:?}");
    }
    // 推开之后整体不能跑偏：均值应当基本不变（否则「拖哪儿停哪儿」的记忆就没了）
    let before: f32 = crowded.iter().sum::<f32>() / 3.0;
    let after: f32 = spread.iter().sum::<f32>() / 3.0;
    assert!((before - after).abs() < 0.5, "整体被转走了：{before} → {after}");
}

#[test]
fn well_separated_cables_are_left_alone() {
    // 已经够远的不许动：用户记住的方位不能被「优化」掉
    let nice = vec![-1.5, 0.0, 1.5];
    assert_eq!(separate(&nice), nice);
}

#[test]
fn every_connection_gets_its_own_ring_point() {
    let center = layout::self_center();
    let connections = vec![
        Connection { id: "a".into(), angle: -1.5 },
        Connection { id: "b".into(), angle: 0.0 },
        Connection { id: "c".into(), angle: 1.5 },
    ];
    let pinned = glowlink_desktop::model::connect::pinned_points(&connections);
    assert_eq!(pinned.len(), 3);
    for (connection, pin) in connections.iter().zip(pinned.iter()) {
        let expected = point_at(center, connection.angle);
        assert!((pin.point.x - expected.x).abs() < 0.001);
        assert!((pin.radius - connected_radius()).abs() < 0.001, "钉子要带缩小后的半径");
        // 环上的点离中心就是一个环半径，且离本机盘够近才叫「拼合」
        assert!(distance(pin.point, center) < 92.0, "落在中心净空之外就不是拼合了");
    }
}
