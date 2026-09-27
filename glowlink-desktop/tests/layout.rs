//! `model/layout.rs` 的测试：节点几何、散布与命中判定。
//!
//! 这些是**渲染层唯一能测的部分**——GPUI 要真实窗口才画得出来，座标一旦写错
//! 只能靠肉眼发现。所以几何全部抽成纯函数，在这里钉死。

use glowlink_desktop::model::layout::{
    self, place_devices, place_devices_free, resolve_contacts, self_center, snap_to_device,
    snaps_to_center,
    NodeSize, Pinned, Point, Rect, Seed, CENTER_CLEAR, DEVICE_RADIUS, GAP, MIN_DIST,
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
    let forward = place_devices_free(&["alpha", "beta", "gamma"]);
    let backward = place_devices_free(&["gamma", "beta", "alpha"]);

    assert_eq!(forward.centers[0], backward.centers[2]);
    assert_eq!(forward.centers[1], backward.centers[1]);
    assert_eq!(forward.centers[2], backward.centers[0]);
    assert_eq!(forward.phases[0], backward.phases[2]);
    assert_eq!(forward.phases[2], backward.phases[0]);
}

#[test]
fn same_device_is_always_placed_in_the_same_spot() {
    // 稳定：调用两次必须完全一致（否则每 300ms 的刷新都会让节点跳一下）
    let first = place_devices_free(&["alpha", "beta"]);
    let second = place_devices_free(&["alpha", "beta"]);
    assert_eq!(first, second);

    // 别的设备进出时，它最多被排斥推开一点点，不会跑到窗口另一头
    let alone = place_devices_free(&["beta"]);
    let crowded = place_devices_free(&["alpha", "beta", "gamma", "delta"]);
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
        place_devices_free(&ids).centers.iter().map(|c| distance(*c, self_center())).collect();
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
        let placement = place_devices_free(&ids);

        for (index, center) in placement.centers.iter().enumerate() {
            let to_center = distance(*center, self_center());
            assert!(
                to_center >= CENTER_CLEAR - 0.5,
                "{count} 台时第 {index} 台压到了中心盘：{to_center:.1}px"
            );
            // 盘 + 设备名 + 漂浮余量都要留在**节点图**里（不是「留在窗口里」就够）：
            // 伸进拖拽区会被那一层盖住（用户真机看到过）
            let half = theme::NODE_DEVICE_SIZE / 2.0;
            assert!(center.x - 44.0 > 0.0, "名字左边出窗：{center:?}");
            assert!(center.x + 44.0 < theme::WINDOW_W, "名字右边出窗：{center:?}");
            assert!(
                center.y - half - 4.0 > theme::TITLEBAR_H,
                "盘顶进标题栏：{center:?}"
            );
            assert!(
                center.y + half + 18.0 + 4.0 < theme::TITLEBAR_H + theme::GRAPH_H,
                "盘或名字伸进拖拽区：{center:?}"
            );

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
    let placement = place_devices_free(&ids);

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
    assert_eq!(placement.phases, place_devices_free(&ids).phases);
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
fn snapping_zone_is_fifty_pixels_from_the_center() {
    // 任务书 v1.0：圆心距 < 50px 才算吸附成功（设计稿的「84px 盘」是直径，
    // 换算成半径 42——多给的这 8px 是给手抖留的）
    assert!(snaps_to_center(Point { x: 160.0, y: 240.0 }), "正中心当然算");
    assert!(snaps_to_center(Point { x: 160.0 + 49.0, y: 240.0 }), "49px 内算吸附");
    assert!(snaps_to_center(Point { x: 160.0 + 42.0, y: 240.0 }), "盘缘仍算吸附");
    assert!(!snaps_to_center(Point { x: 160.0 + 51.0, y: 240.0 }), "51px 就落空");
    assert!(!snaps_to_center(Point { x: 160.0 + 46.0, y: 240.0 + 46.0 }), "斜向 65px 更落空");
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
    let placement = place_devices_free(&[]);
    assert!(placement.centers.is_empty());
    assert!(placement.phases.is_empty());
    // 顺手确认常量之间的关系没被改坏：净空必须让开「本机盘 + 设备盘 + 间隙」
    assert!(CENTER_CLEAR >= (theme::NODE_SELF_SIZE + theme::NODE_DEVICE_SIZE) / 2.0);
    assert_eq!(layout::MIN_DIST, theme::NODE_DEVICE_SIZE + 8.0);
}

#[test]
fn a_pinned_node_keeps_its_exact_spot_and_others_move_out_of_its_way() {
    // 拖到中心连上的那台会被「钉住」：位置由吸附规则定，断开也不回原位。
    // 所以它必须**精确**留在那个点，而其他节点绕着它排——而不是把它挤走。
    let pinned = Pinned { point: Point { x: 220.0, y: 300.0 }, radius: 17.0 };
    let names = device_ids(6);
    let ids: Vec<&str> = names.iter().map(|s| s.as_str()).collect();
    let placement = place_devices(&ids, &[pinned], &[]);

    for center in &placement.centers {
        let gap = distance(*center, pinned.point);
        // 已连接的盘变小了（半径 17），所以间距也按小盘算，不该再要求 60
        assert!(
            gap >= DEVICE_RADIUS + pinned.radius + GAP - 0.5,
            "活节点压在钉住的节点上：{gap:.1}px"
        );
    }
    // 钉子不受影响：它不在返回的 centers 里（它自己那台由界面直接摆）
    assert_eq!(placement.centers.len(), ids.len());
}

#[test]
fn pinned_nodes_do_not_drag_the_floating_ones_across_the_window() {
    // 钉子只影响附近：远处的活节点不该被它拽过去
    let names = device_ids(3);
    let ids: Vec<&str> = names.iter().map(|s| s.as_str()).collect();
    let free = place_devices_free(&ids);
    let pinned = Pinned { point: Point { x: 60.0, y: 100.0 }, radius: 17.0 };
    let with_pin = place_devices(&ids, &[pinned], &[]);
    for (a, b) in free.centers.iter().zip(with_pin.centers.iter()) {
        assert!(distance(*a, *b) < MIN_DIST, "位置被钉子拽动了：{a:?} → {b:?}");
    }
}

#[test]
fn a_seeded_node_starts_where_it_is_but_is_still_pushed_out_of_illegal_spots() {
    // 断开连接后就地留下的那台：从当前点出发，但**不能**停在压住中心盘的位置上。
    // 真机上的表现就是「恢复大小后图标跑到中心盘下面去了」——盘变回 26px 半径，
    // 而环上的点离中心只有 57px，比 42+26 小。
    let ids = ["alpha"];
    let too_close = Point { x: 160.0 + 57.0, y: 240.0 };
    let seeds = [Seed { id: "alpha".into(), point: too_close }];
    let placement = place_devices(&ids, &[], &seeds);
    let center = self_center();
    let gap = distance(placement.centers[0], center);
    assert!(gap >= CENTER_CLEAR - 0.5, "被排斥推到合法距离：{gap:.1}px");
    // 方向应当保留（沿原方向推出去，而不是换个方向）
    assert!(placement.centers[0].x > center.x, "仍在右侧：{:?}", placement.centers[0]);
}

#[test]
fn a_seeded_node_keeps_its_spot_when_that_spot_is_legal() {
    // 「位置就在那儿了」：合法就一点都别动
    let ids = ["alpha"];
    let home = Point { x: 60.0, y: 120.0 };
    let seeds = [Seed { id: "alpha".into(), point: home }];
    let placement = place_devices(&ids, &[], &seeds);
    assert_eq!(placement.centers[0], home);
}

#[test]
fn contact_only_pushes_what_is_actually_touching() {
    // 用户真机的抱怨：「都没遇到就碰」。所以推开量必须**恰好是重叠量**——
    // 相切（圆心距 = 半径和）时一点都不许动，再近 1px 才开始推。
    // 一次只放一枚节点：多枚一起放时几何互相牵扯，测出来的东西说不清。
    let radius = DEVICE_RADIUS;
    let obstacle = (Point { x: 100.0, y: 100.0 }, radius);
    let along = |distance: f32| Point { x: 100.0 + distance, y: 100.0 };

    // 1) 没碰到：一点都不许动
    let far = vec![along(2.0 * radius + 40.0)];
    assert_eq!(resolve_contacts(&far, radius, &[obstacle]), far, "没碰到的绝不许动");

    // 2) 正好相切：也不算碰
    let tangent = vec![along(2.0 * radius)];
    assert_eq!(resolve_contacts(&tangent, radius, &[obstacle]), tangent, "相切不算碰");

    // 3) 重叠 10px：推到恰好相切（不多推）
    let overlapping = vec![along(2.0 * radius - 10.0)];
    let after = resolve_contacts(&overlapping, radius, &[obstacle]);
    let gap = distance(after[0], obstacle.0);
    assert!((gap - 2.0 * radius).abs() < 0.01, "推开后应恰好相切：{gap:.2}px");
    assert!(after[0].x > overlapping[0].x, "往远离障碍物的方向推");
}

#[test]
fn a_pushed_node_passes_it_on_to_its_neighbour() {
    // 被推的又挤到别人：链条要传下去（但只传接触链，不是全场重排）
    let radius = DEVICE_RADIUS;
    let obstacle = (Point { x: 100.0, y: 100.0 }, radius);
    let rest = vec![
        Point { x: 100.0 + radius + 4.0, y: 100.0 },  // 被障碍物压到
        Point { x: 100.0 + 3.0 * radius + 4.0, y: 100.0 }, // 被它挤到
        Point { x: 260.0, y: 380.0 },                  // 远处的旁观者
    ];
    let after = resolve_contacts(&rest, radius, &[obstacle]);
    assert_eq!(after[2], rest[2], "远处的旁观者不该动");
    assert!(distance(after[0], after[1]) >= 2.0 * radius - 0.5, "被推的两枚不该叠在一起");
}

#[test]
fn contact_resolution_is_deterministic() {
    // 同一输入必须同一输出：否则每帧重算会自己抖起来
    let radius = DEVICE_RADIUS;
    let obstacle = (Point { x: 120.0, y: 90.0 }, radius);
    let rest = vec![Point { x: 130.0, y: 100.0 }, Point { x: 160.0, y: 96.0 }];
    assert_eq!(
        resolve_contacts(&rest, radius, &[obstacle]),
        resolve_contacts(&rest, radius, &[obstacle])
    );
}
