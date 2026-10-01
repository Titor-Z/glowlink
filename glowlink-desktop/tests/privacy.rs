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
fn the_footnote_promises_encryption_in_plain_words() {
    // 脚注不再是"可自行抓包验证：仅 UDP 广播 9267…"（用户拍板改掉）：
    // ① 那是工程师的话；② 端口被占用会顺延，那时"仅 9267"就是假话——
    //    可验证的实现细节留给文档，这一层只说用户能懂的那条最强事实。
    let foot = privacy::FOOTNOTE;
    assert!(foot.contains("端到端加密"), "必须说清加密：{foot}");
    assert!(foot.contains("只有你与对方"), "必须说清只有双方能解开：{foot}");
    assert!(foot.starts_with("本软件承诺"), "口径是承诺，不是宣传：{foot}");
    // 不许再把实现词写回来：端口/协议/工具名一出现，就说明又漂回工程师视角了
    for word in ["抓包", "UDP", "TCP", "9267", "8888", "端口"] {
        assert!(!foot.contains(word), "脚注里不该出现「{word}」：{foot}");
    }
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
