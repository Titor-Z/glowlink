//! `glowlink-ipc/src/row.rs` 的单元测试：数据行的线上格式（跨进程契约）。

use glowlink_ipc::row::{DeviceRow, PendingRow};
use glowlink_core::model::packet::DeviceKind;
use glowlink_core::{Device, DeviceId};

use std::time::Instant;

#[test]
fn device_row_roundtrip_preserves_every_field() {
    let line = "甲机\tabc123def4567890\t192.168.31.37:53500\t1234\tlaptop";
    let row = DeviceRow::from_line(line).expect("标准行必须能解析");
    assert_eq!(row.name, "甲机");
    assert_eq!(row.id, "abc123def4567890");
    assert_eq!(row.addr.to_string(), "192.168.31.37:53500");
    assert_eq!(row.age_ms, 1234);
    assert_eq!(row.kind, DeviceKind::Laptop);
    assert_eq!(row.to_line(), line, "解析再编码必须逐字节相同");
}

#[test]
fn device_row_without_kind_column_reads_as_unknown() {
    // 类型是后加的列：老守护进程的 DEVICES 行只有四个字段，
    // 客户端不能因此把整行丢掉（丢掉就是「看不见设备」）。
    let row = DeviceRow::from_line("甲机\tabc123def4567890\t192.168.31.37:53500\t1234").unwrap();
    assert_eq!(row.kind, DeviceKind::Unknown);
    assert_eq!(row.name, "甲机");

    // 认不出的类型名同样归「未知」，不报错
    let row = DeviceRow::from_line("甲机\tabc\t127.0.0.1:1\t1\thologram").unwrap();
    assert_eq!(row.kind, DeviceKind::Unknown);
}

#[test]
fn every_kind_has_a_readable_token() {
    for (kind, token) in [
        (DeviceKind::Unknown, "unknown"),
        (DeviceKind::Phone, "phone"),
        (DeviceKind::Tablet, "tablet"),
        (DeviceKind::Desktop, "desktop"),
        (DeviceKind::Laptop, "laptop"),
    ] {
        assert_eq!(kind.as_token(), token);
        assert_eq!(DeviceKind::from_token(token), kind, "token 必须能往返");
    }
}

#[test]
fn device_row_rejects_broken_lines() {
    // 少字段、地址坏掉、时长不是数字：都算坏行，交给调用方跳过
    assert!(DeviceRow::from_line("甲机\tabc\t192.168.31.37:53500").is_none());
    assert!(DeviceRow::from_line("甲机\tabc\t不是地址\t1").is_none());
    assert!(DeviceRow::from_line("甲机\tabc\t127.0.0.1:1\t昨天").is_none());
}

#[test]
fn device_row_carries_age_not_timestamp() {
    // 存活时长是相对的：两端时钟不同源，传绝对时间没有可比性
    let device = Device {
        id: DeviceId::new("abc123def4567890"),
        name: "甲机".into(),
        addr: "127.0.0.1:9911".parse().unwrap(),
        last_seen: Instant::now(),
        kind: DeviceKind::Laptop,
    };
    let row = DeviceRow::from_device(&device);
    assert!(row.age_ms < 1_000, "刚见过的设备存活时长应在毫秒级：{}", row.age_ms);
    assert_eq!(row.kind, DeviceKind::Laptop, "类型要跟着设备走");

    // 还原回去也必须落在「刚刚」
    let restored = row.to_device();
    assert_eq!(restored.name, "甲机");
    assert_eq!(restored.kind, DeviceKind::Laptop);
    assert!(restored.last_seen.elapsed().as_secs() < 1);
}

#[test]
fn pending_row_roundtrip_preserves_every_field() {
    let line = "001\t192.168.31.9\t报告.pdf\t2048\tabcdef0123456789\t482913";
    let row = PendingRow::from_line(line).expect("标准行必须能解析");
    assert_eq!(row.token, "001");
    assert_eq!(row.peer, "192.168.31.9");
    assert_eq!(row.name, "报告.pdf");
    assert_eq!(row.size, 2048);
    assert_eq!(row.fingerprint, "abcdef0123456789");
    assert_eq!(row.code, "482913");
    assert_eq!(row.to_line(), line);
}

#[test]
fn pending_row_tolerates_missing_fingerprint_and_code() {
    // 老守护进程不带这两列：少它们算「未配对」，不该整行丢掉
    let row = PendingRow::from_line("001\t192.168.31.9\t报告.pdf\t2048").unwrap();
    assert_eq!(row.fingerprint, "");
    assert_eq!(row.code, "");
    // 但 token / 来源 / 文件名 / 大小缺一个就是坏行
    assert!(PendingRow::from_line("001\t192.168.31.9\t报告.pdf").is_none());
}

#[test]
fn file_name_with_spaces_and_tabs_survives() {
    // 文件名里带空格很常见；制表符是分隔符，不该出现在字段里（出现即为协议错）
    let row = PendingRow::from_line("001\t甲机\t屏幕录制 2026-09-15 040220.mp4\t842400000\tf\t123456")
        .unwrap();
    assert_eq!(row.name, "屏幕录制 2026-09-15 040220.mp4");
    assert_eq!(row.size, 842_400_000);
}
