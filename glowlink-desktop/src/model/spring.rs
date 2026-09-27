//! 弹簧：本项目**唯一**的运动原语。
//!
//! 为什么不用「一次性缓动」（我们之前就是这么写的，并因此栽过两次）：
//! - 一次性动画有**固定时长**，中途改目标只能重头播 → 速度突变，看起来像撞墙。
//!   弹簧没有时长：目标随时可改，位置与速度都连续（「打断」等于「换一个目标」）。
//! - 之前还把状态塞进动画 id，导致无关变量也会触发重播。弹簧压根没有 id。
//!
//! 参数按 Apple 的口径（不是质量/刚度/阻尼那三件套）：
//! - `zeta` 阻尼比：`1.0` 临界阻尼不弹；`< 1.0` 会过冲（只给「带着动量松手」用）
//! - `response` 响应时间（秒）：不是时长，是「多快够到目标」；越大越慢

/// 临界阻尼：不弹（绝大多数位移都用它）。
pub const ZETA_CRITICAL: f32 = 1.0;
/// 带着动量松手时给一点点回弹。
pub const ZETA_MOMENTUM: f32 = 0.8;

/// 单帧最多积分多少秒：切到别的 App 再切回来时 dt 可能是几十秒，
/// 直接积分会让节点「嗖」地飞出去。
const MAX_FRAME: f32 = 1.0 / 30.0;
/// 积分步长上限：弹簧是刚的（ω≈18），大步长会数值发散。
const MAX_SUBSTEP: f32 = 1.0 / 240.0;
/// 「停下」的判定：位置误差与速度都小于它就算到位（亚像素，看不见）。
const SETTLED_POS: f32 = 0.02;
const SETTLED_VEL: f32 = 0.5;

/// 一维弹簧。二维运动用两个（X / Y 各自独立——一个二维弹簧在 X、Y 速度不同时会「拧」）。
#[derive(Clone, Copy, Debug)]
pub struct Spring {
    x: f32,
    v: f32,
    target: f32,
    zeta: f32,
    response: f32,
}

impl Spring {
    pub fn new(x: f32, zeta: f32, response: f32) -> Self {
        Self { x, v: 0.0, target: x, zeta, response }
    }

    pub fn x(&self) -> f32 {
        self.x
    }

    pub fn velocity(&self) -> f32 {
        self.v
    }

    /// 换目标：**保留速度**。这就是「可打断」的全部实现（skill §3）。
    pub fn retarget(&mut self, target: f32) {
        self.target = target;
    }

    /// 换响应时间（保留位置与速度）：用来做「亮起快、熄灭慢」这种不对称过渡。
    pub fn set_response(&mut self, response: f32) {
        self.response = response;
    }

    /// 外力给的速度（接触冲量、松手时的指针速度交接）。
    pub fn kick(&mut self, velocity: f32) {
        self.v = velocity;
    }

    /// 硬置位：新出现的节点不该从 (0,0) 飞过来。
    pub fn jump(&mut self, x: f32) {
        self.x = x;
        self.target = x;
        self.v = 0.0;
    }

    pub fn is_settled(&self) -> bool {
        (self.x - self.target).abs() < SETTLED_POS && self.v.abs() < SETTLED_VEL
    }

    /// 前进 `dt` 秒。`dt` 会被切成小步：数值稳定，且与帧率无关。
    pub fn step(&mut self, dt: f32) {
        let omega = std::f32::consts::TAU / self.response.max(0.01);
        let stiffness = omega * omega;
        let damping = 2.0 * self.zeta * omega;
        let mut left = dt.clamp(0.0, MAX_FRAME);
        while left > 0.0 {
            let h = left.min(MAX_SUBSTEP);
            left -= h;
            let accel = -stiffness * (self.x - self.target) - damping * self.v;
            // 半隐式欧拉：先速度后位置，阻尼大时也不会发散
            self.v += accel * h;
            self.x += self.v * h;
        }
    }
}
