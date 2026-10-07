//! `glowlink-cli/src/keys.rs` 的单元测试：身份密钥文件、信任列表。
//!
//! 全部指向临时目录，不碰用户真实的 `~/.glowlink`。

use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;

use glowlink_cli::keys::{load_or_create_identity_at, TrustFile};
use glowlink_core::patterns::secure::TrustList;

/// 这些用例会改进程级 `HOME`，必须串行（否则互相把 HOME 改到别的目录）。
static HOME_LOCK: Mutex<()> = Mutex::new(());

fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("glowlink-keys-{tag}"));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn identity_is_created_once_and_reloaded() {
    let _guard = HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let dir = temp_dir("identity");
    let path = dir.join("identity.txt");

    let first = load_or_create_identity_at(&path).unwrap();
    assert!(path.is_file(), "首次调用应生成身份文件");

    let second = load_or_create_identity_at(&path).unwrap();
    assert_eq!(
        first.fingerprint(),
        second.fingerprint(),
        "再次启动必须复用同一身份，否则每次运行都要重新配对"
    );
    assert_eq!(first.public_bytes(), second.public_bytes());
}

#[test]
fn identity_file_is_private_on_unix() {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _guard = HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = temp_dir("perms");
        let path = dir.join("identity.txt");
        load_or_create_identity_at(&path).unwrap();

        let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "私钥文件必须是 0600，实际 {:o}", mode);
    }
}

#[test]
fn identity_file_rejects_tampering() {
    let _guard = HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let dir = temp_dir("tamper");
    let path = dir.join("identity.txt");
    load_or_create_identity_at(&path).unwrap();

    // 把公钥改成别的值 ⇒ 必须报错，而不是默默用一个对不上的身份
    let text = fs::read_to_string(&path).unwrap();
    let broken = text
        .lines()
        .map(|line| {
            if line.starts_with("public=") {
                "public=0000000000000000000000000000000000000000000000000000000000000000"
            } else {
                line
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    fs::write(&path, broken).unwrap();
    assert!(load_or_create_identity_at(&path).is_err(), "公私钥不匹配必须报错");

    // 缺 secret ⇒ 报错
    fs::write(&path, "# 空的\npublic=00\n").unwrap();
    assert!(load_or_create_identity_at(&path).is_err());
}

#[test]
fn trust_list_starts_empty_and_persists() {
    let dir = temp_dir("trust");
    let path = dir.join("trusted.txt");
    let trust = TrustFile::open(path.clone()).unwrap();
    assert!(trust.devices().is_empty());
    assert!(!trust.is_trusted("abcdef0123456789"));

    trust.trust("abcdef0123456789", "甲机").unwrap();
    assert!(trust.is_trusted("abcdef0123456789"));
    assert_eq!(trust.name_of("abcdef0123456789").as_deref(), Some("甲机"));
    assert!(path.is_file(), "配对后必须落盘");

    // 重新打开：内容还在
    let reopened = TrustFile::open(path).unwrap();
    assert_eq!(reopened.devices().len(), 1);
    assert!(reopened.is_trusted("abcdef0123456789"));
    assert_eq!(reopened.name_of("abcdef0123456789").as_deref(), Some("甲机"));
}

#[test]
fn trusting_twice_updates_name_without_duplicating() {
    let dir = temp_dir("trust-twice");
    let trust = TrustFile::open(dir.join("trusted.txt")).unwrap();
    trust.trust("aaaa000011112222", "旧名字").unwrap();
    trust.trust("aaaa000011112222", "新名字").unwrap();

    let devices = trust.devices();
    assert_eq!(devices.len(), 1, "同一指纹只能有一条");
    assert_eq!(devices[0].name, "新名字");
}

#[test]
fn forget_removes_and_reports() {
    let dir = temp_dir("forget");
    let path = dir.join("trusted.txt");
    let trust = TrustFile::open(path.clone()).unwrap();
    trust.trust("aaaa000011112222", "甲机").unwrap();
    trust.trust("bbbb333344445555", "乙机").unwrap();

    assert!(trust.forget("aaaa000011112222").unwrap(), "删存在的应返回 true");
    assert!(!trust.forget("aaaa000011112222").unwrap(), "再删应返回 false");
    assert_eq!(trust.devices().len(), 1);
    // 另一个设备不受影响
    assert!(trust.is_trusted("bbbb333344445555"));

    let reopened = TrustFile::open(path).unwrap();
    assert_eq!(reopened.devices().len(), 1);
}

#[test]
fn rename_updates_existing_entry_and_never_creates_one() {
    // 改名刻意不复用 trust()：那会把改名变成「凭空信任一台陌生设备」
    let dir = temp_dir("rename");
    let trust = TrustFile::open(dir.join("trusted.txt")).unwrap();
    trust.trust("aaaa000011112222", "旧名字").unwrap();

    assert!(trust.rename("aaaa000011112222", "新名字").unwrap());
    let devices = trust.devices();
    assert_eq!(devices.len(), 1);
    assert_eq!(devices[0].name, "新名字");

    assert!(!trust.rename("bbbb333344445555", "陌生机").unwrap(), "改不存在的条目应返回 false");
    assert_eq!(trust.devices().len(), 1, "名单不该多出条目");
    assert!(!trust.is_trusted("bbbb333344445555"), "改名不许凭空信任");
}

#[test]
fn trust_file_tolerates_broken_lines() {
    let dir = temp_dir("broken");
    let path = dir.join("trusted.txt");
    // 手改坏了的文件：空行、注释、缺字段、指纹不合法、时间不合法
    fs::write(
        &path,
        "# 注释\n\
         \nabcdef0123456789\t甲机\t123\n\
         1122334455667788\t乙机\tnot-a-number\n\
         只有指纹没有其它字段\n\
         \t没指纹\t1\n\
         zz\t坏指纹\t1\n",
    )
    .unwrap();

    let trust = TrustFile::open(path).unwrap();
    let devices = trust.devices();
    assert_eq!(devices.len(), 2, "坏行跳过，好行保留：{devices:?}");
    assert_eq!(devices[0].fingerprint, "abcdef0123456789");
    assert_eq!(devices[0].paired_at, 123);
    assert_eq!(devices[1].fingerprint, "1122334455667788");
    assert_eq!(devices[1].paired_at, 0, "坏时间退化为 0 而不是丢整条");

    // 关键：坏行不能凭空造出一台「已信任设备」
    assert!(!trust.is_trusted("没指纹"), "缺指纹的行不能被当成信任项");
    assert!(!trust.is_trusted("zz"), "指纹必须合法");
    assert!(!trust.is_trusted("只有指纹没有其它字段"));
    assert_eq!(trust.devices().len(), 2);
}

#[test]
fn identity_path_can_be_overridden_by_env() {
    // 同机模拟两台设备：两个进程用不同的身份文件
    let _guard = HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let dir = temp_dir("env-identity");
    let custom = dir.join("another-identity.txt");
    std::env::set_var("GLOWLINK_IDENTITY", &custom);

    assert_eq!(glowlink_cli::keys::identity_path(), custom);

    let identity = load_or_create_identity_at(&glowlink_cli::keys::identity_path()).unwrap();
    assert!(custom.is_file(), "应写到覆盖后的路径");

    // 与默认路径下的身份互不影响
    let default_path = dir.join("identity.txt");
    let other = load_or_create_identity_at(&default_path).unwrap();
    assert_ne!(
        identity.fingerprint(),
        other.fingerprint(),
        "两份身份文件应当是两个不同的设备身份"
    );

    std::env::remove_var("GLOWLINK_IDENTITY");
    assert!(glowlink_cli::keys::identity_path().ends_with(".glowlink/identity.txt"));
}

#[test]
fn identity_and_trust_use_distinct_files() {
    let _guard = HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let dir = temp_dir("paths");
    // 直接检查默认路径的构成（不改 HOME，只看文件名与目录）
    let _ = dir;
    assert!(glowlink_cli::keys::identity_path().ends_with(".glowlink/identity.txt"));
    assert!(glowlink_cli::keys::trust_path().ends_with(".glowlink/trusted.txt"));
}

/// 真跑二进制两次：本机 ID 必须**跨运行一致**，且与配对指纹同源。
///
/// 这条用例同时盯着一个启动顺序问题：ID 是在 `main()` 第一句注入的，
/// 一旦有人把注入挪到 `DeviceIdentity::global()` 之后，两次运行的 ID 就会不同。
#[test]
fn local_id_is_stable_across_runs_and_matches_the_fingerprint() {
    let home = temp_dir("stable-id");
    let identity = home.join("identity.txt");
    let bin = env!("CARGO_BIN_EXE_glowlink");

    let run = |args: &[&str]| {
        let output = std::process::Command::new(bin)
            // 子进程用自己的 HOME：不碰真实 ~/.glowlink，也不需要与别的用例串行
            .env("HOME", &home)
            .env("GLOWLINK_IDENTITY", &identity)
            .args(args)
            .output()
            .expect("应当能启动 glowlink");
        String::from_utf8_lossy(&output.stdout).to_string()
    };

    // 只在「本机：」那一行里找括号：标题里也有括号，全局找第一个会抓错行
    let local_id = |text: &str| {
        let line = text
            .lines()
            .find(|line| line.contains("本机："))
            .expect("面板应打印本机信息");
        let start = line.find('（').expect("本机信息应带括号") + '（'.len_utf8();
        let rest = &line[start..];
        rest[..rest.find('）').expect("本机信息应带括号")].to_string()
    };

    let first = local_id(&run(&[]));
    let second = local_id(&run(&[]));
    assert_eq!(first, second, "本机 ID 跨运行必须一致：{first} vs {second}");
    assert_eq!(first.len(), 8, "面板显示 8 位十六进制：{first}");

    // 与配对指纹同源：面板 ID 必须是指纹的前缀（同一套身份，只是显示长度不同）
    let fingerprint = load_or_create_identity_at(&identity)
        .expect("身份应能加载")
        .fingerprint();
    assert!(
        fingerprint.starts_with(&first),
        "面板 ID {first} 应当是指纹 {fingerprint} 的前缀"
    );

    // 用户可读路径也要能拿到完整指纹（空信任列表时同样打印）
    let trust = run(&["trust"]);
    assert!(
        trust.contains(&fingerprint),
        "glowlink trust 应当打印本机指纹：{trust}"
    );
}
