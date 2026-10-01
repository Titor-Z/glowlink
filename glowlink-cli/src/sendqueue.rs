//! 发送队列：`SEND` 只**入队**，守护进程按顺序一个文件一个文件地发。
//!
//! 为什么队列在守护进程而不是界面里：
//! - 用户一次拖 5 个文件，期望的是「一个接一个」，而不是 5 条连接同时抢带宽；
//! - 界面（窗口）随时可能关掉，而「还没发的那些」不该跟着一起消失
//!   ——「界面是守护进程的前端」这条原则的直接后果。
//!
//! 顺序是**单个 FIFO** 而不是「每台设备一条队列」：一次投放就是一批，
//! 而一台设备同一时刻只会有一个文件在传，多设备并发反而是用户没要求过的行为。

use std::collections::VecDeque;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, Condvar, Mutex};

use crate::transfers::Job;

/// 一件待发的文件。
#[derive(Debug)]
pub struct Item {
    pub addr: SocketAddr,
    /// 对端显示名（日志与卡片都用它）。
    pub peer: String,
    pub path: PathBuf,
    pub job: Job,
}

/// 待发队列（守护进程里只有一份）。
#[derive(Debug, Default)]
pub struct SendQueue {
    inner: Mutex<VecDeque<Item>>,
    ready: Condvar,
}

impl SendQueue {
    pub fn new() -> Self {
        Self::default()
    }

    /// 入队（立刻返回：界面不该等队列）。
    pub fn push(&self, item: Item) {
        self.lock().push_back(item);
        self.ready.notify_one();
    }

    /// 队列里还剩几件（界面/日志用得上）。
    pub fn len(&self) -> usize {
        self.lock().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// 取下一件，没有就阻塞等着（工作线程的循环入口）。
    pub fn wait(&self) -> Item {
        let mut queue = self.lock();
        loop {
            if let Some(item) = queue.pop_front() {
                return item;
            }
            queue = self
                .ready
                .wait(queue)
                .unwrap_or_else(|e| e.into_inner());
        }
    }

    /// 取下一件（不等）。测试与「收尾时顺带清空」用它。
    pub fn try_pop(&self) -> Option<Item> {
        self.lock().pop_front()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, VecDeque<Item>> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// 队列句柄（守护进程持有，工作线程用克隆）。
pub type SharedQueue = Arc<SendQueue>;
