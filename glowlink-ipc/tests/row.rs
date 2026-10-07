//! `glowlink-ipc/src/row.rs` 的单元测试：数据行的线上格式（跨进程契约）。

use glowlink_ipc::row::{
    parse_transfers, DeviceRow, PairingRow, PairingState, PendingRow, TransferDirection,
    TransferRow, TransferState, TrustRow,
};
use glowlink_core::model::packet::DeviceKind;
use glowlink_core::{Device, DeviceId};

use std::time::Instant;

#[test]
fn device_row_roundtrip_preserves_every_field() {
    let line = "甲机\tabc123def4567890\t192.168.31.37:53500\t1234\tlaptop\t1";
    let row = DeviceRow::from_line(line).expect("标准行必须能解析");
    assert_eq!(row.name, "甲机");
    assert_eq!(row.id, "abc123def4567890");
    assert_eq!(row.addr.to_string(), "192.168.31.37:53500");
    assert_eq!(row.age_ms, 1234);
    assert_eq!(row.kind, DeviceKind::Laptop);
    assert!(row.trusted);
    assert_eq!(row.to_line(), line, "解析再编码必须逐字节相同");
}

#[test]
fn device_row_without_trusted_column_reads_as_untrusted() {
    // 信任列同样是后加的：缺了就当**未信任**。
    // 猜「已信任」等于替用户省掉核对校验码，那是唯一不能省的一步。
    let row = DeviceRow::from_line("甲机\tabc\t127.0.0.1:1\t1\tdesktop").unwrap();
    assert!(!row.trusted);
    assert!(!DeviceRow::from_line("甲机\tabc\t127.0.0.1:1\t1").unwrap().trusted);
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

#[test]
fn transfer_row_round_trips_through_the_wire() {
    let row = TransferRow {
        code: String::new(),
        token: "001".into(),
        direction: TransferDirection::Send,
        name: "屏幕录制 2026.mp4".into(),
        peer: "甲机".into(),
        done: 1234,
        total: 842_400_000,
        state: TransferState::Running,
        note: String::new(),
    };
    let back = TransferRow::from_line(&row.to_line()).expect("数据行要能被客户端解析");
    assert_eq!(back, row);
    assert_eq!(back.direction.as_token(), "send");
}

#[test]
fn transfer_row_note_is_optional_but_the_rest_is_not() {
    // note 是后加的列：老服务端没有它，认成空串而不是整行丢掉
    let row = TransferRow::from_line("002\trecv\t报告.pdf\t乙机\t9\t10\tdone").unwrap();
    assert_eq!(row.note, "");
    assert_eq!(row.state, TransferState::Done);
    assert_eq!(row.direction, TransferDirection::Receive);
    // 前 7 列缺一个就是坏行
    assert!(TransferRow::from_line("002\trecv\t报告.pdf\t乙机\t9\t10").is_none());
}

#[test]
fn unknown_transfer_tokens_fall_back_to_the_cautious_reading() {
    // 方向认不出按「发送」：宁可多显示一个方向，也不要因为一个陌生词丢掉整条进度。
    // 状态认不出按「进行中」：不能把没跑完的传输显示成已完成。
    let row = TransferRow::from_line("003\tweird\tx.bin\t甲机\t1\t2\tweird").unwrap();
    assert_eq!(row.direction, TransferDirection::Send);
    assert_eq!(row.state, TransferState::Running);
}

#[test]
fn parse_transfers_skips_the_count_line() {
    let lines = vec![
        "OK 2".to_string(),
        "001\tsend\ta.bin\t甲机\t5\t10\trunning\t".to_string(),
        "002\trecv\tb.bin\t乙机\t10\t10\tdone\t".to_string(),
    ];
    let rows = parse_transfers(&lines);
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].name, "a.bin");
    assert_eq!(rows[1].state, TransferState::Done);
}

#[test]
fn canceled_is_its_own_state_and_survives_a_round_trip() {
    // 「已取消」不能混进「失败」：一个是用户主动掐掉，一个是真出故障，
    // 界面文案与用户的下一步动作都不一样（前者别再重试，后者要排查）。
    let row = TransferRow {
        code: String::new(),
        token: "004".into(),
        direction: TransferDirection::Send,
        name: "big.bin".into(),
        peer: "iPad mini".into(),
        done: 3 * 1024,
        total: 40 * 1024,
        state: TransferState::Canceled,
        note: "已取消".into(),
    };
    let back = TransferRow::from_line(&row.to_line()).unwrap();
    assert_eq!(back, row);
    assert_eq!(back.state, TransferState::Canceled);
    assert_ne!(back.state, TransferState::Failed);
}

#[test]
fn abandoned_pairing_is_its_own_state() {
    // 「本机已放弃」进协议：与「被拒」「失败」分开，界面文案才说得准
    for state in [
        PairingState::WaitingPeer,
        PairingState::WaitingUser,
        PairingState::Paired,
        PairingState::Denied,
        PairingState::Abandoned,
        PairingState::Failed,
    ] {
        assert_eq!(PairingState::from_token(state.as_token()), state, "{state:?} 往返失败");
    }
    let row = PairingRow {
        token: "003".into(),
        peer: "iPad mini".into(),
        code: "123456".into(),
        state: PairingState::Abandoned,
        remaining: 0,
    };
    assert_eq!(PairingRow::from_line(&row.to_line()).unwrap(), row);
}

#[test]
fn trust_row_roundtrips_with_spaces_in_name() {
    // 名字是设备名常态（可能带空格）：`\t` 分列不受影响
    let row = TrustRow {
        fingerprint: "aaaa000011112222".into(),
        name: "我 的 甲机".into(),
        paired_at: 1_700_000_000,
    };
    let parsed = TrustRow::from_line(&row.to_line()).expect("要能还原");
    assert_eq!(parsed, row);
}

#[test]
fn trust_row_tolerates_missing_name_and_time() {
    // 旧手改的行可能没写名字与时间：缺了就当空 / 0，指纹在就行
    let parsed = TrustRow::from_line("aaaa000011112222").expect("要能还原");
    assert_eq!(parsed.fingerprint, "aaaa000011112222");
    assert_eq!(parsed.name, "");
    assert_eq!(parsed.paired_at, 0);
}
