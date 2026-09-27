//! `glowlink-ipc/src/client.rs` 的单元测试。
//!
//! 往返用例必然同时用到服务端骨架（`bind_ipc` / `serve_connection`）——
//! 客户端只有在真连接上才谈得上「响应边界」「错误还原」，
//! 所以这里用真实回环而不是替身。

mod common;

use std::sync::Arc;

use common::{a_device, pending_row, sample_state, FakeHost};

use glowlink_ipc::client::IpcClient;
use glowlink_ipc::server::{bind_ipc, serve_connection};

#[test]
fn client_and_server_talk_over_loopback() {
    let host = Arc::new(FakeHost::default());
    host.devices
        .lock()
        .unwrap()
        .push(a_device("甲机", "192.168.31.37:53500".parse().unwrap()));
    host.pending
        .lock()
        .unwrap()
        .push(pending_row("001", "192.168.31.9", "报告.pdf", 2048));

    let (listener, port) = bind_ipc(0).expect("应能绑到一个回环端口");
    let server_host = Arc::clone(&host);
    let server = std::thread::spawn(move || {
        for stream in listener.incoming().take(1) {
            serve_connection(server_host.as_ref(), stream.unwrap());
        }
    });

    let mut state = sample_state();
    state.ipc_port = port;
    let mut client = IpcClient::connect(&state).expect("应能连上守护进程");
    // 一条连接连发多条指令：验证响应边界（END）没串行错位
    assert!(client.call_first("PING").unwrap().starts_with("OK 0.3.0"));
    assert_eq!(client.call("STATUS").unwrap()[0].split(' ').nth(2), Some("1"));
    assert_eq!(client.call("DEVICES").unwrap().len(), 2);
    assert_eq!(client.call("PENDING").unwrap().len(), 2);

    let reply = client.call_first("ACCEPT 001").unwrap();
    assert!(reply.contains("#001"), "实际：{reply}");
    assert_eq!(host.decided.lock().unwrap().len(), 1);

    // 错误响应要能被客户端识别成错误
    let err = client.call_first("RESOLVE 幽灵机").unwrap_err().to_string();
    assert!(err.contains("幽灵机"), "实际：{err}");

    // 必须先关客户端：服务端是「读到对端关闭才退出」的循环，
    // 否则 join 会永远等下去（CI 上就是这么挂住 30 分钟的）。
    drop(client);
    server.join().unwrap();
}

#[test]
fn client_fails_fast_when_nothing_listens() {
    // 找一个没人听的端口
    let (listener, port) = bind_ipc(0).unwrap();
    drop(listener);
    let mut state = sample_state();
    state.ipc_port = port;
    assert!(IpcClient::connect(&state).is_err(), "没人监听时必须报错而不是挂住");
}
