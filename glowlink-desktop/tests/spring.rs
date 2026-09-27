//! `model/spring.rs`：唯一的运动原语。

use glowlink_desktop::model::spring::{Spring, ZETA_CRITICAL, ZETA_MOMENTUM};

/// 固定步长推进，返回走过的总时长（秒）。测试里一律用它，避免依赖真实时钟。
fn advance(spring: &mut Spring, total: f32, dt: f32) {
    let mut left = total;
    while left > 0.0 {
        let h = left.min(dt);
        left -= h;
        spring.step(h);
    }
}

#[test]
fn a_critical_spring_reaches_its_target_without_overshooting() {
    let mut spring = Spring::new(0.0, ZETA_CRITICAL, 0.35);
    spring.retarget(100.0);
    let mut previous = spring.x();
    let mut left = 3.0;
    while left > 0.0 {
        spring.step(1.0 / 60.0);
        left -= 1.0 / 60.0;
        // 临界阻尼：不许冲过目标（过冲就是「弹」，只有带着动量松手才允许）
        assert!(spring.x() <= 100.0 + 1e-3, "过冲了：{}", spring.x());
        assert!(spring.x() >= previous - 1e-3, "不该倒退：{} -> {}", previous, spring.x());
        previous = spring.x();
    }
    assert!((spring.x() - 100.0).abs() < 0.5, "3 秒后该到位：{}", spring.x());
    assert!(spring.is_settled(), "该判定为已停下：{}", spring.x());
}

#[test]
fn an_underdamped_spring_overshoots() {
    // 0.8 阻尼比 = 会过冲（这是「带着动量松手」才给的参数）
    let mut spring = Spring::new(0.0, ZETA_MOMENTUM, 0.35);
    spring.retarget(100.0);
    let mut peak: f32 = 0.0;
    let mut left = 3.0;
    while left > 0.0 {
        spring.step(1.0 / 60.0);
        left -= 1.0 / 60.0;
        peak = peak.max(spring.x());
    }
    assert!(peak > 100.5, "0.8 阻尼比应该过冲：峰值 {peak:.1}");
}

#[test]
fn retargeting_keeps_position_and_velocity() {
    // 可打断（skill §3）：中途换目标不能跳位置、也不能把速度清零
    let mut spring = Spring::new(0.0, ZETA_CRITICAL, 0.35);
    spring.retarget(100.0);
    advance(&mut spring, 0.15, 1.0 / 240.0);
    let (position, velocity) = (spring.x(), spring.velocity());
    assert!(position > 1.0 && velocity > 1.0, "应该已经在路上：{position:.1} / {velocity:.1}");
    spring.retarget(-50.0);
    assert_eq!(spring.x(), position, "换目标不许跳位置");
    assert_eq!(spring.velocity(), velocity, "换目标不许丢速度");
    advance(&mut spring, 3.0, 1.0 / 240.0);
    assert!((spring.x() + 50.0).abs() < 0.5, "最终该到新目标：{}", spring.x());
}

#[test]
fn the_trajectory_does_not_depend_on_the_frame_rate() {
    // 帧率无关：60fps 与 240fps 走同样长的时间，落点应当一致
    let mut slow = Spring::new(0.0, ZETA_CRITICAL, 0.35);
    let mut fast = Spring::new(0.0, ZETA_CRITICAL, 0.35);
    slow.retarget(100.0);
    fast.retarget(100.0);
    advance(&mut slow, 0.4, 1.0 / 60.0);
    advance(&mut fast, 0.4, 1.0 / 240.0);
    assert!((slow.x() - fast.x()).abs() < 0.5, "{} vs {}", slow.x(), fast.x());
}

#[test]
fn a_huge_gap_does_not_launch_the_spring() {
    // 切走再切回来时 dt 可能是几十秒：必须被夹住，而不是「嗖」地飞出去
    let mut spring = Spring::new(0.0, ZETA_CRITICAL, 0.35);
    spring.retarget(100.0);
    spring.step(30.0);
    assert!(spring.x() <= 100.5, "一帧最多走到目标：{}", spring.x());
}

#[test]
fn kick_sets_velocity_and_jump_resets_everything() {
    let mut spring = Spring::new(10.0, ZETA_CRITICAL, 0.35);
    spring.kick(50.0);
    assert_eq!(spring.velocity(), 50.0);
    spring.jump(7.0);
    assert_eq!(spring.x(), 7.0);
    assert_eq!(spring.velocity(), 0.0, "硬置位要连速度一起归零");
    assert!(spring.is_settled());
}
