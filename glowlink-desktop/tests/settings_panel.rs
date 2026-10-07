//! `view/settings.rs`（设置面板）的命中测试回归：**浮层必须挡住下面的元素**。
//!
//! 背景：设置面板盖在拖拽区上，用户却能点穿面板点到拖拽区——浮层「只管画、
//! 不管命中」。修复用的是 gpui 的 `occlude()`（`HitboxBehavior::BlockMouse`），
//! 但这类行为文档说得再多也不如实证：这里用与产品**完全相同的结构**
//! （相对根 → 底部按钮 → `absolute().inset_0()` 包裹 → 面板 `occlude`）
//! 注入真实鼠标事件（down + up），断言按钮收不到点击。
//!
//! 需要 gpui 的 `test-support`（dev-dependencies 里已开）：只有它能注入
//! 鼠标事件并跑真实的 `hit_test`。

use std::{cell::Cell, rc::Rc};

use gpui::{
    div, prelude::*, px, InputEvent as _, IntoElement, MouseButton, MouseDownEvent, MouseMoveEvent,
    MouseUpEvent, ParentElement, Pixels, Render, Styled, TestAppContext, Window,
};

/// 按钮中心：在浮层覆盖区里（浮层从 44 起，铺到底）。
const COVERED_POS: (f32, f32) = (150.0, 470.0);

struct Probe {
    /// 是否画浮层（对照组不画，验证测试装置本身有效）。
    overlay: bool,
    /// 按钮被点过吗。
    clicked: Rc<Cell<bool>>,
}

impl Render for Probe {
    fn render(&mut self, _window: &mut Window, _cx: &mut gpui::Context<Self>) -> impl IntoElement {
        let clicked = self.clicked.clone();
        let button = div()
            .id("probe-dropzone")
            .absolute()
            .left(px(50.0))
            .top(px(450.0))
            .w(px(200.0))
            .h(px(40.0))
            .on_click(move |_, _, _| clicked.set(true));
        let root = div().relative().size_full().child(button);
        if self.overlay {
            // 与 `view::settings::panel` / `view::app` 同构：
            // 包裹层 inset_0，面板本体 occlude
            root.child(
                div().absolute().inset_0().child(
                    div()
                        .id("probe-panel")
                        .absolute()
                        .top(px(44.0))
                        .left_0()
                        .right_0()
                        .bottom_0()
                        .occlude(),
                ),
            )
        } else {
            root
        }
    }
}

fn click_at(cx: &mut TestAppContext, window: gpui::AnyWindowHandle, pos: (f32, f32)) {
    cx.update_window(window, |_, window, cx| {
        window.draw(cx).clear(cx);
        let position = gpui::point(px(pos.0), px(pos.1));
        window
            .dispatch_event(
                MouseMoveEvent {
                    position,
                    pressed_button: None,
                    modifiers: Default::default(),
                }
                .to_platform_input(),
                cx,
            );
        window
            .dispatch_event(
                MouseDownEvent {
                    position,
                    button: MouseButton::Left,
                    modifiers: Default::default(),
                    click_count: 1,
                    first_mouse: false,
                }
                .to_platform_input(),
                cx,
            );
        window
            .dispatch_event(
                MouseUpEvent {
                    position,
                    button: MouseButton::Left,
                    modifiers: Default::default(),
                    click_count: 1,
                }
                .to_platform_input(),
                cx,
            );
    })
    .unwrap();
}

#[test]
fn sanity_button_without_overlay_receives_clicks() {
    let mut cx = TestAppContext::single();
    let clicked = Rc::new(Cell::new(false));
    let window = cx.add_window({
        let clicked = clicked.clone();
        move |_, _| Probe { overlay: false, clicked }
    });
    click_at(&mut cx, window.into(), COVERED_POS);
    assert!(
        clicked.get(),
        "没有浮层时按钮必须能点：测试装置本身要先有效"
    );
}

#[test]
fn occluding_overlay_blocks_clicks_to_elements_below() {
    let mut cx = TestAppContext::single();
    let clicked = Rc::new(Cell::new(false));
    let window = cx.add_window({
        let clicked = clicked.clone();
        move |_, _| Probe { overlay: true, clicked }
    });
    click_at(&mut cx, window.into(), COVERED_POS);
    assert!(
        !clicked.get(),
        "浮层（occlude）必须挡住下方按钮的点击——用户在设置面板里点穿到拖拽区就是这个断言在管"
    );
}
