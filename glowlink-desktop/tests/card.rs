//! `glowlink-desktop/src/view/card.rs`：浮层卡片的共用外壳（几何口径）。
//!
//! 这些断言的由来：配对卡曾自己写死 `bottom: 12`，被拖拽区盖住，
//! 用户看到的是「卡片跑到拖拽按钮下面去了」。位置只有一个定义，就不会再分叉。

use glowlink_desktop::theme;
use glowlink_desktop::view::card;

#[test]
fn the_card_sits_above_the_dropzone_not_under_it() {
    // 底边必须**让开**整个拖拽区（这是被 bug 咬过的一条）
    assert!(
        card::CARD_BOTTOM >= theme::DROPZONE_INSET + theme::DROPZONE_H,
        "卡片底边 {} 低于拖拽区上沿 {}，会被拖拽区盖住",
        card::CARD_BOTTOM,
        theme::DROPZONE_INSET + theme::DROPZONE_H
    );
    // 也别飘太高：配音卡与拖拽区是一列
    assert!(card::CARD_BOTTOM < theme::DROPZONE_INSET + theme::DROPZONE_H + 24.0);
}

#[test]
fn the_card_and_the_dropzone_share_one_column() {
    // 同宽同边距：两张卡（传输 / 配对）左右对齐，肉眼才是一条竖线
    assert_eq!(card::CARD_INSET, theme::DROPZONE_INSET);
}

#[test]
fn the_close_button_gets_reserved_room_in_the_title_row() {
    // 标题行右侧要给 ✕ 留位置，否则长文件名/大号配对码会压到按钮底下
    assert!(
        card::title_reserve() >= card::CLOSE_SIZE,
        "预留 {} 放不下直径 {} 的按钮",
        card::title_reserve(),
        card::CLOSE_SIZE
    );
}
