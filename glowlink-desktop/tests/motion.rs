//! `model/motion.rs`：全场运动状态（意图 → 弹簧 → 接触约束 → 只读位置）。
//!
//! 这里钉住的都是用户真机上出过问题的行为，不是「跑通就行」。

use glowlink_desktop::model::layout::{self, Point};
use glowlink_desktop::model::motion::{rubber_clamp, Ghost, Motion, RUBBER_LIMIT};

const DT: f32 = 1.0 / 60.0;

fn run(motion: &mut Motion, seconds: f32, ghost: Option<&Ghost>) {
    let mut left = seconds;
    while left > 0.0 {
        motion.step(DT, ghost);
        left -= DT;
    }
}

fn body(id: &str, point: Point) -> (String, Point, f32) {
    (id.to_string(), point, 0.0)
}

fn at(x: f32, y: f32) -> Point {
    Point { x, y }
}

#[test]
fn a_body_goes_to_its_target_and_stops_there() {
    let mut motion = Motion::default();
    motion.sync(&[body("a", at(60.0, 300.0))]);
    motion.set_target("a", at(200.0, 120.0));
    run(&mut motion, 2.0, None);
    let point = motion.position("a").unwrap();
    // 漂浮会绕着目标走（±3/±3.75），所以判定给一点余量
    assert!((point.x - 200.0).abs() < 5.0, "{point:?}");
    assert!((point.y - 120.0).abs() < 6.0, "{point:?}");
}

#[test]
fn a_dragged_body_is_pushed_only_when_it_is_actually_touched() {
    // 用户真机抱怨的「都没遇到就碰」：远处的那枚一点都不许动
    let mut motion = Motion::default();
    motion.sync(&[body("near", at(120.0, 240.0)), body("far", at(260.0, 240.0))]);
    motion.jump("near", at(120.0, 240.0));
    motion.jump("far", at(260.0, 240.0));
    // 关掉漂浮再量：漂浮本身就是一直动的，开着它就分不清「被撞」和「在漂」
    motion.set_drifting("near", false);
    motion.set_drifting("far", false);
    // 拖影从远处逼近，停在离 near 圆心 70px 处（半径和 52 → 没接触）
    let ghost = Ghost { id: "dragged".into(), point: at(50.0, 240.0), radius: layout::DEVICE_RADIUS };
    let before_far = motion.position("far").unwrap();
    run(&mut motion, 0.5, Some(&ghost));
    let after_far = motion.position("far").unwrap();
    assert!(
        layout::distance(before_far, after_far) < 1.0,
        "没碰到的绝不能动：{before_far:?} -> {after_far:?}"
    );
}

#[test]
fn a_touched_body_is_pushed_out_to_exactly_tangent() {
    let mut motion = Motion::default();
    motion.sync(&[body("near", at(140.0, 240.0))]);
    motion.jump("near", at(140.0, 240.0));
    let ghost = Ghost { id: "dragged".into(), point: at(100.0, 240.0), radius: layout::DEVICE_RADIUS };
    run(&mut motion, 0.2, Some(&ghost));
    let point = motion.position("near").unwrap();
    let gap = layout::distance(point, ghost.point);
    assert!((gap - 2.0 * layout::DEVICE_RADIUS).abs() < 1.0, "推开后应恰好相切：{gap:.1}px");
    assert!(point.x > 140.0, "该往远离指针的方向推：{point:?}");
}

#[test]
fn a_swept_body_gets_a_push_away_from_the_pointer() {
    // 「碰到轻轻弹开」：掠过时被推开的节点要有离开指针的速度，而不是被粘着走
    let mut motion = Motion::default();
    motion.sync(&[body("near", at(140.0, 240.0))]);
    motion.jump("near", at(140.0, 240.0));
    let mut ghost = Ghost { id: "dragged".into(), point: at(116.0, 240.0), radius: layout::DEVICE_RADIUS };
    // 指针从右往左扫过（每帧 20px）
    for _ in 0..6 {
        ghost.point = at(ghost.point.x - 20.0, ghost.point.y);
        motion.step(DT, Some(&ghost));
    }
    let point = motion.position("near").unwrap();
    assert!(point.x > 140.0, "被扫到的节点该被推到指针右侧：{point:?}");
}

#[test]
fn a_pushed_body_returns_home_once_the_pointer_leaves() {
    let mut motion = Motion::default();
    motion.sync(&[body("near", at(140.0, 240.0))]);
    motion.jump("near", at(140.0, 240.0));
    let ghost = Ghost { id: "dragged".into(), point: at(100.0, 240.0), radius: layout::DEVICE_RADIUS };
    run(&mut motion, 0.3, Some(&ghost));
    assert!(motion.position("near").unwrap().x > 150.0, "先被推开");
    run(&mut motion, 2.0, None);
    let point = motion.position("near").unwrap();
    assert!((point.x - 140.0).abs() < 5.0, "指针走了要回到原位：{point:?}");
}

#[test]
fn the_dragged_body_follows_the_pointer_one_to_one() {
    let mut motion = Motion::default();
    motion.sync(&[body("dragged", at(140.0, 240.0))]);
    motion.jump("dragged", at(140.0, 240.0));
    let ghost = Ghost { id: "dragged".into(), point: at(80.0, 200.0), radius: layout::DEVICE_RADIUS };
    motion.step(DT, Some(&ghost));
    let point = motion.position("dragged").unwrap();
    assert_eq!(point, ghost.point, "拖起来的那枚必须 1:1 跟手（不加弹簧）");
}

#[test]
fn bodies_never_stay_overlapping_each_other() {
    let mut motion = Motion::default();
    motion.sync(&[body("a", at(140.0, 240.0)), body("b", at(150.0, 245.0))]);
    motion.jump("a", at(140.0, 240.0));
    motion.jump("b", at(150.0, 245.0));
    run(&mut motion, 1.0, None);
    let a = motion.position("a").unwrap();
    let b = motion.position("b").unwrap();
    assert!(
        layout::distance(a, b) >= 2.0 * layout::DEVICE_RADIUS - 1.0,
        "两枚不该叠在一起：{:.1}px",
        layout::distance(a, b)
    );
}

#[test]
fn a_body_shoved_at_the_border_is_held_in_the_figure() {
    let mut motion = Motion::default();
    let corner = at(60.0, 240.0);
    motion.sync(&[body("near", corner)]);
    motion.jump("near", corner);
    motion.set_drifting("near", false);
    // 指针从**内侧**往左挤节点，一路把它推向图区左界
    let mut ghost = Ghost { id: "dragged".into(), point: at(90.0, 240.0), radius: layout::DEVICE_RADIUS };
    for _ in 0..60 {
        ghost.point = at(ghost.point.x - 4.0, ghost.point.y);
        motion.step(DT, Some(&ghost));
    }
    let point = motion.position("near").unwrap();
    let (x0, _) = layout::bound_x();
    // 橡皮筋允许短暂越界（越界后有一股越来越强的回拉），但绝不能跑没影
    assert!(point.x >= x0 - 24.0, "不许越出图区太远：{point:?}（左界 {x0}）");
    // 指针走了以后必须回到图区里
    run(&mut motion, 1.5, None);
    let point = motion.position("near").unwrap();
    assert!(point.x >= x0 - 0.5, "松手后要回到图区内：{point:?}（左界 {x0}）");
}

#[test]
fn sync_keeps_existing_bodies_in_place() {
    // 快照每 300ms 来一次：来一次就把节点瞬移回去是绝对不能接受的
    let mut motion = Motion::default();
    motion.sync(&[body("a", at(60.0, 300.0))]);
    motion.jump("a", at(60.0, 300.0));
    motion.set_target("a", at(200.0, 120.0));
    run(&mut motion, 0.3, None);
    let midway = motion.position("a").unwrap();
    motion.sync(&[
        body("a", at(60.0, 300.0)),
        body("b", at(240.0, 300.0)),
    ]);
    assert_eq!(motion.position("a").unwrap(), midway, "已有节点不许被 sync 挪动");
    assert!(motion.contains("b"), "新设备要进来");
}

#[test]
fn motion_reports_when_everything_has_settled() {
    // 用来决定「还要不要每 16ms 重绘」：全都到位了就该省电
    let mut motion = Motion::default();
    motion.sync(&[body("a", at(120.0, 240.0))]);
    motion.jump("a", at(120.0, 240.0));
    assert!(motion.is_settled());
    motion.set_target("a", at(220.0, 120.0));
    motion.step(DT, None);
    assert!(!motion.is_settled(), "刚改目标就该认为还在动");
    run(&mut motion, 3.0, None);
    assert!(motion.is_settled(), "到位后要报停下");
}

#[test]
fn a_pinned_body_never_moves_but_still_pushes_others() {
    // 本机盘是**钉死的不动体**：别人推不动它，它却要把别人挤开
    // （用户真机看到的 bug：中心盘恢复全尺寸后盖住了周围的气泡）
    let mut motion = Motion::default();
    let center = layout::self_center();
    motion.sync(&[body(layout::SELF_ID, center), body("guest", at(center.x + 50.0, center.y))]);
    motion.set_pinned(layout::SELF_ID, true);
    motion.set_radius(layout::SELF_ID, layout::SELF_RADIUS);
    motion.set_radius("guest", layout::DEVICE_RADIUS);
    motion.set_drifting(layout::SELF_ID, false);
    motion.set_drifting("guest", false);
    motion.jump(layout::SELF_ID, center);
    motion.jump("guest", at(center.x + 50.0, center.y));
    motion.set_target(layout::SELF_ID, center);
    // 目标就是「贴在中心盘上」：物理必须把它推出去到恰好相切
    motion.set_target("guest", at(center.x + 50.0, center.y));
    run(&mut motion, 1.0, None);
    assert_eq!(motion.position(layout::SELF_ID).unwrap(), center, "中心盘一丝都不许动");
    let guest = motion.position("guest").unwrap();
    let gap = layout::distance(guest, center);
    let need = layout::SELF_RADIUS + layout::DEVICE_RADIUS;
    assert!(gap >= need - 1.0, "被挤开后才该停下：{gap:.1}px（需要 {need:.1}px）");
}

#[test]
fn a_connected_device_sits_on_its_slot_and_stays_clear_of_the_center() {
    // 槽位落点必须真的离得开中心盘：本机盘 + 设备盘 + 缝，否则「拼合」会变成「盖上」
    let mut motion = Motion::default();
    let center = layout::self_center();
    let self_radius = layout::SELF_RADIUS * glowlink_desktop::model::connect::CONNECTED_SCALE;
    let device_radius = glowlink_desktop::model::connect::connected_radius();
    let slot = glowlink_desktop::model::connect::point_at(
        center,
        glowlink_desktop::model::connect::FIRST_SLOT,
    );
    motion.sync(&[body(layout::SELF_ID, center), body("tablet", slot)]);
    motion.set_pinned(layout::SELF_ID, true);
    motion.set_radius(layout::SELF_ID, self_radius);
    motion.set_radius("tablet", device_radius);
    motion.set_drifting(layout::SELF_ID, false);
    motion.set_drifting("tablet", false);
    motion.jump(layout::SELF_ID, center);
    motion.jump("tablet", slot);
    motion.set_target(layout::SELF_ID, center);
    motion.set_target("tablet", slot);
    run(&mut motion, 1.0, None);
    assert_eq!(motion.position(layout::SELF_ID).unwrap(), center);
    let landed = motion.position("tablet").unwrap();
    let gap = layout::distance(landed, center);
    assert!(
        gap >= self_radius + device_radius - 0.5,
        "两台缩小盘必须相切而不重叠：{gap:.1}px（需要 {}px）",
        self_radius + device_radius
    );
    assert!((landed.x - slot.x).abs() < 2.0 && (landed.y - slot.y).abs() < 2.0, "该停在它的槽位上");
}

#[test]
fn a_body_dragged_out_of_the_window_never_leaves_the_figure() {
    // 用户真机：把气泡拖到窗口外松手，事件收不到 → 气泡贴在边框上不动。
    // 第一道防线是几何：拖起来的坐标先过橡皮筋，圆心永远留在图区里。
    let (x0, x1) = layout::bound_x();
    let (y0, y1) = layout::bound_y();
    let far = at(-800.0, -500.0);
    let (clamped, was_clamped) = rubber_clamp(far, layout::DEVICE_RADIUS);
    assert!(was_clamped, "越界了就该报「夹过」");
    // 允许最多 RUBBER_LIMIT 的越界（渐近线），但绝不能「跟到无穷远」
    assert!(clamped.x >= x0 - RUBBER_LIMIT, "圆心出图区太远：{clamped:?}");
    assert!(clamped.x <= x1 + RUBBER_LIMIT, "圆心出图区太远：{clamped:?}");
    assert!(clamped.y >= y0 - RUBBER_LIMIT, "圆心出图区太远：{clamped:?}");
    assert!(clamped.y <= y1 + RUBBER_LIMIT, "圆心出图区太远：{clamped:?}");
    // 图区内的点一个像素都不许动
    let inside = at(160.0, 200.0);
    assert_eq!(rubber_clamp(inside, layout::DEVICE_RADIUS), (inside, false));
}

#[test]
fn the_boundary_resists_more_the_further_you_push() {
    // 橡皮筋：越界越多、跟得越少（硬夹读起来是「冻住了」）
    let one = glowlink_desktop::model::motion::rubberband(10.0, RUBBER_LIMIT, 0.55);
    let two = glowlink_desktop::model::motion::rubberband(20.0, RUBBER_LIMIT, 0.55);
    assert!(one < 10.0 && two < 20.0, "越界部分必须被打折：{one:.1} / {two:.1}");
    assert!(two > one, "越界越多还是该多跟一点：{one:.1} / {two:.1}");
    assert!(two < 2.0 * one, "但不许线性跟下去（那就不叫阻力了）");
    // 渐近线就是上限：推得再远也不超过它
    let far = glowlink_desktop::model::motion::rubberband(10_000.0, RUBBER_LIMIT, 0.55);
    assert!(far < RUBBER_LIMIT, "越界必须有上限：{far:.1}");
}

#[test]
fn the_dragged_body_stops_at_the_edge_when_the_pointer_leaves() {
    let mut motion = Motion::default();
    motion.sync(&[body("dragged", at(160.0, 200.0))]);
    motion.jump("dragged", at(160.0, 200.0));
    let ghost = Ghost {
        id: "dragged".into(),
        point: at(-400.0, 200.0),
        radius: layout::DEVICE_RADIUS,
    };
    motion.step(DT, Some(&ghost));
    let point = motion.position("dragged").unwrap();
    let (x0, _) = layout::bound_x();
    assert!(point.x >= x0 - RUBBER_LIMIT, "拖出去的坐标要被橡皮筋拦住：{point:?}");
}
