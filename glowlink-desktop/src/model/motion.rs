//! 运动系统：把「节点在哪儿」收敛成**一处**状态。
//!
//! 之前「一个位置有五个来源、在渲染函数里用 if 级联决定」（接触 > 连接环 >
//! 过渡动画 > 漂浮），渲染层同时干着布局、物理、计时三件事。现在拆成：
//!
//! - **意图**：`set_target` / `set_pointer` 告诉它「想去哪」（谁想动由上层决定）
//! - **约束**：接触解析（只对真重叠做修正 + 速度冲量，局部、不重排全局）
//! - **动力学**：弹簧积分（唯一的运动原语，见 [`super::spring`]）
//! - **表现**：`position` 只读——渲染层不再有任何 if
//!
//! 一个时钟（[`Motion::step`] 的 dt）、一套状态（[`Motion::bodies`]）。

use super::layout::{self, Point};
use super::spring::{Spring, ZETA_CRITICAL};

/// 常规位移的响应时间（秒）：0.35 ≈ 快而不急。（skill 表格「Move/reposition」= 0.4）
pub const RESPONSE_MOVE: f32 = 0.35;
/// 边界的橡皮筋（skill §9：越界后阻力渐增，而不是硬停）。
pub const RESPONSE_BOUND: f32 = 0.12;
/// 接触回弹系数：0 = 完全吸收（不弹），0.2 = 轻轻弹开。
pub const RESTITUTION: f32 = 0.2;
/// 漂浮：周期与幅度（与设计稿一致：慢、小，不能干扰「拖动/连接」这些真动作）。
pub const DRIFT_PERIOD_MS: f32 = 7000.0;
pub const DRIFT_AMP_X: f32 = 3.0;
pub const DRIFT_AMP_Y: f32 = 3.75;

/// 橡皮筋：越界越多、跟得越少（skill §9）。
///
/// 公式与 Apple 一致：`overshoot * dimension * c / (dimension + c * |overshoot|)`。
/// 注意 `dimension` 是**渐近线**——也就是「最多允许越界多少像素」，不是窗口尺寸
/// （曾按窗口宽传 396，结果渐近线也在 396，等于没拦）。小块越界按 `c` 打折（≈0.55 倍）。
pub fn rubberband(overshoot: f32, dimension: f32, constant: f32) -> f32 {
    (overshoot * dimension * constant) / (dimension + constant * overshoot.abs())
}

/// 橡皮筋的最大越界量（px）：圆心最多出界这么多，再多也不动。
/// 图区外面本来就是圆的窗角与 `overflow_hidden`，24px 的余量读起来就是「到头了」。
pub const RUBBER_LIMIT: f32 = 24.0;

/// 把一个点按橡皮筋夹进图区：返回（夹后坐标，是否被夹过）。
///
/// 拖到窗口外时用：气泡**永远不许真的出图区**（松手事件可能在窗口外丢失，
/// 那就会留下一个贴在边框上的气泡），越界时给出渐增的阻力。
pub fn rubber_clamp(point: Point, radius: f32) -> (Point, bool) {
    let (x0, x1) = layout::bound_x();
    let (y0, y1) = layout::bound_y();
    let mut clamped = false;
    let mut fix = |value: f32, low: f32, high: f32| {
        let _ = high;
        if value < low {
            clamped = true;
            low - rubberband(low - value, RUBBER_LIMIT, RUBBER_CONSTANT)
        } else if value > high {
            clamped = true;
            high + rubberband(value - high, RUBBER_LIMIT, RUBBER_CONSTANT)
        } else {
            value
        }
    };
    // 半径不参与夹取：圆心范围（`bound_x/bound_y`）本来就已经把盘与名字算进去了
    let _ = radius;
    let x = fix(point.x, x0, x1);
    let y = fix(point.y, y0, y1);
    (Point { x, y }, clamped)
}

/// 橡皮筋常数：越小越「硬」（Apple 网页版给 0.55）。
pub const RUBBER_CONSTANT: f32 = 0.55;

/// 拖起来的那一枚：**运动学**物体——位置就是指针（1:1，不加弹簧），
/// 且对别人来说是不可推动的障碍物（只有它推别人，别人推不动它）。
/// 它的速度由指针的位移算出来，松手时正好当速度交接用（skill §5）。
#[derive(Clone, Debug)]
pub struct Ghost {
    pub id: String,
    pub point: Point,
    pub radius: f32,
}

struct Body {
    id: String,
    radius: f32,
    sx: Spring,
    sy: Spring,
    /// 意图点（不含漂浮）。`target` = 它 + 漂浮偏移。
    base: Point,
    /// 漂浮相位（由设备 ID 派生，见 `layout::seeded_phase`）。
    phase: f32,
    /// 是否参与漂浮（拖着的、连接中的都不漂）。
    drifting: bool,
    /// **钉死的不动体**（本机盘）：位置由界面固定，弹簧不动它、接触也推不动它，
    /// 但它会把别人推开。
    pinned: bool,
    /// 上一帧的指针位置与速度（拖拽的速度交接用）。
    pointer_velocity: Point,
}

/// 全场运动状态。设备增删只改列表，**已有的位置与速度一律保留**
/// （否则每来一次快照，节点就从原地瞬移回去）。
#[derive(Default)]
pub struct Motion {
    bodies: Vec<Body>,
    clock_ms: f32,
}

impl Motion {
    /// 对齐设备列表：新增的落在给定落点上，消失的丢掉，**其余原位原位**。
    pub fn sync(&mut self, wanted: &[(String, Point, f32)]) {
        self.bodies.retain(|body| wanted.iter().any(|(id, _, _)| *id == body.id));
        for (id, rest, phase) in wanted {
            if self.bodies.iter().any(|body| body.id == *id) {
                continue;
            }
            self.bodies.push(Body {
                id: id.clone(),
                radius: layout::DEVICE_RADIUS,
                sx: Spring::new(rest.x, ZETA_CRITICAL, RESPONSE_MOVE),
                sy: Spring::new(rest.y, ZETA_CRITICAL, RESPONSE_MOVE),
                base: *rest,
                phase: *phase,
                drifting: true,
                pinned: false,
                pointer_velocity: Point { x: 0.0, y: 0.0 },
            });
        }
    }

    pub fn contains(&self, id: &str) -> bool {
        self.bodies.iter().any(|body| body.id == id)
    }

    /// 换半径（本机盘比设备盘大）。
    pub fn set_radius(&mut self, id: &str, radius: f32) {
        if let Some(body) = self.bodies.iter_mut().find(|b| b.id == id) {
            body.radius = radius;
        }
    }

    /// 这台设备「想去哪」。
    pub fn set_target(&mut self, id: &str, point: Point) {
        if let Some(body) = self.bodies.iter_mut().find(|b| b.id == id) {
            body.base = point;
        }
    }

    /// 钉死（本机盘）：位置只由界面给，弹簧与接触都动不了它。
    pub fn set_pinned(&mut self, id: &str, pinned: bool) {
        if let Some(body) = self.bodies.iter_mut().find(|b| b.id == id) {
            body.pinned = pinned;
        }
    }

    /// 是否参与漂浮（拖起来的、已连接的都不漂：它们的位置由别人说了算）。
    pub fn set_drifting(&mut self, id: &str, drifting: bool) {
        if let Some(body) = self.bodies.iter_mut().find(|b| b.id == id) {
            body.drifting = drifting;
        }
    }

    /// 硬置位（刚出现 / 重置），位置与速度都归零。
    pub fn jump(&mut self, id: &str, point: Point) {
        if let Some(body) = self.bodies.iter_mut().find(|b| b.id == id) {
            body.sx.jump(point.x);
            body.sy.jump(point.y);
            body.base = point;
        }
    }

    /// 施加速度（接触冲量之外的额外推力）。
    pub fn kick(&mut self, id: &str, velocity: Point) {
        if let Some(body) = self.bodies.iter_mut().find(|b| b.id == id) {
            body.sx.kick(velocity.x);
            body.sy.kick(velocity.y);
        }
    }

    /// 松手时的一次性参数调整：给一点回弹（只有带着动量松手才该弹，skill §4）。
    pub fn set_zeta(&mut self, id: &str, zeta: f32) {
        if let Some(body) = self.bodies.iter_mut().find(|b| b.id == id) {
            body.sx = Spring::new(body.sx.x(), zeta, RESPONSE_MOVE);
            body.sy = Spring::new(body.sy.x(), zeta, RESPONSE_MOVE);
        }
    }

    pub fn position(&self, id: &str) -> Option<Point> {
        self.bodies.iter().find(|b| b.id == id).map(|b| Point { x: b.sx.x(), y: b.sy.x() })
    }

    pub fn radius(&self, id: &str) -> f32 {
        self.bodies.iter().find(|b| b.id == id).map(|b| b.radius).unwrap_or(layout::DEVICE_RADIUS)
    }

    /// 有没有**自己的**运动（漂浮不算：它一直在动，而且本来就该一直在动）。
    ///
    /// 判定口径：位置落在漂浮包络里、速度不超过漂浮的峰值速度。
    pub fn is_settled(&self) -> bool {
        /// 漂浮的峰值速度 ≈ 幅度 × 2π / 周期（7s、±3.75px → ≈3.4px/s），留一点余量
        const DRIFT_VELOCITY: f32 = 7.0;
        self.bodies.iter().all(|body| {
            (body.sx.x() - body.base.x).abs() <= DRIFT_AMP_X + 1.0
                && (body.sy.x() - body.base.y).abs() <= DRIFT_AMP_Y + 1.0
                && body.sx.velocity().abs() <= DRIFT_VELOCITY
                && body.sy.velocity().abs() <= DRIFT_VELOCITY
        })
    }

    /// 前进一帧：**唯一的时钟**。
    ///
    /// 顺序是有讲究的：先把「意图 + 漂浮」写进目标，再积分（弹簧），最后解约束
    /// （位置修正 + 速度冲量）。反过来会让约束每帧被弹簧撤销一半，接触看起来是软的。
    pub fn step(&mut self, dt: f32, ghost: Option<&Ghost>) {
        self.clock_ms += dt * 1000.0;

        // ① 意图 → 目标（漂浮只是一条**随时间游走的目标**，不是另一套动画）
        for body in self.bodies.iter_mut() {
            let drift = if body.drifting
                && !body.pinned
                && ghost.map(|g| g.id != body.id).unwrap_or(true)
            {
                let theta =
                    (self.clock_ms / DRIFT_PERIOD_MS + body.phase) * std::f32::consts::TAU;
                Point { x: DRIFT_AMP_X * theta.sin(), y: DRIFT_AMP_Y * theta.cos() }
            } else {
                Point { x: 0.0, y: 0.0 }
            };
            body.sx.retarget(body.base.x + drift.x);
            body.sy.retarget(body.base.y + drift.y);
        }

        // ② 拖起来的那枚是运动学的：位置就是指针，速度由指针位移算出
        if let Some(ghost) = ghost {
            if let Some(body) = self.bodies.iter_mut().find(|b| b.id == ghost.id) {
                let previous = Point { x: body.sx.x(), y: body.sy.x() };
                if dt > 0.0 {
                    body.pointer_velocity = Point {
                        x: (ghost.point.x - previous.x) / dt,
                        y: (ghost.point.y - previous.y) / dt,
                    };
                }
                // 指针可能跑到图区外（窗口外）：先过橡皮筋，气泡绝不真的出图
                let (point, _) = rubber_clamp(ghost.point, ghost.radius);
                body.radius = ghost.radius;
                body.base = point;
                body.sx.jump(point.x);
                body.sy.jump(point.y);
            }
        }

        // ③ 动力学（钉死的物体不走弹簧：它只在界面的指挥下移动）
        for body in self.bodies.iter_mut() {
            if body.pinned {
                continue;
            }
            body.sx.step(dt);
            body.sy.step(dt);
        }

        // ④ 接触：只对**真的重叠**动手，且只动接触的那几枚
        self.resolve_contacts(dt, ghost);

        // ⑤ 边界：橡皮筋（越界后阻力渐增并拉回来，而不是硬夹死在边上）
        self.hold_in_bounds();
    }

    /// 接触解析：位置修正 + 法向速度冲量（「碰到轻轻弹开」= 推开 + 一点回弹）。
    fn resolve_contacts(&mut self, dt: f32, ghost: Option<&Ghost>) {
        const PASSES: usize = 3;
        for _ in 0..PASSES {
            if let Some(ghost) = ghost {
                let ghost_velocity = self
                    .bodies
                    .iter()
                    .find(|b| b.id == ghost.id)
                    .map(|b| b.pointer_velocity)
                    .unwrap_or(Point { x: 0.0, y: 0.0 });
                for body in self.bodies.iter_mut() {
                    if body.id == ghost.id || body.pinned {
                        continue;
                    }
                    let point = Point { x: body.sx.x(), y: body.sy.x() };
                    let Some((nx, ny, overlap)) =
                        layout::separation(point, ghost.point, body.radius + ghost.radius)
                    else {
                        continue;
                    };
                    // 位置：推到恰好相切（障碍物＝指针，推不动，位移全给节点）
                    let moved = Point { x: point.x - nx * overlap, y: point.y - ny * overlap };
                    body.sx.jump(moved.x);
                    body.sy.jump(moved.y);
                    // 速度：只沿法向结算（切向保持），否则掠过时会「粘住」
                    let normal_velocity = (body.sx.velocity() - ghost_velocity.x) * nx
                        + (body.sy.velocity() - ghost_velocity.y) * ny;
                    if normal_velocity < 0.0 {
                        let impulse = -(1.0 + RESTITUTION) * normal_velocity;
                        body.sx.kick(body.sx.velocity() + impulse * nx);
                        body.sy.kick(body.sy.velocity() + impulse * ny);
                    }
                }
            }
            // 节点之间：两边等质量，各让一半、各拿一半冲量
            for i in 0..self.bodies.len() {
                for j in (i + 1)..self.bodies.len() {
                    let (ax, ay) = (self.bodies[i].sx.x(), self.bodies[i].sy.x());
                    let (bx, by) = (self.bodies[j].sx.x(), self.bodies[j].sy.x());
                    let need = self.bodies[i].radius + self.bodies[j].radius;
                    let Some((nx, ny, overlap)) = layout::separation(
                        Point { x: ax, y: ay },
                        Point { x: bx, y: by },
                        need,
                    ) else {
                        continue;
                    };
                    // 钉死的物体（本机盘）不动，位移与冲量全给对方；两个都能动的各让一半
                    let (pinned_i, pinned_j) = (self.bodies[i].pinned, self.bodies[j].pinned);
                    if pinned_i && pinned_j {
                        continue;
                    }
                    let (share_i, share_j) = if pinned_i {
                        (0.0, 1.0)
                    } else if pinned_j {
                        (1.0, 0.0)
                    } else {
                        (0.5, 0.5)
                    };
                    if share_i > 0.0 {
                        self.bodies[i].sx.jump(ax - nx * overlap * share_i);
                        self.bodies[i].sy.jump(ay - ny * overlap * share_i);
                    }
                    if share_j > 0.0 {
                        self.bodies[j].sx.jump(bx + nx * overlap * share_j);
                        self.bodies[j].sy.jump(by + ny * overlap * share_j);
                    }
                    let relative = (self.bodies[j].sx.velocity() - self.bodies[i].sx.velocity()) * nx
                        + (self.bodies[j].sy.velocity() - self.bodies[i].sy.velocity()) * ny;
                    if relative < 0.0 {
                        // 无限质量那一侧收全部冲量系数（0.5 是等质量各半）
                        let impulse = -(1.0 + RESTITUTION) * relative;
                        if share_i > 0.0 {
                            let (vi, vj) =
                                (self.bodies[i].sx.velocity(), self.bodies[j].sx.velocity());
                            self.bodies[i].sx.kick(vi - impulse * share_i * nx);
                            self.bodies[j].sx.kick(vj + impulse * share_j * nx);
                            let (vi, vj) =
                                (self.bodies[i].sy.velocity(), self.bodies[j].sy.velocity());
                            self.bodies[i].sy.kick(vi - impulse * share_i * ny);
                            self.bodies[j].sy.kick(vj + impulse * share_j * ny);
                        }
                    }
                }
            }
        }
        // 边界橡皮筋：越界就吃一股回拉力，不直接夹死（skill §9）
        let (x0, x1) = layout::bound_x();
        let (y0, y1) = layout::bound_y();
        for body in self.bodies.iter_mut() {
            if body.pinned {
                continue;
            }
            for (spring, low, high) in [(&mut body.sx, x0, x1), (&mut body.sy, y0, y1)] {
                let limit = spring.x().clamp(low, high);
                if spring.x() != limit {
                    let omega = std::f32::consts::TAU / RESPONSE_BOUND;
                    let pull = -(omega * omega) * (spring.x() - limit);
                    spring.kick(spring.velocity() + pull * dt.max(1.0 / 240.0));
                }
            }
        }
    }

    /// 越界的最终兜底：橡皮筋允许短暂越界，但绝不许消失在窗口外。
    fn hold_in_bounds(&mut self) {
        const OUTSIDE: f32 = 24.0;
        let (x0, x1) = layout::bound_x();
        let (y0, y1) = layout::bound_y();
        for body in self.bodies.iter_mut() {
            if body.pinned {
                continue;
            }
            let x = body.sx.x();
            if x < x0 - OUTSIDE || x > x1 + OUTSIDE {
                body.sx.jump(x.clamp(x0 - OUTSIDE, x1 + OUTSIDE));
            }
            let y = body.sy.x();
            if y < y0 - OUTSIDE || y > y1 + OUTSIDE {
                body.sy.jump(y.clamp(y0 - OUTSIDE, y1 + OUTSIDE));
            }
        }
    }
}
