//! `model/layout.rs` 的测试：节点几何、散布与命中判定。
//!
//! 这些是**渲染层唯一能测的部分**——GPUI 要真实窗口才画得出来，座标一旦写错
//! 只能靠肉眼发现。所以几何全部抽成纯函数，在这里钉死。

use glowlink_desktop::model::layout::{
    self, hits_center, place_devices, self_center, snap_to_device, NodeSize, Point, Rect,
    CENTER_CLEAR, MIN_DIST,
};
use glowlink_desktop::theme;

fn distance(a: Point, b: Point) -> f32 {
    ((a.x - b.x).powi(2) + (a.y - b.y).powi(2)).sqrt()
}

/// 造 `count` 台设备的一组 ID。
fn device_ids(count: usize) -> Vec<String> {
    (0..count).map(|index| format!("device-{index:02}")).collect()
}

#[test]
fn self_center_is_a_full_window_coordinate() {
    // 设计稿 CSS 里 `.node--self` 写的是 top:196px，那是「节点图内」；加上 44px 标题栏
    // 才是全窗坐标。这里只认全窗一套，杜绝二次换算。
    assert_eq!(self_center(), Point { x: 160.0, y: 240.0 });
}

#[test]
fn placements_do_not_depend_on_the_order_of_the_device_list() {
    // 发现层返回的顺序可能变（谁先应答），位置若跟着顺序走，界面会整体抽一下。
    let forward = place_devices(&["alpha", "beta", "gamma"]);
    let backward = place_devices(&["gamma", "beta", "alpha"]);

    assert_eq!(forward.centers[0], backward.centers[2]);
    assert_eq!(forward.centers[1], backward.centers[1]);
    assert_eq!(forward.centers[2], backward.centers[0]);
    assert_eq!(forward.phases[0], backward.phases[2]);
    assert_eq!(forward.phases[2], backward.phases[0]);
}

#[test]
fn same_device_is_always_placed_in_the_same_spot() {
    // 稳定：调用两次必须完全一致（否则每 300ms 的刷新都会让节点跳一下）
    let first = place_devices(&["alpha", "beta"]);
    let second = place_devices(&["alpha", "beta"]);
    assert_eq!(first, second);

    // 别的设备进出时，它最多被排斥推开一点点，不会跑到窗口另一头
    let alone = place_devices(&["beta"]);
    let crowded = place_devices(&["alpha", "beta", "gamma", "delta"]);
    let drift = distance(alone.centers[0], crowded.centers[1]);
    assert!(drift < MIN_DIST, "设备因他人进出被挪了 {drift:.1}px（上限 {MIN_DIST}）");
}

#[test]
fn scattered_nodes_are_not_frozen_on_one_orbit() {
    // 「神经元」不是一个固定半径的环：散点的远近也该不一样，
    // 否则一眼就看得出是拿模板摆出来的（半径全等 = 行星轨道）。
    let names = device_ids(4);
    let ids: Vec<&str> = names.iter().map(|s| s.as_str()).collect();
    let radii: Vec<f32> =
        place_devices(&ids).centers.iter().map(|c| distance(*c, self_center())).collect();
    let spread = radii.iter().cloned().fold(f32::MIN, f32::max)
        - radii.iter().cloned().fold(f32::MAX, f32::min);
    assert!(spread > 10.0, "四台的半径几乎一样（差 {spread:.1}px），看不出是散点");
}

#[test]
fn devices_never_overlap_the_center_or_each_other() {
    // 「神经元」要的是一圈散开的亮点：压在中心盘上或是互相叠着都不成立。
    // 从 1 台测到 10 台——数量越多排斥越难收敛，这是算法真正的压力点。
    for count in 1..=10 {
        let names = device_ids(count);
        let ids: Vec<&str> = names.iter().map(|s| s.as_str()).collect();
        let placement = place_devices(&ids);

        for (index, center) in placement.centers.iter().enumerate() {
            let to_center = distance(*center, self_center());
            assert!(
                to_center >= CENTER_CLEAR - 0.5,
                "{count} 台时第 {index} 台压到了中心盘：{to_center:.1}px"
            );
            // 盘（半径 26）+ 设备名都要留在窗口内
            assert!(center.x > 26.0 && center.x < theme::WINDOW_W - 26.0, "x 越界：{center:?}");
            assert!(center.y > theme::TITLEBAR_H + 26.0, "y 顶到标题栏：{center:?}");
            assert!(center.y < theme::WINDOW_H - 26.0, "y 越出窗口底：{center:?}");

            for other in placement.centers.iter().skip(index + 1) {
                let gap = distance(*center, *other);
                assert!(
                    gap >= MIN_DIST - 0.5,
                    "{count} 台时两枚节点叠在一起：{gap:.1}px < {MIN_DIST}"
                );
            }
        }
    }
}

#[test]
fn drift_phase_is_stable_in_range_and_spread_out() {
    let names = device_ids(6);
    let ids: Vec<&str> = names.iter().map(|s| s.as_str()).collect();
    let placement = place_devices(&ids);

    for phase in &placement.phases {
        assert!((0.0..1.0).contains(phase), "相位必须落在 0..1：{phase}");
    }
    // 错相是设计稿的硬要求（「所有运动都是缓慢、连续、错相的」）：
    // 同拍的话一圈节点像被同一根线扯着，顿时变机械舞。
    for (index, phase) in placement.phases.iter().enumerate() {
        for other in placement.phases.iter().skip(index + 1) {
            assert!((phase - other).abs() > 0.01, "两台设备同拍：{phase} / {other}");
        }
    }
    // 同一台设备的相位也必须稳定
    assert_eq!(placement.phases, place_devices(&ids).phases);
}

#[test]
fn rect_is_derived_from_the_center() {
    // GPUI 用左上角定位，而布局给的是圆心——换算只能有一处
    let rect = Rect::from_center(self_center(), NodeSize::SelfNode);
    assert_eq!(rect.size, 84.0);
    assert_eq!((rect.x, rect.y), (160.0 - 42.0, 240.0 - 42.0));

    let device = Rect::from_center(Point { x: 66.0, y: 86.0 }, NodeSize::Device);
    assert_eq!(device.size, 52.0);
    assert_eq!((device.x, device.y), (66.0 - 26.0, 86.0 - 26.0));
}

#[test]
fn device_label_sits_below_the_disc() {
    let rect = Rect::from_center(Point { x: 66.0, y: 86.0 }, NodeSize::Device);
    // 设计稿：标签 top: 58px（对 52px 的盘来说就是「盘底 + 6」）
    assert_eq!(rect.label_y(), rect.y + 52.0 + 6.0);
}

#[test]
fn center_hit_is_the_disc_itself() {
    // 命中区 = 中心盘本身（84px），不含 8px 的呼吸光圈：
    // 把光圈算进命中会让「看着没放上去」也生效，手感变糊。
    assert!(hits_center(Point { x: 160.0, y: 240.0 }), "正中心当然算命中");
    assert!(hits_center(Point { x: 160.0 + 41.0, y: 240.0 }), "盘内边缘算命中");
    assert!(!hits_center(Point { x: 160.0 + 43.0, y: 240.0 }), "盘外就落空");
    // 光圈区域（42..50）必须明确落空
    assert!(!hits_center(Point { x: 160.0 + 46.0, y: 240.0 }), "光圈不算命中区");
}

#[test]
fn snapping_picks_the_nearest_device_within_radius() {
    let centers = [Point { x: 66.0, y: 86.0 }, Point { x: 254.0, y: 86.0 }];
    let near_phone = Point { x: 76.0, y: 96.0 };
    assert_eq!(snap_to_device(near_phone, &centers, 26.0), Some(0));

    // 离谁都远 ⇒ 回弹（None），不是「就近选一个」
    assert_eq!(snap_to_device(Point { x: 160.0, y: 200.0 }, &centers, 26.0), None);

    // 一台设备都没有时也不该 panic
    assert_eq!(snap_to_device(near_phone, &[], 26.0), None);
}

#[test]
fn snapping_prefers_the_closer_one_when_two_are_in_range() {
    let centers = [
        Point { x: 100.0, y: 100.0 },
        Point { x: 120.0, y: 100.0 },
    ];
    // 落点离第二台更近，就该吸到第二台
    assert_eq!(snap_to_device(Point { x: 118.0, y: 100.0 }, &centers, 26.0), Some(1));
    // 完全重合时选较小下标（确定性，不随机跳）
    let same = [Point { x: 100.0, y: 100.0 }, Point { x: 100.0, y: 100.0 }];
    assert_eq!(snap_to_device(Point { x: 100.0, y: 100.0 }, &same, 26.0), Some(0));
}

#[test]
fn empty_device_list_places_nothing() {
    let placement = place_devices(&[]);
    assert!(placement.centers.is_empty());
    assert!(placement.phases.is_empty());
    // 顺手确认常量之间的关系没被改坏：净空必须让开「本机盘 + 设备盘 + 间隙」
    assert!(CENTER_CLEAR >= (theme::NODE_SELF_SIZE + theme::NODE_DEVICE_SIZE) / 2.0);
    assert_eq!(layout::MIN_DIST, theme::NODE_DEVICE_SIZE + 8.0);
}
