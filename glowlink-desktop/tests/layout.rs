//! `model/layout.rs` 的测试：节点几何与命中判定。
//!
//! 这些是**渲染层唯一能测的部分**——GPUI 要真实窗口才画得出来，座标一旦写错
//! 只能靠肉眼发现。所以几何全部抽成纯函数，在这里钉死。

use glowlink_desktop::model::layout::{
    self, device_center, hits_center, self_center, snap_to_device, NodeSize, Point, Rect,
};
use glowlink_desktop::theme;

#[test]
fn node_centers_are_full_window_coordinates() {
    // 设计稿 CLAUDE 契约：四个圆心写死在全窗坐标上，不是「节点图内 +44」。
    // CSS 里 .node--phone 写的是 top:42px，那是图内坐标；加上 44px 标题栏才是这里。
    assert_eq!(self_center(), Point { x: 160.0, y: 240.0 });
    assert_eq!(device_center(0), Point { x: 66.0, y: 86.0 }, "手机槽");
    assert_eq!(device_center(1), Point { x: 254.0, y: 86.0 }, "平板槽");
    assert_eq!(device_center(2), Point { x: 160.0, y: 364.0 }, "台式机槽");
}

#[test]
fn extra_devices_share_the_last_slot_and_never_wrap_around() {
    // 第 4 台以后叠在最后一槽上（界面用「+N」提示），**不回绕到第一个槽**：
    // 回绕会让第 4 台与第 1 台视觉上互相顶掉，看起来像设备丢了。
    assert_eq!(device_center(3), device_center(2));
    assert_eq!(device_center(9), device_center(2));
}

#[test]
fn all_centers_stay_inside_the_window() {
    // 窗口锁死 320×500，任何圆心都不许跑到外面去
    let mut centers = vec![self_center()];
    for index in 0..4 {
        centers.push(device_center(index));
    }
    for point in centers {
        assert!(
            point.x > 0.0 && point.x < theme::WINDOW_W,
            "圆心 x 越界：{point:?}"
        );
        assert!(
            point.y > theme::TITLEBAR_H && point.y < theme::WINDOW_H,
            "圆心 y 必须在标题栏之下、窗口之内：{point:?}"
        );
    }
}

#[test]
fn rect_is_derived_from_the_center() {
    // GPUI 用左上角定位，而设计稿给的是圆心——换算只能有一处
    let rect = Rect::from_center(self_center(), NodeSize::SelfNode);
    assert_eq!(rect.size, 84.0);
    assert_eq!((rect.x, rect.y), (160.0 - 42.0, 240.0 - 42.0));

    let device = Rect::from_center(device_center(0), NodeSize::Device);
    assert_eq!(device.size, 52.0);
    assert_eq!((device.x, device.y), (66.0 - 26.0, 86.0 - 26.0));
}

#[test]
fn device_label_sits_below_the_disc() {
    let rect = Rect::from_center(device_center(0), NodeSize::Device);
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
    let phone = device_center(0);
    let near_phone = Point { x: phone.x + 10.0, y: phone.y + 10.0 };
    assert_eq!(snap_to_device(near_phone, 3, 26.0), Some(0));

    // 离谁都远 ⇒ 回弹（None），不是「就近选一个」
    assert_eq!(snap_to_device(Point { x: 160.0, y: 200.0 }, 3, 26.0), None);

    // 一台设备都没有时也不该 panic
    assert_eq!(snap_to_device(near_phone, 0, 26.0), None);
}

#[test]
fn snapping_prefers_the_closer_one_when_two_overlap() {
    // 第 4、5 台叠在同一槽位：命中时选**较小的下标**（稳定，不随机跳）
    let overlapping = device_center(2);
    assert_eq!(snap_to_device(overlapping, 5, 26.0), Some(2));
}

#[test]
fn slot_count_matches_the_designed_layout() {
    // 设计稿只画了三台设备；增删槽位等于改设计，必须在此显式确认
    assert_eq!(layout::SLOTS.len(), 3);
}
