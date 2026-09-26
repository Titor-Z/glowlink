use glowlink_cli::view::cli::{human_duration, human_size, human_speed};

#[test]
fn human_size_switches_units() {
    assert_eq!(human_size(0), "0 B");
    assert_eq!(human_size(999), "999 B");
    assert_eq!(human_size(1024), "1.0 KB");
    assert_eq!(human_size(1024 * 1024), "1.0 MB");
    assert_eq!(human_size(3 * 1024 * 1024 * 1024), "3.0 GB");
}

#[test]
fn human_speed_always_uses_megabytes() {
    assert_eq!(human_speed(1024.0 * 1024.0), "1.0 MB/s");
    assert_eq!(human_speed(0.0), "0.0 MB/s");
    // 千兆网线量级，避免出现 "0.1 GB/s" 这种不好比较的单位
    assert_eq!(human_speed(112.0 * 1024.0 * 1024.0), "112.0 MB/s");
}

#[test]
fn human_duration_picks_scale() {
    assert_eq!(human_duration(0), "0s");
    assert_eq!(human_duration(45), "45s");
    assert_eq!(human_duration(60), "1m00s");
    assert_eq!(human_duration(125), "2m05s");
    assert_eq!(human_duration(3600), "1h00m");
    assert_eq!(human_duration(3990), "1h06m");
}
