//! 环形缓冲池：字节环 + 跨线程阻塞封装，用于实现背压控制。
//!
//! 背压策略：生产者写满即阻塞（`SharedRing::push_all`），消费者按需取走
//! （`SharedRing::pop_into`），因此磁盘读取速度会自动匹配网络写入速度，
//! 内存占用恒定为环容量上限。

use std::sync::{Condvar, Mutex, MutexGuard};

/// 纯数据结构的字节环形缓冲，写满时少写而非覆盖。
#[derive(Debug)]
pub struct RingBuffer {
    buf: Vec<u8>,
    head: usize,
    len: usize,
}

impl RingBuffer {
    pub fn new(capacity: usize) -> Self {
        assert!(capacity > 0, "环形缓冲容量必须大于 0");
        RingBuffer { buf: vec![0; capacity], head: 0, len: 0 }
    }

    pub fn capacity(&self) -> usize {
        self.buf.len()
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn free(&self) -> usize {
        self.capacity() - self.len
    }

    /// 写入数据，返回实际写入字节数（空间不足时只写入一部分）。
    pub fn push(&mut self, data: &[u8]) -> usize {
        let n = data.len().min(self.free());
        if n == 0 {
            return 0;
        }
        let cap = self.capacity();
        let tail = (self.head + self.len) % cap;
        let first = (cap - tail).min(n);
        self.buf[tail..tail + first].copy_from_slice(&data[..first]);
        if n > first {
            self.buf[..n - first].copy_from_slice(&data[first..n]);
        }
        self.len += n;
        n
    }

    /// 读出数据，返回实际读出字节数。
    pub fn pop(&mut self, out: &mut [u8]) -> usize {
        let n = out.len().min(self.len);
        if n == 0 {
            return 0;
        }
        let cap = self.capacity();
        let first = (cap - self.head).min(n);
        out[..first].copy_from_slice(&self.buf[self.head..self.head + first]);
        if n > first {
            out[first..n].copy_from_slice(&self.buf[..n - first]);
        }
        self.head = (self.head + n) % cap;
        self.len -= n;
        n
    }
}

struct State {
    ring: RingBuffer,
    closed: bool,
}

/// 线程安全的阻塞环形缓冲。
pub struct SharedRing {
    inner: Mutex<State>,
    not_full: Condvar,
    not_empty: Condvar,
}

impl SharedRing {
    pub fn new(capacity: usize) -> Self {
        SharedRing {
            inner: Mutex::new(State { ring: RingBuffer::new(capacity), closed: false }),
            not_full: Condvar::new(),
            not_empty: Condvar::new(),
        }
    }

    pub fn capacity(&self) -> usize {
        self.lock().ring.capacity()
    }

    /// 关闭后：生产者立刻返回，消费者排空剩余数据后返回 0。
    pub fn close(&self) {
        let mut state = self.lock();
        state.closed = true;
        drop(state);
        self.not_full.notify_all();
        self.not_empty.notify_all();
    }

    pub fn is_closed(&self) -> bool {
        self.lock().closed
    }

    /// 阻塞写入全部数据；被关闭时返回已写入的字节数。
    pub fn push_all(&self, data: &[u8]) -> usize {
        let mut written = 0;
        while written < data.len() {
            let mut state = self.lock();
            while state.ring.free() == 0 && !state.closed {
                state = self.wait(&self.not_full, state);
            }
            if state.closed {
                break;
            }
            written += state.ring.push(&data[written..]);
            drop(state);
            self.not_empty.notify_all();
        }
        written
    }

    /// 阻塞等待数据；被关闭且已排空时返回 0。
    pub fn pop_into(&self, out: &mut [u8]) -> usize {
        let mut state = self.lock();
        loop {
            if state.ring.len() > 0 {
                let n = state.ring.pop(out);
                drop(state);
                self.not_full.notify_all();
                return n;
            }
            if state.closed {
                return 0;
            }
            state = self.wait(&self.not_empty, state);
        }
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn wait<'a>(&self, condvar: &Condvar, state: MutexGuard<'a, State>) -> MutexGuard<'a, State> {
        condvar.wait(state).unwrap_or_else(|e| e.into_inner())
    }
}
