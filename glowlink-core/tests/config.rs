use glowlink_core::model::error::GlowError;
use glowlink_core::model::ports::DISCOVERY_PORT_SPAN;
use glowlink_core::Config;

fn config(text: &str) -> Config {
    Config::parse(text).expect("配置应能解析")
}

fn config_result(text: &str) -> Result<Config, GlowError> {
    Config::parse(text)
}

#[test]
fn defaults_match_shipped_config() {
    let default = Config::default();
    // 端口以 packet.rs 的协议常量为唯一来源，不在配置里另写一份。
    assert_eq!(default.discovery_port, glowlink_core::DISCOVERY_PORT);
    assert_eq!(default.transfer_port, 0);
    assert_eq!(default.receive_dir.to_string_lossy(), "./received");
    assert_eq!(default.chunk_size_kib, 64);
    assert_eq!(default.ring_capacity_kib, 1024);
    assert_eq!(
        default.pending_timeout_secs, 300,
        "等待人工确认的上限默认给 5 分钟：人可能离开一会儿"
    );

    // 随工程分发的 glowlink.toml 应与内置默认一致，避免两处数值漂移。
    let shipped = include_str!("../../glowlink.toml");
    assert_eq!(config(shipped), default, "glowlink.toml 与内置默认不一致");
}

#[test]
fn parses_values_and_ignores_noise() {
    let parsed = config(
        "# 注释行\n\
         \n\
         discovery_port = 9000   # 行内注释\n\
         receive_dir = \"/tmp/glowlink\"  # 带引号\n\
         chunk_size_kib = 128\n",
    );
    assert_eq!(parsed.discovery_port, 9000);
    assert_eq!(parsed.receive_dir.to_string_lossy(), "/tmp/glowlink");
    assert_eq!(parsed.chunk_size_kib, 128);
    assert_eq!(parsed.transfer_port, 0, "未出现的键应保持默认");
}

#[test]
fn hash_inside_quotes_is_not_a_comment() {
    let parsed = config("receive_dir = \"/tmp/a#b\"\n");
    assert_eq!(parsed.receive_dir.to_string_lossy(), "/tmp/a#b");
}

#[test]
fn apply_overlays_only_present_keys() {
    let mut base = config("discovery_port = 9000\ntransfer_port = 4000\n");
    base.apply("transfer_port = 5000\n").unwrap();
    assert_eq!(base.discovery_port, 9000, "未出现的键不应被重置");
    assert_eq!(base.transfer_port, 5000);
}

#[test]
fn unknown_key_is_rejected_with_suggestion() {
    let error = Config::parse("discovery_prot = 9000\n").unwrap_err();
    let message = error.to_string();
    assert!(matches!(error, GlowError::Config(_)));
    assert!(message.contains("第 1 行"), "应指出行号：{message}");
    assert!(message.contains("discovery_prot"), "应指出非法键名：{message}");
    assert!(message.contains("discovery_port"), "应给出可用键名：{message}");
}

#[test]
fn invalid_number_is_rejected() {
    let error = Config::parse("discovery_port = 99999\n").unwrap_err();
    assert!(error.to_string().contains("discovery_port"), "{error}");
    let error = Config::parse("chunk_size_kib = 很大\n").unwrap_err();
    assert!(error.to_string().contains("第 1 行"), "{error}");
}

#[test]
fn missing_equals_and_sections_are_rejected() {
    assert!(Config::parse("discovery_port 8888\n").is_err());
    assert!(Config::parse("[network]\ndiscovery_port = 8888\n").is_err());
}

#[test]
fn sizes_convert_to_bytes() {
    let parsed = config("chunk_size_kib = 8\nring_capacity_kib = 32\n");
    assert_eq!(parsed.chunk_size(), 8 * 1024);
    assert_eq!(parsed.ring_capacity(), 32 * 1024);
}

/// 候选端口个数：可解析，且越界必须报错并指出允许范围。
#[test]
fn discovery_port_span_is_parsed_and_bounded() {
    let parsed = config("discovery_port_span = 5");
    assert_eq!(parsed.discovery_port_span, 5);

    // 0 等于「只发约定端口」，顺延过的节点就再也找不到了，必须拦住
    let zero = config_result("discovery_port_span = 0").unwrap_err().to_string();
    assert!(zero.contains("discovery_port_span"), "错误要指出是哪个键：{zero}");
    assert!(zero.contains("1..=64"), "错误要给出允许范围：{zero}");

    let too_big = config_result("discovery_port_span = 65").unwrap_err().to_string();
    assert!(too_big.contains("1..=64"), "错误要给出允许范围：{too_big}");
}

/// 默认值与协议常量同源：默认必须能覆盖协议约定的顺延范围。
#[test]
fn default_span_matches_protocol_constant() {
    let parsed = config("");
    assert_eq!(parsed.discovery_port_span, DISCOVERY_PORT_SPAN);
    assert_eq!(
        parsed.discovery_port_span,
        glowlink_core::model::ports::candidate_ports(9267, DISCOVERY_PORT_SPAN).len() as u16,
        "跨度与实际探测的端口个数必须一致"
    );
}
