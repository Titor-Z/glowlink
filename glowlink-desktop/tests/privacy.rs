//! 隐私浮层（`model/privacy.rs`）：文案是**事实陈述**，所以这里钉的是「说的和做的一致」。

use glowlink_desktop::model::privacy;

#[test]
fn the_three_promises_are_the_designed_ones() {
    // 三条承诺是设计稿原文，顺序也是规格；加一条、删一条、改词都该在这里被发现
    assert_eq!(
        privacy::PROMISES,
        ["无云端 · 无中转", "无注册 · 无账号", "无遥测 · 无日志"]
    );
    assert_eq!(privacy::TITLE, "隐私承诺");
}

#[test]
fn the_footnote_names_the_port_we_actually_use() {
    // 设计稿的原话：内容是事实陈述，不是宣传。事实里出现**过期端口**比不写更伤，
    // 所以端口从 core 常量生成，这里再把两头钉死（写死数字就会漂）。
    let foot = privacy::footnote();
    assert!(
        foot.contains(&glowlink_core::DISCOVERY_PORT.to_string()),
        "脚注里的端口必须与实现一致：{foot}"
    );
    // 早期端口写在设计稿示例里，落地时不许照抄
    assert!(!foot.contains("8888"), "8888 是早期端口，不该再出现：{foot}");
    // 机制与加密状态也必须是事实
    assert!(foot.contains("UDP 广播") && foot.contains("TCP 点对点"));
    assert!(foot.contains("端到端加密"));
}

#[test]
fn the_popover_geometry_matches_the_design() {
    // 设计稿 `.privacy` 的数值：236 宽 / top 48 / right 10 / 圆角 14 / 内边距 12
    assert_eq!(privacy::WIDTH, 236.0);
    assert_eq!(privacy::TOP, 48.0);
    assert_eq!(privacy::RIGHT, 10.0);
    assert_eq!(privacy::RADIUS, 14.0);
    assert_eq!(privacy::PAD, 12.0);
    assert_eq!(privacy::DOT, 4.0);
}

#[test]
fn the_entrance_settles_and_never_overshoots() {
    // 起点：透明、在上方；终点：不透明、归位
    let (o0, dy0) = privacy::pop_in(0.0);
    let (o1, dy1) = privacy::pop_in(1.0);
    assert_eq!((o0, dy0), (0.0, privacy::IN_SHIFT));
    assert_eq!((o1, dy1), (1.0, 0.0));
    assert!(privacy::animating(0.5) && !privacy::animating(1.0));
    // 越界的进度按落定算（帧率抖动时不许越跑越远）
    assert_eq!(privacy::pop_in(3.0), (1.0, 0.0));
    assert_eq!(privacy::pop_in(-1.0), (0.0, privacy::IN_SHIFT));
    // 单调：不透明度只增、位移只减，中途不会回弹
    let mut last = (0.0, privacy::IN_SHIFT);
    for step in 1..=10 {
        let now = privacy::pop_in(step as f32 / 10.0);
        assert!(now.0 >= last.0 && now.1 <= last.1, "入场曲线中途回头了");
        last = now;
    }
}
