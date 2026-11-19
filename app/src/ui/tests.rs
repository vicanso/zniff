use super::*;

use std::net::Ipv4Addr;

use super::{LanAddr, cert_url, choose_lan, is_listening_lan, usable_v4};
use crate::i18n::Lang;
use crate::session::Protocol;

#[test]
fn is_listening_lan_only_matches_unspecified_ipv4() {
    assert!(is_listening_lan("0.0.0.0:8877"));
    assert!(!is_listening_lan("127.0.0.1:8877"));
    assert!(!is_listening_lan(""));
}

#[test]
fn protocol_tunnel_label_is_localized() {
    assert_eq!(Protocol::Tunnel.label_in(Lang::Zh), "隧道");
    assert_eq!(Protocol::Tunnel.label_in(Lang::En), "Tunnel");
    assert_eq!(Protocol::WebSocket.label_in(Lang::En), "WebSocket");
    assert_eq!(Protocol::Http2.label_in(Lang::En), "HTTP/2");
}

#[test]
fn cert_url_uses_only_the_chosen_lan_address() {
    let ip = Ipv4Addr::new(192, 168, 1, 8);
    assert_eq!(cert_url("127.0.0.1:8877", Some(ip)), "");
    assert_eq!(cert_url("0.0.0.0:8877", None), "");
    assert_eq!(cert_url("0.0.0.0:0", Some(ip)), "");
    assert_eq!(
        cert_url("0.0.0.0:8877", Some(ip)),
        "http://192.168.1.8:8877/zniff"
    );
}

#[test]
fn choose_lan_keeps_a_present_choice_then_the_routed_address() {
    let wifi = LanAddr {
        name: "en0".into(),
        ip: Ipv4Addr::new(192, 168, 1, 8),
    };
    let vpn = LanAddr {
        name: "utun3".into(),
        ip: Ipv4Addr::new(10, 8, 0, 2),
    };
    let bridge = LanAddr {
        name: "bridge100".into(),
        ip: Ipv4Addr::new(192, 168, 64, 1),
    };
    let addrs = [vpn.clone(), bridge, wifi.clone()];
    assert_eq!(
        choose_lan(&addrs, Some(vpn.ip), "192.168.1.8").map(|addr| addr.ip),
        Some(wifi.ip)
    );
    assert_eq!(
        choose_lan(&addrs, Some(vpn.ip), "10.8.0.2").map(|addr| addr.ip),
        Some(vpn.ip)
    );
    assert_eq!(
        choose_lan(&addrs, Some(vpn.ip), "").map(|addr| addr.ip),
        Some(wifi.ip)
    );
    assert_eq!(
        choose_lan(&addrs, Some(vpn.ip), "10.1.1.1").map(|addr| addr.ip),
        Some(wifi.ip)
    );
    assert_eq!(
        choose_lan(&[], Some(wifi.ip), "").map(|addr| addr.ip),
        Some(wifi.ip)
    );
    assert!(choose_lan(&[], Some(Ipv4Addr::LOCALHOST), "").is_none());
    assert!(choose_lan(&[], Some(Ipv4Addr::new(198, 18, 0, 1)), "").is_none());
}

#[test]
fn usable_v4_skips_addresses_a_phone_cannot_open() {
    assert!(usable_v4(Ipv4Addr::new(192, 168, 1, 8)));
    assert!(usable_v4(Ipv4Addr::new(10, 8, 0, 2)));
    assert!(!usable_v4(Ipv4Addr::UNSPECIFIED));
    assert!(!usable_v4(Ipv4Addr::LOCALHOST));
    assert!(!usable_v4(Ipv4Addr::new(169, 254, 1, 1)));
    assert!(!usable_v4(Ipv4Addr::BROADCAST));
}

#[test]
fn filter_chips_toggle_a_whole_token() {
    assert_eq!(
        super::toggle_filter_chip("", "type:json", super::JSON_ALIASES),
        "type:json"
    );
    assert_eq!(
        super::toggle_filter_chip("api.test type:json", "type:json", super::JSON_ALIASES),
        "api.test"
    );
    assert_eq!(
        super::toggle_filter_chip("CT:JSON pin:", "type:json", super::JSON_ALIASES),
        "pin:"
    );
    assert_eq!(
        super::toggle_filter_chip("fail example", "失败", super::FAIL_ALIASES),
        "example"
    );
    assert_eq!(
        super::toggle_filter_chip("example", "失败", super::FAIL_ALIASES),
        "example 失败"
    );
    assert_eq!(
        super::toggle_filter_chip("pin: slow:", "slow:", super::SLOW_ALIASES),
        "pin:"
    );
    assert_eq!(
        super::toggle_filter_chip("", "status:4xx|5xx|err", super::ERR_ALIASES),
        "status:4xx|5xx|err"
    );
    assert_eq!(
        super::toggle_filter_chip(
            "slow: 4xx|5xx|err",
            "status:4xx|5xx|err",
            super::ERR_ALIASES
        ),
        "slow:"
    );
    assert!(!super::filter_has_token("type:jsonl", super::JSON_ALIASES));
    assert!(super::filter_has_token("pin:", super::PIN_ALIASES));
    assert!(super::filter_has_token(
        "status:4xx|5xx|err",
        super::ERR_ALIASES
    ));
}

#[test]
fn saved_filters_keep_the_newest_sixteen_without_duplicates() {
    let saved: Vec<String> = (0..16).map(|n| format!("f{n}")).collect();
    let next = super::push_saved_filter(&saved, "fresh");
    assert_eq!(next.len(), 16);
    assert_eq!(next[0], "fresh");
    assert!(!next.iter().any(|item| item == "f15"));
    assert_eq!(
        super::push_saved_filter(&["a".into()], "  "),
        vec!["a".to_string()]
    );
    assert_eq!(
        super::push_saved_filter(&["a".into(), "b".into()], "b"),
        vec!["b".to_string(), "a".to_string()]
    );
}

#[test]
fn neighbor_steps_the_filtered_ids_and_clamps() {
    assert_eq!(super::neighbor(&[], None, 1), None);
    assert_eq!(super::neighbor(&[], Some(1), -1), None);
    assert_eq!(super::neighbor(&[2, 4, 9], None, 1), Some(2));
    assert_eq!(super::neighbor(&[2, 4, 9], None, -1), Some(9));
    assert_eq!(super::neighbor(&[2, 4, 9], Some(4), 1), Some(9));
    assert_eq!(super::neighbor(&[2, 4, 9], Some(4), -1), Some(2));
    assert_eq!(super::neighbor(&[2, 4, 9], Some(2), -1), Some(2));
    assert_eq!(super::neighbor(&[2, 4, 9], Some(9), 1), Some(9));
    assert_eq!(super::neighbor(&[2, 4, 9], Some(7), 1), Some(2));
    assert_eq!(super::neighbor(&[2, 4, 9], Some(7), -1), Some(9));
}

#[test]
fn listed_lan_addresses_are_usable_ipv4() {
    for addr in super::lan_addrs() {
        assert!(usable_v4(addr.ip), "{} {}", addr.name, addr.ip);
    }
}

#[test]
fn xdg_download_dir_reads_the_quoted_home_path() {
    let text = "\
# comment
XDG_DESKTOP_DIR=\"$HOME/Desktop\"
XDG_DOWNLOAD_DIR=\"$HOME/Downloads\"
XDG_DOCUMENTS_DIR=\"$HOME/Documents\"
";
    let path = super::xdg_download_dir("/home/ada", text).unwrap();
    assert_eq!(path, std::path::PathBuf::from("/home/ada/Downloads"));
    let absolute = super::xdg_download_dir("/home/ada", "XDG_DOWNLOAD_DIR=/srv/downloads\n");
    assert_eq!(absolute, Some(std::path::PathBuf::from("/srv/downloads")));
    assert!(super::xdg_download_dir("/home/ada", "XDG_DOWNLOAD_DIR=\"\"\n").is_none());
    assert!(super::xdg_download_dir("/home/ada", "XDG_DESKTOP_DIR=\"$HOME/Desktop\"\n").is_none());
}

#[test]
fn hex_display_numbers_each_row_and_copy_does_not() {
    let body: Vec<u8> = (0..20).collect();
    let shown = super::hex_display_rows(&body, 16);
    let mut lines = shown.lines();
    let first = lines.next().unwrap();
    let second = lines.next().unwrap();
    assert!(first.starts_with("0001  "), "{first}");
    assert!(second.starts_with("0002  "), "{second}");
    assert!(first.split_whitespace().skip(1).count() == 16);
    let wide = super::hex_display_rows(&body, 32);
    assert_eq!(wide.lines().count(), 1, "{wide}");
    assert!(wide.split_whitespace().skip(1).count() == 20);
    assert!(wide.contains("0f  10 "), "{wide}");
    assert_eq!(super::hex_row_bytes(400.0), 16);
    assert_eq!(super::hex_row_bytes(super::hex_line_width(32)), 32);
    assert_eq!(super::hex_row_bytes(super::hex_line_width(32) - 1.0), 16);
    assert_eq!(super::hex_cols_for_pane(0.0), 16);
    assert_eq!(super::hex_cols_for_pane(400.0), 16);
    let wide_pane = super::hex_line_width(32) + super::HEX_TEXT_PAD + super::HEX_AREA_CHROME;
    assert_eq!(super::hex_cols_for_pane(wide_pane), 32);
    assert_eq!(super::hex_cols_for_pane(wide_pane - 1.0), 16);
    let copied = super::hex_dump(&body);
    assert!(!copied.contains("0001"), "{copied}");
    assert!(copied.starts_with("00 01 02 "));
    assert!(
        super::response_hex_for_view(true, "/tmp/preview.png", body.clone()).is_empty(),
        "pretty image view hides hex bytes"
    );
    assert_eq!(
        super::response_hex_for_view(false, "/tmp/preview.png", body.clone()),
        body,
        "raw image view keeps hex bytes for 16/32-byte responsive reflow"
    );
}

#[test]
fn body_box_follows_the_text_and_stops_at_fifteen_lines() {
    assert_eq!(super::body_box_height(1), 34.0);
    assert_eq!(super::body_box_height(super::body_line_count("a\nb")), 52.0);
    assert_eq!(super::body_box_height(10), 196.0);
    assert_eq!(super::body_box_height(15), 286.0);
    assert_eq!(super::body_box_height(40), 286.0);
    assert_eq!(super::editor_box_height("only", 0.0), 34.0);
    let long = "x\n".repeat(30);
    assert_eq!(super::editor_box_height(&long, 0.0), 286.0);
}

#[test]
fn soft_wrapped_long_lines_open_the_editor_box() {
    // Minified JS: one line, many rows once wrapped, so the box goes full height.
    let minified = "a".repeat(20_000);
    assert!(super::wrapped_line_count(&minified, 100) > super::BODY_LINES);
    assert_eq!(super::editor_box_height(&minified, 800.0), 286.0);
    // A line a bit wider than the box takes two rows, not the full height.
    let cols = super::editor_wrap_cols(800.0);
    let two_rows = "b".repeat(cols + 1);
    assert_eq!(super::wrapped_line_count(&two_rows, cols), 2);
    assert_eq!(super::editor_box_height(&two_rows, 800.0), 52.0);
    // Short lines still count one row each; empty lines too.
    assert_eq!(super::wrapped_line_count("a\n\nb", 80), 3);
    assert_eq!(super::editor_wrap_cols(0.0), 80);
    assert_eq!(super::editor_wrap_cols(100.0), 20);
}

#[test]
fn body_text_cuts_on_a_char_boundary() {
    let mut raw = vec![b'a'; 256 * 1024 - 2];
    raw.extend_from_slice("不".as_bytes());
    raw.extend(std::iter::repeat_n(b'b', 40));
    let shown = super::body_text(&raw, false, None, None, super::Lang::Zh, 0);
    assert_eq!(shown.text.len(), 256 * 1024 - 2);
    assert!(shown.text.is_char_boundary(shown.text.len()));
    assert!(shown.text.chars().all(|ch| ch == 'a'));
    let rest = raw.len() - shown.text.len();
    assert!(shown.note.contains(&rest.to_string()), "{}", shown.note);
    assert!(shown.window.has_more());
    assert!(!shown.window.has_prev());

    let ascii = vec![b'x'; 256 * 1024 + 10];
    let ascii_shown = super::body_text(&ascii, false, None, None, super::Lang::En, 0);
    assert_eq!(ascii_shown.text.len(), 256 * 1024);
    assert!(
        ascii_shown.note.contains("10 more bytes"),
        "{}",
        ascii_shown.note
    );

    let next = super::body_text(
        &ascii,
        false,
        None,
        None,
        super::Lang::En,
        ascii_shown.window.end,
    );
    assert_eq!(next.text, "x".repeat(10));
    assert!(next.window.has_prev());
    assert!(!next.window.has_more());
    assert!(
        next.note.contains("Showing bytes 262145-262154 of 262154"),
        "{}",
        next.note
    );

    let continued = super::body_text(&raw, false, None, None, super::Lang::Zh, shown.window.end);
    assert!(continued.text.starts_with('不'), "{}", continued.text);
    assert!(continued.text.ends_with('b'));
    assert!(continued.window.has_prev());
    let inside = super::body_text(&raw, false, None, None, super::Lang::Zh, 256 * 1024);
    assert_eq!(inside.text, continued.text);

    let mut binary = vec![0xff; 5000];
    binary[0] = 0xfe;
    let binary_shown = super::body_text(&binary, false, None, None, super::Lang::Zh, 4096);
    assert_eq!(binary_shown.hex.len(), 4096);
    assert!(!binary_shown.window.has_more());
    assert!(!binary_shown.window.has_prev());
    assert!(binary_shown.note.contains("4096"), "{}", binary_shown.note);

    let payload = format!("{{\"k\":\"{}\"}}", "a".repeat(super::BODY_DISPLAY_CAP));
    let mut pretty_first =
        super::body_text(payload.as_bytes(), false, None, None, super::Lang::En, 0);
    super::apply_pretty(
        &mut pretty_first,
        payload.as_bytes(),
        Some("application/json"),
        super::Lang::En,
        0,
    );
    assert!(pretty_first.window.has_more(), "{}", pretty_first.note);
    assert!(pretty_first.text.len() <= super::BODY_DISPLAY_CAP);
    assert!(
        pretty_first.note.contains("more bytes"),
        "{}",
        pretty_first.note
    );
    let mut pretty_next = super::body_text(
        payload.as_bytes(),
        false,
        None,
        None,
        super::Lang::En,
        pretty_first.window.end,
    );
    super::apply_pretty(
        &mut pretty_next,
        payload.as_bytes(),
        Some("application/json"),
        super::Lang::En,
        pretty_first.window.end,
    );
    assert!(pretty_next.window.has_prev());
    assert_ne!(pretty_next.text, pretty_first.text);
    assert!(
        pretty_next.note.contains("Showing bytes"),
        "{}",
        pretty_next.note
    );

    let expired_shown = super::body_text(&[], false, Some(65536), None, super::Lang::Zh, 0);
    assert!(expired_shown.text.is_empty());
    assert!(
        expired_shown.note.contains("64.0 KB"),
        "{}",
        expired_shown.note
    );
}

#[test]
fn flow_count_shows_total_or_filtered_over_total() {
    assert_eq!(super::flow_count_text(0, 0, false), "0");
    assert_eq!(super::flow_count_text(4, 4, false), "4");
    assert_eq!(super::flow_count_text(2, 9, true), "2 / 9");
}

#[test]
fn list_columns_round_trip_and_the_pane_clamps() {
    let mut config = crate::config::Config::default();
    assert!(
        config.show_id
            && config.show_method
            && config.show_status
            && config.show_path
            && config.show_gap
            && config.show_elapsed
            && config.show_size
    );
    assert!(!config.show_protocol && !config.show_host && !config.show_client_ip);
    let columns = super::ListColumns::from_config(&config)
        .toggle(super::ListColumn::Protocol)
        .toggle(super::ListColumn::ClientIp)
        .toggle(super::ListColumn::Size)
        .with_path_display(crate::config::PathDisplay::FullUrl)
        .with_ellipsize(crate::config::ListEllipsize::Tail);
    columns.write(&mut config);
    assert!(config.show_protocol);
    assert!(config.show_client_ip);
    assert!(!config.show_size);
    assert!(config.show_path);
    assert!(config.show_full_url);
    assert_eq!(config.ellipsize, crate::config::ListEllipsize::Tail);
    assert_eq!(super::ListColumns::from_config(&config), columns);
    assert_eq!(columns.ellipsize_mode(), super::EllipsizeMode::Tail);
    let row = super::RowLine {
        id: 1,
        method: "GET".to_string(),
        status: Some(200),
        time: "0 ms".to_string(),
        gap_nanos: 0,
        clock: "12:00:00.000".to_string(),
        elapsed: "10 ms".to_string(),
        elapsed_nanos: Some(10_000_000),
        elapsed_slow: false,
        host: "api.example.com".to_string(),
        client_ip: "127.0.0.1".to_string(),
        path: "/v1/users".to_string(),
        host_path: "api.example.com/v1/users".to_string(),
        url: "https://api.example.com/v1/users".to_string(),
        comment: String::new(),
        protocol: "HTTP/2".to_string(),
        size: "128 B".to_string(),
        size_bytes: 128,
        failed: false,
        pinned: false,
        relay_short: String::new(),
    };
    assert_eq!(
        super::row_target_text(&row, super::EffectivePathDisplay::PathOnly),
        "/v1/users"
    );
    assert_eq!(
        super::row_target_text(&row, super::EffectivePathDisplay::HostAndPath),
        "api.example.com/v1/users"
    );
    assert_eq!(
        super::row_target_text(&row, super::EffectivePathDisplay::FullUrl),
        "https://api.example.com/v1/users"
    );
    let empty_url_row = super::RowLine {
        url: String::new(),
        ..row
    };
    assert_eq!(
        super::row_target_text(&empty_url_row, super::EffectivePathDisplay::FullUrl),
        "/v1/users"
    );
    assert!(super::filter_has_host("host:api.example.com"));
    assert!(super::filter_has_host("domain:api.example.com 2xx"));
    assert!(super::filter_has_host(
        "host:test.vicanso.dev|t1.vicanso.dev path:/finance-api"
    ));
    assert!(super::filter_has_host("host:a.com,b.com"));
    assert!(super::filter_has_host("host:a.com | b.com"));
    assert!(super::filter_has_host("host:*.example.com"));
    assert!(!super::filter_has_host(""));
    assert!(!super::filter_has_host("host:"));
    assert!(!super::filter_has_host("host:|,"));
    assert!(!super::filter_has_host("-host:api.example.com"));
    assert!(!super::filter_has_host("-domain:api.example.com"));
    assert!(!super::filter_has_host("mute:api.example.com"));
    assert!(!super::filter_has_host("type:json status:500"));
    assert_eq!(
        super::list_host_path("api.example.com", "/v1/users?a=1", None, false, Lang::Zh),
        "api.example.com/v1/users"
    );
    assert_eq!(
        super::list_host_path(
            "api.example.com",
            "api.example.com:443",
            None,
            false,
            Lang::Zh
        ),
        "api.example.com:443"
    );
    let auto_cols = super::ListColumns::from_config(&crate::config::Config::default());
    assert_eq!(
        super::effective_path_display(auto_cols, ""),
        super::EffectivePathDisplay::HostAndPath
    );
    assert_eq!(
        super::effective_path_display(auto_cols, "type:json"),
        super::EffectivePathDisplay::HostAndPath
    );
    assert_eq!(
        super::effective_path_display(auto_cols, "-host:api.example.com"),
        super::EffectivePathDisplay::HostAndPath
    );
    assert_eq!(
        super::effective_path_display(auto_cols, "mute:api.example.com"),
        super::EffectivePathDisplay::HostAndPath
    );
    assert_eq!(
        super::effective_path_display(auto_cols, "host:test.vicanso.dev|t1.vicanso.dev"),
        super::EffectivePathDisplay::PathOnly
    );
    assert_eq!(
        super::effective_path_display(auto_cols, "host:*.example.com"),
        super::EffectivePathDisplay::PathOnly
    );
    assert_eq!(
        super::effective_path_display(auto_cols, "host:api.example.com"),
        super::EffectivePathDisplay::PathOnly
    );
    assert_eq!(
        super::effective_path_display(auto_cols.toggle(super::ListColumn::Host), ""),
        super::EffectivePathDisplay::PathOnly
    );
    assert_eq!(
        super::effective_path_display(
            auto_cols.with_path_display(crate::config::PathDisplay::PathOnly),
            ""
        ),
        super::EffectivePathDisplay::PathOnly
    );
    assert_eq!(
        super::effective_path_display(
            auto_cols.with_path_display(crate::config::PathDisplay::HostAndPath),
            "host:api.example.com"
        ),
        super::EffectivePathDisplay::HostAndPath
    );
    assert_eq!(
        super::effective_path_hover_display(auto_cols, "host:test.vicanso.dev|t1.vicanso.dev"),
        super::EffectivePathDisplay::FullUrl
    );
    assert_eq!(
        super::effective_path_hover_display(
            auto_cols.toggle(super::ListColumn::Host),
            "host:test.vicanso.dev"
        ),
        super::EffectivePathDisplay::PathOnly
    );
    assert_eq!(
        super::effective_path_hover_display(
            auto_cols.with_path_display(crate::config::PathDisplay::PathOnly),
            "host:test.vicanso.dev"
        ),
        super::EffectivePathDisplay::PathOnly
    );
    assert_eq!(
        super::effective_path_hover_display(auto_cols, ""),
        super::EffectivePathDisplay::HostAndPath
    );
    assert_eq!(
        super::clamp_list_px(10.0),
        crate::config::LIST_WIDTH_MIN as f32
    );
    assert_eq!(
        super::clamp_list_px(5_000.0),
        crate::config::LIST_WIDTH_MAX as f32
    );
    assert_eq!(super::clamp_list_px(640.6), 641.0);
    assert_eq!(
        super::clamp_list_px(-20.0),
        crate::config::LIST_WIDTH_MIN as f32
    );
    assert_eq!(super::clamp_list_px_for_window(900.0, 1000.0), 660.0);
    assert_eq!(super::clamp_list_px_for_window(900.0, 0.0), 900.0);
    assert_eq!(super::clamp_list_px_for_window(900.0, 500.0), 280.0);
    assert_eq!(super::adaptive_filter_panel_width(0.0, 560.0), 760.0);
    assert_eq!(super::adaptive_filter_panel_width(600.0, 560.0), 580.0);
    assert_eq!(super::adaptive_filter_panel_width(900.0, 560.0), 680.0);
    assert_eq!(super::adaptive_filter_panel_width(1200.0, 560.0), 792.0);
    assert_eq!(super::adaptive_filter_panel_width(1448.0, 560.0), 1040.0);
    assert_eq!(super::adaptive_filter_panel_width(1448.0, 644.0), 981.0);
    assert_eq!(super::adaptive_rules_editor_height(0.0), 105.0);
    assert_eq!(super::adaptive_rules_editor_height(520.0), 84.0);
    assert_eq!(super::adaptive_rules_editor_height(760.0), 84.0);
    assert_eq!(super::adaptive_rules_editor_height(1018.0), 264.0);
    assert_eq!(super::adaptive_rules_editor_height_for(760.0, true), 56.0);
    assert_eq!(super::adaptive_rules_editor_height_for(1018.0, true), 82.0);
    for window_h in [520.0, 640.0, 760.0, 900.0, 1018.0, 1440.0] {
        for import_open in [false, true] {
            let panel_max = super::adaptive_filter_panel_max_height(window_h);
            let body_max = super::adaptive_filter_body_max_height(window_h, import_open);
            let import_h = if import_open {
                super::FILTER_PANEL_IMPORT_EXTRA_HEIGHT
            } else {
                0.0
            };
            // Top toolbar (42) + popover offset (6) + panel_max + bottom status bar (26) <= window_h
            assert!(
                48.0 + panel_max + 26.0 <= window_h,
                "filter panel overflows window_h={window_h}: panel_max={panel_max}"
            );
            assert!(
                body_max + super::FILTER_PANEL_DOCK_HEIGHT + import_h <= panel_max,
                "filter body+dock exceeds panel_max at window_h={window_h}, import_open={import_open}"
            );
        }
    }
    assert_eq!(
        super::StatusMsg::ProfilesExported { count: 2 }.toast_kind(),
        Some(false)
    );
    assert_eq!(
        super::StatusMsg::ExportFailed("err".into()).toast_kind(),
        Some(true)
    );
    assert_eq!(
        super::StatusMsg::Listening {
            port: 8877,
            via_system_proxy: false,
        }
        .toast_kind(),
        None
    );
}

#[test]
fn flow_columns_pin_method_and_path_and_keep_a_dragged_width() {
    let shown = super::ListColumns {
        id: false,
        method: true,
        path: true,
        status: true,
        gap: true,
        elapsed: true,
        size: false,
        protocol: false,
        host: false,
        client_ip: false,
        path_display: crate::config::PathDisplay::Auto,
        ellipsize: crate::config::ListEllipsize::Middle,
        order: super::ListColumn::ORDERED,
        order_rev: 0,
    };
    let mut widths = std::collections::HashMap::new();
    widths.insert("path".to_string(), 400);
    let cols = super::flow_table_columns(shown, &widths);
    let keys: Vec<&str> = cols.iter().map(|column| column.key.as_ref()).collect();
    assert_eq!(keys, ["method", "path", "status", "gap", "elapsed"]);
    assert_eq!(cols[0].fixed, super::ColumnFixed::Left);
    assert_eq!(cols[1].fixed, super::ColumnFixed::Left);
    assert!(
        cols[2..]
            .iter()
            .all(|column| column.fixed == super::ColumnFixed::None)
    );
    assert_eq!(cols[0].width.unwrap().0, 76.0);
    assert_eq!(cols[1].width.unwrap().0, 400.0);
    assert_eq!(cols[1].min_width.unwrap().0, 80.0);
    assert!(
        cols.iter()
            .all(|column| { column.movable == Some(true) && column.resizable == Some(true) })
    );
    let hidden_path = super::ListColumns {
        path: false,
        ..shown
    };
    let cols = super::flow_table_columns(hidden_path, &widths);
    assert_eq!(cols[0].key.as_ref(), "method");
    assert!(cols.iter().all(|column| column.key.as_ref() != "path"));
    assert_eq!(
        cols.iter()
            .filter(|column| column.fixed == super::ColumnFixed::Left)
            .count(),
        1
    );

    let default_shown = super::ListColumns::from_config(&crate::config::Config::default());
    let default_cols = super::flow_table_columns(default_shown, &std::collections::HashMap::new());
    let default_keys: Vec<&str> = default_cols.iter().map(|c| c.key.as_ref()).collect();
    assert_eq!(
        default_keys,
        ["id", "method", "path", "status", "gap", "elapsed", "size"]
    );
    assert!(
        default_cols[..3]
            .iter()
            .all(|column| column.fixed == super::ColumnFixed::Left)
    );
    let default_sum: f32 = default_cols.iter().map(|c| c.width.unwrap().0).sum();
    assert_eq!(default_sum, 648.0);

    let all_shown = super::ListColumns {
        id: true,
        method: true,
        path: true,
        status: true,
        gap: true,
        elapsed: true,
        size: true,
        protocol: true,
        host: true,
        client_ip: true,
        path_display: crate::config::PathDisplay::Auto,
        ellipsize: crate::config::ListEllipsize::Middle,
        order: super::ListColumn::ORDERED,
        order_rev: 0,
    };
    let all_cols = super::flow_table_columns(all_shown, &widths);
    let toggle_keys: Vec<&str> = super::ListColumn::ORDERED.iter().map(|c| c.key()).collect();
    let table_keys: Vec<&str> = all_cols.iter().map(|column| column.key.as_ref()).collect();
    assert_eq!(
        toggle_keys,
        [
            "id",
            "method",
            "path",
            "status",
            "gap",
            "elapsed",
            "size",
            "protocol",
            "host",
            "client_ip"
        ]
    );
    assert_eq!(toggle_keys, table_keys);

    // Reordering via up/down buttons respects fixed (0..3) and scrolling (3..10) zones.
    assert!(!default_shown.has_custom_order());
    assert!(!default_shown.can_move(super::ListColumn::Id, -1));
    assert!(default_shown.can_move(super::ListColumn::Id, 1));
    assert!(!default_shown.can_move(super::ListColumn::Path, 1));
    assert!(!default_shown.can_move(super::ListColumn::Status, -1));
    assert!(!default_shown.can_move(super::ListColumn::ClientIp, 1));

    let moved = default_shown
        .move_by(super::ListColumn::Method, -1)
        .move_by(super::ListColumn::Elapsed, -1);
    assert!(moved.has_custom_order());
    assert_eq!(moved.first_visible(), Some(super::ListColumn::Method));
    let moved_cols = super::flow_table_columns(moved, &std::collections::HashMap::new());
    let moved_keys: Vec<&str> = moved_cols.iter().map(|c| c.key.as_ref()).collect();
    assert_eq!(
        moved_keys,
        ["method", "id", "path", "status", "elapsed", "gap", "size"]
    );

    // Dragging visible headers updates visible slots while preserving hidden slots.
    let dragged = moved.apply_visible_order(&[
        "path".into(),
        "method".into(),
        "id".into(),
        "size".into(),
        "status".into(),
        "elapsed".into(),
        "gap".into(),
    ]);
    assert_eq!(dragged.first_visible(), Some(super::ListColumn::Path));
    assert_eq!(dragged.last_visible(), Some(super::ListColumn::Gap));
    assert_eq!(
        super::column_pad_right(Some(super::ListColumn::Size), true, default_shown).0,
        14.0
    );
    assert_eq!(
        super::column_pad_right(Some(super::ListColumn::Size), true, dragged).0,
        6.0
    );
    assert_eq!(
        super::column_pad_right(Some(super::ListColumn::Gap), true, dragged).0,
        14.0
    );
    assert_eq!(
        super::column_pad_right(Some(super::ListColumn::ClientIp), false, all_shown).0,
        12.0
    );
    assert_eq!(
        super::column_pad_right(Some(super::ListColumn::Status), false, all_shown).0,
        4.0
    );
    let dragged_cols = super::flow_table_columns(dragged, &std::collections::HashMap::new());
    let dragged_keys: Vec<&str> = dragged_cols.iter().map(|c| c.key.as_ref()).collect();
    assert_eq!(
        dragged_keys,
        ["path", "method", "id", "size", "status", "elapsed", "gap"]
    );
    let mut cfg = crate::config::Config::default();
    dragged.write(&mut cfg);
    assert_eq!(super::ListColumns::from_config(&cfg), dragged);
    assert_eq!(dragged.reset_order(), default_shown);
}

#[test]
fn failed_rows_show_err_and_the_short_reason() {
    assert_eq!(super::list_status(Some(304), false), "304");
    assert_eq!(super::list_status(None, false), "—");
    assert_eq!(super::list_status(None, true), "ERR");
    assert_eq!(super::list_status(Some(502), true), "502");
    assert_eq!(
        super::list_path("/index", None, false, super::Lang::Zh),
        "/index"
    );
    assert_eq!(
        super::list_path("/get?env=staging&ts=1", None, false, super::Lang::Zh),
        "/get"
    );
    assert_eq!(
        super::list_url(
            "https://httpbin.org/get?env=staging&ts=1",
            "/get?env=staging&ts=1",
            None,
            false,
            super::Lang::Zh,
        ),
        "https://httpbin.org/get?env=staging&ts=1"
    );
    assert_eq!(
        super::list_url("", "/get?env=staging&ts=1", None, false, super::Lang::Zh),
        "/get?env=staging&ts=1"
    );
    assert_eq!(
        super::list_path("/", Some("证书被拒绝"), true, super::Lang::Zh),
        "证书被拒绝"
    );
    assert_eq!(
        super::list_path(":443", Some("证书被拒绝"), true, super::Lang::En),
        "Certificate rejected"
    );
    assert_eq!(super::list_path("/", None, true, super::Lang::En), "");
}

#[test]
fn dropped_files_keep_only_har_urls() {
    assert_eq!(
        super::dropped_har_path("file:///Users/tree/a.har").as_deref(),
        Some("/Users/tree/a.har")
    );
    assert_eq!(
        super::dropped_har_path("file://localhost/tmp/my%20capture.har").as_deref(),
        Some("/tmp/my capture.har")
    );
    assert_eq!(
        super::dropped_har_path("file:///C:/captures/one.HAR").as_deref(),
        Some("C:/captures/one.HAR")
    );
    assert_eq!(
        super::dropped_har_path("file:///tmp/%E4%B8%AD.har").as_deref(),
        Some("/tmp/中.har")
    );
    assert!(super::dropped_har_path("file:///tmp/a.txt").is_none());
    assert!(super::dropped_har_path("/tmp/a.har").is_none());
    assert!(super::dropped_har_path("file://server/tmp/a.har").is_none());
    assert!(super::dropped_har_path("file:///tmp/notes.har.bak").is_none());
    assert_eq!(
        super::dropped_toml_path("file:///Users/tree/profiles.toml").as_deref(),
        Some("/Users/tree/profiles.toml")
    );
    assert_eq!(
        super::dropped_toml_path("file:///C:/Users/tree/My%20Setups/dev.TOML").as_deref(),
        Some("C:/Users/tree/My Setups/dev.TOML")
    );
    assert!(super::dropped_toml_path("file:///Users/tree/a.har").is_none());
}

#[test]
fn profiles_export_and_parse_from_text_or_path() {
    let exported = super::profiles_for_export(
        &[],
        "Dev",
        "host:api.dev",
        "",
        "delay api.dev/v1 100ms",
        "",
        true,
        true,
        false,
    );
    assert_eq!(exported.len(), 1);
    assert_eq!(exported[0].name, "Dev");
    assert_eq!(
        super::profile_export_filename("Dev", &exported, 123),
        "zniff-Dev.toml"
    );
    assert_eq!(
        super::profile_export_filename("prod/api:v1", &exported, 123),
        "zniff-prod-api-v1.toml"
    );
    assert_eq!(
        super::profile_export_filename("", &[], 123),
        "zniff-profiles-123.toml"
    );
    let toml_text = crate::config::export_profiles_toml(&exported).unwrap();
    let parsed = super::parse_profiles_from_text_or_path(&toml_text).unwrap();
    assert_eq!(parsed, exported);
    assert!(super::parse_profiles_from_text_or_path("   ").is_err());

    // When `saved` already has a profile and the unnamed working setup matches it,
    // `profiles_for_export` updates that profile without creating a duplicate.
    let same_unnamed = super::profiles_for_export(
        &exported,
        "",
        "host:api.dev",
        "",
        "delay api.dev/v1 100ms",
        "",
        false,
        true,
        false,
    );
    assert_eq!(same_unnamed.len(), 1);
    assert_eq!(same_unnamed[0].name, "Dev");
    assert!(!same_unnamed[0].rules_enabled);

    // When `saved` has a profile and the user is editing a new or selected setup,
    // `profiles_for_export` exports only the currently selected/editing setup.
    let with_new_unnamed = super::profiles_for_export(
        &exported,
        "",
        "host:staging.dev",
        "",
        "",
        "",
        true,
        true,
        false,
    );
    assert_eq!(with_new_unnamed.len(), 1);
    assert_eq!(with_new_unnamed[0].name, "host:staging.dev");

    // When the working fields are empty, `profiles_for_export` falls back to `saved`.
    let fallback_all = super::profiles_for_export(&exported, "", "", "", "", "", true, true, false);
    assert_eq!(fallback_all.len(), 1);
    assert_eq!(fallback_all[0].name, "Dev");
}

#[test]
fn structured_filter_fields_sync_host_path_and_exclude_or() {
    let f = super::set_host_or("GET", "api.example.com | *.cdn.com, sub.test");
    assert_eq!(f, "host:api.example.com|*.cdn.com|sub.test GET");
    assert_eq!(
        super::extract_host_or(&f),
        "api.example.com | *.cdn.com | sub.test"
    );
    let f = super::set_path_or(&f, "/v1 | /auth");
    assert_eq!(
        f,
        "host:api.example.com|*.cdn.com|sub.test GET path:/v1|/auth"
    );
    assert_eq!(super::extract_path_or(&f), "/v1 | /auth");
    let f = super::set_exclude_or(&f, "ads.test | /health");
    assert_eq!(super::extract_exclude_or(&f), "ads.test | /health");
}

#[test]
fn named_filters_and_rules_store_and_deduplicate() {
    let saved = super::push_saved_filter_named(&[], "调试接口", "host:example.com type:json");
    assert_eq!(
        saved,
        vec!["调试接口\thost:example.com type:json".to_string()]
    );
    assert_eq!(
        super::split_saved_filter(&saved[0]),
        ("调试接口", "host:example.com type:json")
    );
    // Updating the same expression with a new name replaces the old entry.
    let saved = super::push_saved_filter_named(&saved, "预发接口", "host:example.com type:json");
    assert_eq!(
        saved,
        vec!["预发接口\thost:example.com type:json".to_string()]
    );

    let rules = super::push_saved_rule(&[], "改文案", "replace example.com/api req hello -> hi");
    assert_eq!(rules.len(), 1);
    assert_eq!(rules[0].name, "改文案");
    // Unnamed rule auto-generates name from the first rule line.
    let rules = super::push_saved_rule(&rules, "", "break api.test/v1");
    assert_eq!(rules.len(), 2);
    assert_eq!(rules[0].name, "break api.test/v1");
}

#[test]
fn save_stays_off_until_the_open_profile_changes() {
    let saved = [crate::config::SavedProfile {
        name: "test".into(),
        filter: "host:a.test".into(),
        bypass_hosts: String::new(),
        rules: "break a.test/api\n".into(),
        upstream_proxy: String::new(),
        rules_enabled: true,
        breaks_enabled: true,
        show_tunnel_failures: false,
    }];
    let ready = |name, filter, rules, rules_on, tunnels, host, from| {
        super::profile_save_ready(
            &saved, name, filter, "", rules, "", rules_on, true, tunnels, host, "", "", from, "",
            false, 0,
        )
    };
    assert!(!ready(
        "test",
        "host:a.test",
        "break a.test/api\n",
        true,
        false,
        "",
        ""
    ));
    assert!(!ready(
        "test",
        " host:a.test ",
        "break a.test/api",
        true,
        false,
        "",
        ""
    ));
    assert!(ready(
        "test",
        "host:b.test",
        "break a.test/api",
        true,
        false,
        "",
        ""
    ));
    assert!(ready(
        "test",
        "host:a.test",
        "break a.test/api",
        false,
        false,
        "",
        ""
    ));
    assert!(ready(
        "test",
        "host:a.test",
        "break a.test/api",
        true,
        true,
        "",
        ""
    ));
    assert!(ready(
        "Test",
        "host:a.test",
        "break a.test/api",
        true,
        false,
        "",
        ""
    ));
    assert!(ready(
        "other",
        "host:a.test",
        "break a.test/api",
        true,
        false,
        "",
        ""
    ));
    assert!(!ready("test", "", "", true, false, "", ""));
    assert!(ready("", "host:b.test", "", true, false, "", ""));
    assert!(ready(
        "test",
        "host:a.test",
        "break a.test/api",
        true,
        false,
        "a.test",
        "keep"
    ));
    assert_eq!(
        super::draft_rules("break a.test/api", "a.test", "", "keep", "", false, 0),
        "break a.test/api\nreplace a.test req keep -> \n"
    );
    assert_eq!(
        super::draft_rules("break a.test/api", "a.test", "", "with space", "", false, 0),
        "break a.test/api\nreplace a.test req \"with space\" -> \n"
    );
    assert!(!ready(
        "test",
        "host:a.test",
        "break a.test/api",
        true,
        false,
        "a.test",
        ""
    ));
}

#[test]
fn parse_status_code_accepts_three_digit_codes_and_reason_phrases() {
    assert_eq!(super::parse_status_code("201"), Some(201));
    assert_eq!(super::parse_status_code(" 201 Created "), Some(201));
    assert_eq!(super::parse_status_code("99"), None);
    assert_eq!(super::parse_status_code("600"), None);
    assert_eq!(super::parse_status_code(""), None);
}

#[test]
fn detached_owner_survives_child_owner_disposal() {
    use nsh_core::element::{Runtime, create_root};
    use nsh_core::prelude::{Owner, cleanup};
    use std::cell::Cell;
    use std::rc::Rc;

    let runtime = Runtime::default();
    let _entered = runtime.enter();
    let scoped_cancelled = Rc::new(Cell::new(false));
    let detached_cancelled = Rc::new(Cell::new(false));
    let scoped_flag = Rc::clone(&scoped_cancelled);
    let detached_flag = Rc::clone(&detached_cancelled);
    let (menu_owner, ()) = create_root(move || {
        cleanup(move || scoped_flag.set(true));
        Owner::detached().run(|| {
            cleanup(move || detached_flag.set(true));
        });
    });
    menu_owner.dispose();
    assert!(scoped_cancelled.get());
    assert!(!detached_cancelled.get());
}

#[test]
fn list_sort_cycles_asc_desc_none_and_orders_rows() {
    let s0 = None;
    let s1 = super::ListSort::toggle(s0, super::ListColumn::Method);
    assert_eq!(
        s1,
        Some(super::ListSort {
            column: super::ListColumn::Method,
            desc: false
        })
    );
    let s2 = super::ListSort::toggle(s1, super::ListColumn::Method);
    assert_eq!(
        s2,
        Some(super::ListSort {
            column: super::ListColumn::Method,
            desc: true
        })
    );
    let s3 = super::ListSort::toggle(s2, super::ListColumn::Method);
    assert_eq!(s3, None);
    let s_other = super::ListSort::toggle(s2, super::ListColumn::Status);
    assert_eq!(
        s_other,
        Some(super::ListSort {
            column: super::ListColumn::Status,
            desc: false
        })
    );

    let make_row = |id: u64,
                    method: &str,
                    status: Option<u16>,
                    failed: bool,
                    size_bytes: u64,
                    elapsed_nanos: Option<u128>,
                    host: &str,
                    path: &str| super::RowLine {
        id,
        method: method.to_string(),
        status,
        time: "0 ms".to_string(),
        gap_nanos: id as u128 * 10,
        clock: "12:00:00.000".to_string(),
        elapsed: "—".to_string(),
        elapsed_nanos,
        elapsed_slow: false,
        host: host.to_string(),
        client_ip: "127.0.0.1".to_string(),
        path: path.to_string(),
        host_path: format!("{host}{path}"),
        url: format!("https://{host}{path}"),
        comment: String::new(),
        protocol: "HTTP/1.1".to_string(),
        size: "0 B".to_string(),
        size_bytes,
        failed,
        pinned: false,
        relay_short: String::new(),
    };

    let base = vec![
        make_row(
            1,
            "POST",
            Some(500),
            false,
            120,
            Some(300),
            "b.test",
            "/users",
        ),
        make_row(
            2,
            "GET",
            Some(200),
            false,
            4096,
            Some(50),
            "a.test",
            "/alpha",
        ),
        make_row(3, "DELETE", None, true, 0, None, "c.test", "/err"),
        make_row(
            4,
            "GET",
            Some(200),
            false,
            512,
            Some(100),
            "a.test",
            "/zeta",
        ),
    ];

    let mut by_method_asc = base.clone();
    super::sort_row_lines(
        &mut by_method_asc,
        super::ListSort {
            column: super::ListColumn::Method,
            desc: false,
        },
        super::EffectivePathDisplay::PathOnly,
    );
    let ids: Vec<u64> = by_method_asc.iter().map(|r| r.id).collect();
    // DELETE (3), GET (2, then 4 by id tie-breaker), POST (1)
    assert_eq!(ids, vec![3, 2, 4, 1]);

    let mut by_size_desc = base.clone();
    super::sort_row_lines(
        &mut by_size_desc,
        super::ListSort {
            column: super::ListColumn::Size,
            desc: true,
        },
        super::EffectivePathDisplay::PathOnly,
    );
    let ids: Vec<u64> = by_size_desc.iter().map(|r| r.id).collect();
    assert_eq!(ids, vec![2, 4, 1, 3]);

    let mut by_elapsed_desc = base.clone();
    super::sort_row_lines(
        &mut by_elapsed_desc,
        super::ListSort {
            column: super::ListColumn::Elapsed,
            desc: true,
        },
        super::EffectivePathDisplay::PathOnly,
    );
    let ids: Vec<u64> = by_elapsed_desc.iter().map(|r| r.id).collect();
    // Unfinished (`None`, id 3) stays at the bottom even in descending sort.
    assert_eq!(ids, vec![1, 4, 2, 3]);

    let candidate = super::SeqCandidate {
        id: 9,
        method: "GET".into(),
        host: "api.example.com".into(),
        path: "/v1/orders?limit=10".into(),
        status: Some(200),
        failed: false,
        elapsed: "12 ms".into(),
        comment: "fetch orders".into(),
        checked: true,
    };
    assert_eq!(
        super::seq_candidate_target(&candidate, false),
        "api.example.com/v1/orders"
    );
    assert_eq!(
        super::seq_candidate_target(&candidate, true),
        "api.example.com/v1/orders?limit=10"
    );
}

#[test]
fn default_save_dir_prefers_xdg_downloads_then_home_and_cwd() {
    let base = std::env::temp_dir().join(format!("zniff-save-dir-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let home = base.join("home");
    let custom_dl = home.join("MyDownloads");
    let std_dl = home.join("Downloads");
    let cwd = base.join("cwd");
    std::fs::create_dir_all(&custom_dl).unwrap();
    std::fs::create_dir_all(&std_dl).unwrap();
    std::fs::create_dir_all(&cwd).unwrap();

    let home_str = home.to_string_lossy();
    let user_dirs = "XDG_DOWNLOAD_DIR=\"$HOME/MyDownloads\"\n";
    assert_eq!(
        super::default_save_dir_from(Some(&home_str), Some(user_dirs), Some(&cwd)),
        Some(custom_dl)
    );
    assert_eq!(
        super::default_save_dir_from(Some(&home_str), None, Some(&cwd)),
        Some(std_dl.clone())
    );
    std::fs::remove_dir_all(&std_dl).unwrap();
    assert_eq!(
        super::default_save_dir_from(Some(&home_str), None, Some(&cwd)),
        Some(home.clone())
    );
    assert_eq!(
        super::default_save_dir_from(None, None, Some(&cwd)),
        Some(cwd.clone())
    );

    let exported = base.join("nested").join("export.har");
    super::write_export_file(&exported, b"{\"log\":{}}").unwrap();
    assert_eq!(std::fs::read(&exported).unwrap(), b"{\"log\":{}}");
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn facet_toggles_host_app_and_mute_tokens_cleanly() {
    let f0 = "type:json";
    let f1 = super::toggle_host_facet(f0, "api.example.com");
    assert_eq!(f1, "host:api.example.com type:json");
    assert!(super::host_facet_active(&f1, "api.example.com"));
    let f2 = super::toggle_host_facet(&f1, "cdn.example.com");
    assert_eq!(f2, "host:api.example.com|cdn.example.com type:json");
    let f3 = super::toggle_host_facet(&f2, "api.example.com");
    assert_eq!(f3, "host:cdn.example.com type:json");
    let f4 = super::toggle_host_facet(&f3, "cdn.example.com");
    assert_eq!(f4, "type:json");

    let a1 = super::toggle_app_facet("type:json", "Google_Chrome");
    assert_eq!(a1, "type:json app:Google_Chrome");
    assert!(super::app_facet_active(&a1, "google_chrome"));
    let a2 = super::toggle_app_facet(&a1, "curl");
    assert_eq!(a2, "type:json app:Google_Chrome|curl");
    let a3 = super::toggle_app_facet(&a2, "Google_Chrome");
    assert_eq!(a3, "type:json app:curl");

    let m1 = super::toggle_mute_facet("type:json", "ads.example.com");
    assert_eq!(m1, "type:json mute:ads.example.com");
    assert!(super::mute_facet_active(&m1, "ads.example.com"));
    let m2 = super::toggle_mute_facet(&m1, "ads.example.com");
    assert_eq!(m2, "type:json");
}

#[test]
fn seq_preview_height_fills_remaining_space_and_clamps_bounds() {
    assert_eq!(super::seq_preview_height(760.0, 7), 260.0);
    assert_eq!(super::seq_preview_height(900.0, 7), 400.0);
    assert_eq!(super::seq_preview_height(900.0, 20), 400.0);
    assert_eq!(super::seq_preview_height(900.0, 2), 560.0);
    assert_eq!(super::seq_preview_height(500.0, 7), 180.0);
    assert_eq!(super::seq_preview_height(0.0, 7), 260.0);
}

#[test]
fn push_profile_history_keeps_last_ten_distinct_versions() {
    let mut history = Vec::new();
    for idx in 1..=12 {
        history = super::push_profile_history(
            &history,
            "test",
            &format!("host:v{idx}.example.com"),
            "",
            "no-cache *",
            "",
            true,
            true,
            false,
        );
    }
    assert_eq!(history.len(), crate::config::PROFILE_HISTORY_CAP);
    assert_eq!(history[0].filter, "host:v12.example.com");
    assert_eq!(history[9].filter, "host:v3.example.com");

    // Re-recording an existing content snapshot moves it to the front without duplicating.
    history = super::push_profile_history(
        &history,
        "test-renamed",
        "host:v5.example.com",
        "",
        "no-cache *",
        "",
        true,
        true,
        false,
    );
    assert_eq!(history.len(), 10);
    assert_eq!(history[0].filter, "host:v5.example.com");
    assert_eq!(history[0].name, "test-renamed");
    assert_eq!(
        history
            .iter()
            .filter(|item| item.filter == "host:v5.example.com")
            .count(),
        1
    );
}

#[test]
fn format_bytes_avoids_clipping_for_large_values() {
    assert_eq!(super::format_bytes(0), "0 B");
    assert_eq!(super::format_bytes(512), "512 B");
    assert_eq!(super::format_bytes(1024), "1.0 KB");
    assert_eq!(super::format_bytes(2048), "2.0 KB");
    assert_eq!(super::format_bytes(99 * 1024), "99.0 KB");
    // Values >= 100 KB omit decimal .0 to prevent cell clipping
    assert_eq!(super::format_bytes(100 * 1024), "100 KB");
    assert_eq!(super::format_bytes(256 * 1024), "256 KB");
    assert_eq!(super::format_bytes(1023 * 1024), "1023 KB");
    // MB formatting
    assert_eq!(super::format_bytes(1024 * 1024), "1.0 MB");
    assert_eq!(super::format_bytes(12 * 1024 * 1024), "12.0 MB");
    assert_eq!(super::format_bytes(99 * 1024 * 1024), "99.0 MB");
    assert_eq!(super::format_bytes(100 * 1024 * 1024), "100 MB");
    assert_eq!(super::format_bytes(256 * 1024 * 1024), "256 MB");
}

#[test]
fn draft_rules_ignores_hidden_leftover_inputs_across_rule_kinds() {
    // Leftover `from` / `to` from Replace must not emit a global `cors *` when host & path are empty.
    assert_eq!(
        super::draft_rules("no-cache *", "", "", "old_from", "old_to", true, 8),
        "no-cache *"
    );
    assert!(!super::profile_save_ready(
        &[crate::config::SavedProfile {
            name: "dev".into(),
            filter: String::new(),
            bypass_hosts: String::new(),
            rules: "no-cache *".into(),
            upstream_proxy: String::new(),
            rules_enabled: true,
            breaks_enabled: true,
            show_tunnel_failures: false,
        }],
        "dev",
        "",
        "",
        "no-cache *",
        "",
        true,
        true,
        false,
        "",
        "",
        "LeftoverOp",
        "old_from",
        "old_to",
        true,
        8,
    ));
    // When `op` is filled on an `op=`-capable rule (`break`, kind 4), `draft_rules_with_op` commits it.
    assert_eq!(
        super::draft_rules_with_op("", "api.test", "/graphql", "GetUser", "", "", false, 4),
        "break api.test/graphql op=GetUser\n"
    );
    // When `op` is left over on `cors` (`kind == 8`), it is ignored.
    assert_eq!(
        super::draft_rules_with_op("", "api.test", "", "GetUser", "", "", true, 8),
        "cors api.test\n"
    );
    // When `host` IS filled on `cors`, leftover `from`/`to` are ignored and `cors api.test` is appended.
    assert_eq!(
        super::draft_rules("", "api.test", "", "old_from", "old_to", true, 8),
        "cors api.test\n"
    );
    // On `block` (`kind == 13`) in `Drop` mode (`response == true`), hidden `to` is ignored.
    assert_eq!(
        super::draft_rules("", "", "", "", "leftover with spaces", true, 13),
        ""
    );
    assert_eq!(
        super::draft_rules("", "api.test", "", "", "leftover with spaces", true, 13),
        "block api.test drop\n"
    );
    // On `map-remote` (`kind == 1`), hidden `to` is ignored so spaces in `to` do not reject a valid target.
    assert_eq!(
        super::draft_rules(
            "",
            "api.test",
            "/v1",
            "https://staging.test/v1",
            "leftover with spaces",
            true,
            1,
        ),
        "map-remote api.test/v1 https://staging.test/v1\n"
    );
    // On `resolve` (`kind == 14`), hidden leftover `path` and `op` are ignored.
    assert_eq!(
        super::draft_rules_with_op("", "", "/v1", "GetUser", "", "", false, 14),
        ""
    );
    assert_eq!(
        super::draft_rules_with_op(
            "",
            "api.test",
            "/v1",
            "GetUser",
            "127.0.0.1",
            "8080",
            false,
            14,
        ),
        "resolve api.test 127.0.0.1:8080\n"
    );
}

#[test]
fn url_encode_query_component_encodes_filter_operators_and_utf8() {
    assert_eq!(
        super::url_encode_query_component("host:a.test|b.test 失败"),
        "host%3Aa.test%7Cb.test%20%E5%A4%B1%E8%B4%A5"
    );
}

#[test]
fn cli_info_output_handles_version_and_help() {
    assert_eq!(super::cli_info_output(&[]), None);
    assert_eq!(super::cli_info_output(&["--web".into()]), None);
    let expected_ver = format!("zniff {}", env!("CARGO_PKG_VERSION"));
    assert_eq!(
        super::cli_info_output(&["--version".into()]).as_deref(),
        Some(expected_ver.as_str())
    );
    assert_eq!(
        super::cli_info_output(&["-V".into()]).as_deref(),
        Some(expected_ver.as_str())
    );
    let help = super::cli_info_output(&["--help".into()]).expect("help text");
    assert!(help.contains(&expected_ver));
    assert!(help.contains("--web"));
    assert!(help.contains("--relay"));
    assert!(help.contains("--mcp"));
    assert_eq!(
        super::cli_info_output(&["-h".into()]).as_deref(),
        Some(help.as_str())
    );
}

#[test]
fn merge_secondary_filter_deduplicates_tokens_and_normalizes_pipes() {
    assert_eq!(
        super::merge_secondary_filter("host:a.test type:json", "type:JSON path:/v1 | /v2"),
        "host:a.test type:json path:/v1|/v2"
    );
    assert_eq!(
        super::merge_secondary_filter("", "  slow:   pin: "),
        "slow: pin:"
    );
    assert_eq!(super::merge_secondary_filter("GET", "   "), "GET");
}

#[test]
fn edit_action_label_and_seq_mermaid_mode_follow_context() {
    use crate::i18n::Lang;
    use crate::proxy::BreakSide;

    assert_eq!(
        super::edit_action_label(Lang::Zh, Some(5), None),
        "编辑重发"
    );
    assert_eq!(
        super::edit_action_label(Lang::En, Some(5), None),
        "Edit & resend"
    );
    assert_eq!(
        super::edit_action_label(Lang::En, Some(5), Some((5, BreakSide::Request))),
        "Edit request"
    );
    assert_eq!(
        super::edit_action_label(Lang::Zh, Some(5), Some((5, BreakSide::Response))),
        "编辑响应"
    );
    assert_eq!(
        super::edit_action_label(Lang::En, Some(6), Some((5, BreakSide::Response))),
        "Edit & resend"
    );

    assert_eq!(
        super::select_seq_mermaid_text(0, true, "sequenceDiagram\nA->>B: 1", "flowchart TD\nA-->B"),
        "sequenceDiagram\nA->>B: 1"
    );
    assert_eq!(
        super::select_seq_mermaid_text(
            2,
            false,
            "sequenceDiagram\nA->>B: 1",
            "flowchart TD\nA-->B"
        ),
        "flowchart TD\nA-->B"
    );
    assert_eq!(
        super::select_seq_mermaid_text(3, true, "sequenceDiagram\nA->>B: 1", "flowchart TD\nA-->B"),
        "flowchart TD\nA-->B"
    );
    assert_eq!(
        super::select_seq_mermaid_text(
            3,
            false,
            "sequenceDiagram\nA->>B: 1",
            "flowchart TD\nA-->B"
        ),
        "sequenceDiagram\nA->>B: 1"
    );
}

#[test]
fn auto_height_popover_scroll_containers_never_use_flex_1() {
    for window_h in [
        0.0, 320.0, 348.0, 420.0, 520.0, 760.0, 917.0, 1018.0, 1440.0,
    ] {
        for import_open in [false, true] {
            let panel_max = super::adaptive_filter_panel_max_height(window_h);
            let body_max = super::adaptive_filter_body_max_height(window_h, import_open);
            let import_h = if import_open {
                super::FILTER_PANEL_IMPORT_EXTRA_HEIGHT
            } else {
                0.0
            };
            assert!(
                body_max > 0.0,
                "filter body_max must stay positive at window_h={window_h}, import_open={import_open}"
            );
            assert!(
                body_max + super::FILTER_PANEL_DOCK_HEIGHT + import_h <= panel_max,
                "filter body+dock ({body_max} + {} + {import_h}) exceeds panel_max ({panel_max}) at window_h={window_h}",
                super::FILTER_PANEL_DOCK_HEIGHT
            );
        }
    }

    let bundled_ui_src = concat!(
        include_str!("app_shell.rs"),
        "\n",
        include_str!("break_bar.rs"),
        "\n",
        include_str!("cli_state.rs"),
        "\n",
        include_str!("column_picker.rs"),
        "\n",
        include_str!("compose_box.rs"),
        "\n",
        include_str!("controls_actions.rs"),
        "\n",
        include_str!("controls_runtime.rs"),
        "\n",
        include_str!("detail_actions.rs"),
        "\n",
        include_str!("detail_body.rs"),
        "\n",
        include_str!("detail_headers_msgs.rs"),
        "\n",
        include_str!("detail_publish.rs"),
        "\n",
        include_str!("detail_summary.rs"),
        "\n",
        include_str!("detail_timing.rs"),
        "\n",
        include_str!("facet_drawer.rs"),
        "\n",
        include_str!("filter_panel.rs"),
        "\n",
        include_str!("filter_tokens.rs"),
        "\n",
        include_str!("flow_cells.rs"),
        "\n",
        include_str!("flow_table.rs"),
        "\n",
        include_str!("footer_qr.rs"),
        "\n",
        include_str!("json_tree.rs"),
        "\n",
        include_str!("lan_qr.rs"),
        "\n",
        include_str!("list_columns.rs"),
        "\n",
        include_str!("machine_settings.rs"),
        "\n",
        include_str!("more_menu.rs"),
        "\n",
        include_str!("more_menu_rows.rs"),
        "\n",
        include_str!("profile_dock.rs"),
        "\n",
        include_str!("profile_io.rs"),
        "\n",
        include_str!("profile_store.rs"),
        "\n",
        include_str!("quick_rules.rs"),
        "\n",
        include_str!("redact_preview.rs"),
        "\n",
        include_str!("row_projection.rs"),
        "\n",
        include_str!("seq_dep_waterfall.rs"),
        "\n",
        include_str!("seq_panel.rs"),
        "\n",
        include_str!("seq_swimlane.rs"),
        "\n",
        include_str!("session_actions.rs"),
        "\n",
        include_str!("theme.rs"),
        "\n",
        include_str!("toolbar.rs"),
    );
    for raw_src in [
        std::borrow::Cow::Borrowed(bundled_ui_src),
        std::borrow::Cow::Owned(bundled_ui_src.replace("\r\n", "\n").replace('\n', "\r\n")),
    ] {
        let normalized = raw_src.replace("\r\n", "\n");
        let prod_src = normalized
            .split("\n#[cfg(test)]\nmod tests")
            .next()
            .expect("ui production source before mod tests");
        assert!(
            !prod_src.contains("fn auto_height_popover_scroll_containers_never_use_flex_1"),
            "prod_src must exclude #[cfg(test)] mod tests on both LF and CRLF checkouts"
        );

        // In Taffy Flexbox, `.flex_1()` sets `flex-basis: 0px`. Putting `.flex_1()` on a
        // vertical `scroll().w_full()` inside an auto-height `.max_h(...)` popover/menu/drawer
        // collapses the scroll body to 0px height!
        for (idx, _) in prod_src.match_indices("scroll()") {
            let tail = &prod_src[idx..];
            let head = tail.split(".child(").next().unwrap_or(tail);
            if head.contains(".w_full()") || head.contains(".max_h(") {
                assert!(
                    !head.contains(".flex_1()"),
                    "scroll() with .w_full()/.max_h() must never use .flex_1() (collapses to 0px in auto-height parent): {head}"
                );
            }
        }

        let fn_body = |name: &str| -> &str {
            prod_src
                .split(name)
                .nth(1)
                .and_then(|s| s.split("\npub(crate) fn ").next())
                .and_then(|s| s.split("\nfn ").next())
                .unwrap_or_else(|| panic!("{name} source"))
        };

        let filter_panel_src = fn_body("fn saved_filter_panel(");
        let filter_outer_and_scroll = filter_panel_src
            .split(
                "uncull_nested_scroll(view())\n                        .w_full()\n                        .p_3()",
            )
            .next()
            .expect("saved_filter_panel outer scroll header before uncull_nested_scroll");
        assert!(
            filter_outer_and_scroll.contains(".max_h(")
                && !filter_outer_and_scroll.contains(".flex_1()"),
            "saved_filter_panel outer auto-height view + scroll() must not use .flex_1()"
        );

        // Outer scroll containers that host nested `editor()`, `diff_view()`, or inner
        // `scroll()` / `virtual_list()` must opt their content roots out of nsh-core 0.5's
        // `cull_scope` via `uncull_nested_scroll`.
        let ready_detail_src = fn_body("fn ready_detail(");
        assert!(
            ready_detail_src.contains("uncull_nested_scroll(view())"),
            "ready_detail root view must use uncull_nested_scroll"
        );
        let detail_pane_src = fn_body("fn detail_pane(");
        assert_eq!(
            detail_pane_src
                .matches("uncull_nested_scroll(view())")
                .count(),
            4,
            "all 4 non-Ready branches of detail_pane must use uncull_nested_scroll"
        );
        let swimlane_src = fn_body("fn seq_swimlane_view(");
        assert!(
            swimlane_src.contains("scroll().horizontal().size_full().child(\n                    uncull_nested_scroll(view())"),
            "seq_swimlane_view horizontal scroll content root must use uncull_nested_scroll"
        );

        // Every `editor()` (6 sites) and `diff_view()` (2 sites) in production code must
        // wrap its container in `guard_culled_pointer_events(view())` so `Layer::Paint` /
        // selection overlays emit `MountItem::ViewProps { pointer_events: None }`.
        for (idx, _) in prod_src.match_indices("editor()\n") {
            let start = prod_src.floor_char_boundary(idx.saturating_sub(600));
            let before = &prod_src[start..idx];
            assert!(
                before.contains("guard_culled_pointer_events(view())"),
                "every editor() must be wrapped in guard_culled_pointer_events(view()): {before}"
            );
        }
        for (idx, _) in prod_src.match_indices("diff_view()\n") {
            let start = prod_src.floor_char_boundary(idx.saturating_sub(600));
            let before = &prod_src[start..idx];
            assert!(
                before.contains("guard_culled_pointer_events(view())"),
                "every diff_view() must be wrapped in guard_culled_pointer_events(view()): {before}"
            );
        }
        for (idx, _) in prod_src.match_indices("tree()\n") {
            let start = prod_src.floor_char_boundary(idx.saturating_sub(600));
            let before = &prod_src[start..idx];
            assert!(
                before.contains("guard_culled_pointer_events(view())"),
                "every tree() must be wrapped in guard_culled_pointer_events(view()): {before}"
            );
        }
    }
}

#[tokio::test]
async fn nested_scroll_widgets_emit_scroll_content_under_content_culling() {
    use nsh_base::{DiffLayout, diff_view, editor};
    use nsh_core::prelude::*;
    use nsh_core::{
        ApproxTextEngine, Core, EventKind, EventPayload, MountItem, NodeId, Op, PressEvent, Prop,
        ROOT_ID, Tag, ViewKind,
    };

    let sample_json: String = (0..32)
        .map(|i| format!("  \"field_{i:02}\": \"value_{i:02}\""))
        .collect::<Vec<_>>()
        .join(",\n");
    let sample_json = format!("{{\n{sample_json}\n}}");

    // Unguarded control component: raw `editor()` inside an unguarded `scroll()`
    // reproduces the `nsh-core` 0.5 `mount_state` bug where `Layer::Paint` is created
    // via `sync_culled` without ever receiving `MountItem::ViewProps { pointer_events: None }`.
    #[component]
    fn UnguardedRawEditorHarness() -> impl IntoElement {
        let selected = signal(false);
        let body = signal({
            let lines = (0..30)
                .map(|i| format!("  \"k{i}\": {i}"))
                .collect::<Vec<_>>()
                .join(",\n");
            format!("{{\n{lines}\n}}")
        });
        view()
            .w(px(540.0))
            .h(px(600.0))
            .on_press(move || selected.set(true))
            .child(scroll().flex_1().w_full().child(view().p_3().child(when(
                move || selected.get(),
                move || {
                    view().w_full().h(px(196.0)).overflow_hidden().child(
                        editor()
                            .value(move || SharedString::from(body.get()))
                            .read_only(true)
                            .language("json")
                            .line_numbers(true)
                            .folding(true)
                            .soft_wrap(true)
                            .font_size(Points(12.0))
                            .line_height(Points(18.0))
                            .scroll_beyond_last_line(false)
                            .size_full(),
                    )
                },
            ))))
    }

    // Guarded harness matching real `detail_pane` / `saved_filter_panel`:
    // - Mounts an empty `scroll()` first, then dynamically inserts content on press;
    // - Renders 80 sibling rows (`> PACE_AFTER = 64` views) + tall content pushing
    //   subsequent editors/diffs below the fold (`y > 1600pt`) so both `cull_scope`
    //   and `pace_hold` are exercised;
    // - Exercises `code_body` (`editor()`), editable `compose_editor` (`editor()`),
    //   plain/hex nested `scroll()`, guarded `diff_view()`, and horizontal swimlane
    //   `scroll()` + `virtual_list()`.
    #[component]
    fn GuardedScrollHarness() -> impl IntoElement {
        let selected = signal(false);
        let body = signal({
            let lines = (0..30)
                .map(|i| format!("  \"k{i}\": {i}"))
                .collect::<Vec<_>>()
                .join(",\n");
            format!("{{\n{lines}\n}}")
        });
        let editable = signal(
            (0..24)
                .map(|i| format!("X-Header-{i}: value-{i}"))
                .collect::<Vec<_>>()
                .join("\n"),
        );
        let lang = signal("json".to_string());
        let search = signal(String::new());
        let old_diff = (0..28)
            .map(|i| format!("old line {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let new_diff = (0..28)
            .map(|i| format!("new line {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let plain_tall = (0..30)
            .map(|i| format!("00{i:02}  48 65 6c 6c 6f 20 77 6f 72 6c 64"))
            .collect::<Vec<_>>()
            .join("\n");
        view()
            .w(px(540.0))
            .h(px(600.0))
            .on_press(move || selected.set(true))
            .child(
                scroll().flex_1().w_full().child(
                    super::uncull_nested_scroll(view())
                        .w_full()
                        .p_3()
                        .gap_3()
                        .child(when(
                            move || selected.get(),
                            move || {
                                let old_diff = old_diff.clone();
                                let new_diff = new_diff.clone();
                                let plain_tall = plain_tall.clone();
                                super::uncull_nested_scroll(view())
                                    .w_full()
                                    .gap_3()
                                    // 80 header rows (> PACE_AFTER = 64 views, ~1760pt tall)
                                    // to push subsequent widgets past both PACE_AFTER and OVERSCAN!
                                    .children((0..80).map(|i| {
                                        view()
                                            .w_full()
                                            .h(px(22.0))
                                            .child(text(format!("header-{i}: val")))
                                    }))
                                    // 1. Response/Request `code_body` (`editor()`)
                                    .child(super::code_body(body, lang, search))
                                    // 2. Editable `editor()` (`compose_editor` / `rules_editor`)
                                    .child(
                                        super::guard_culled_pointer_events(view())
                                            .w_full()
                                            .h(px(120.0))
                                            .overflow_hidden()
                                            .child(
                                                editor()
                                                    .value(move || {
                                                        SharedString::from(editable.get())
                                                    })
                                                    .language("plaintext")
                                                    .placeholder("headers")
                                                    .line_numbers(true)
                                                    .folding(true)
                                                    .soft_wrap(true)
                                                    .font_size(Points(12.0))
                                                    .line_height(Points(18.0))
                                                    .scroll_beyond_last_line(false)
                                                    .on_change(move |t: &SharedString| {
                                                        editable.set(t.to_string());
                                                    })
                                                    .size_full(),
                                            ),
                                    )
                                    // 3. Plain/hex/Markdown nested `scroll()`
                                    .child(
                                        view().w_full().h(px(196.0)).overflow_hidden().child(
                                            scroll()
                                                .size_full()
                                                .child(super::mono(plain_tall).text_size(px(12.0))),
                                        ),
                                    )
                                    // 4. Baseline / Redaction `diff_view()`
                                    .child(
                                        super::guard_culled_pointer_events(view())
                                            .w_full()
                                            .h(px(240.0))
                                            .overflow_hidden()
                                            .child(
                                                diff_view()
                                                    .old_value(SharedString::from(old_diff))
                                                    .new_value(SharedString::from(new_diff))
                                                    .layout(DiffLayout::Unified)
                                                    .language("plaintext")
                                                    .soft_wrap(true)
                                                    .font_size(Points(12.0))
                                                    .line_height(Points(18.0))
                                                    .size_full(),
                                            ),
                                    )
                                    // 5. Horizontal swimlane `scroll()` + `virtual_list()`
                                    .child(
                                        view().w_full().h(px(180.0)).overflow_hidden().child(
                                            scroll().horizontal().size_full().child(
                                                super::uncull_nested_scroll(view())
                                                    .h_full()
                                                    .w(px(960.0))
                                                    .child(
                                                        virtual_list(
                                                            || 25usize,
                                                            |at| {
                                                                view().h(px(28.0)).w_full().child(
                                                                    text(move || {
                                                                        format!(
                                                                            "step {}",
                                                                            at.index()
                                                                        )
                                                                    }),
                                                                )
                                                            },
                                                        )
                                                        .estimated_item_size(px(28.0))
                                                        .flex_1()
                                                        .w_full(),
                                                    ),
                                            ),
                                        ),
                                    )
                            },
                        )),
                ),
            )
    }

    struct MountedTree {
        kinds: std::collections::HashMap<NodeId, ViewKind>,
        children: std::collections::HashMap<NodeId, Vec<NodeId>>,
        frames: std::collections::HashMap<NodeId, Bounds<Points>>,
        scroll_contents: std::collections::HashMap<NodeId, Size<Points>>,
        scroll_horiz: std::collections::HashMap<NodeId, bool>,
        pointer_events: std::collections::HashMap<NodeId, PointerEvents>,
    }

    fn drive_select_flow<C: nsh_core::element::Component>() -> MountedTree {
        let mut core = Core::new(ApproxTextEngine, Size::new(px(540.0), px(600.0)));
        core.set_content_culling(true);
        let mut registry = Registry::new();
        registry.register::<C>();
        core.set_components(registry);
        for op in [
            Op::Create {
                id: 2,
                tag: Tag::View,
            },
            Op::SetProp {
                id: 2,
                prop: Prop::Component(std::any::type_name::<C>().into()),
            },
            Op::Insert {
                parent: ROOT_ID,
                child: 2,
                before: None,
            },
        ] {
            core.apply(op).expect("apply op");
        }

        let mut tree = MountedTree {
            kinds: std::collections::HashMap::new(),
            children: std::collections::HashMap::new(),
            frames: std::collections::HashMap::new(),
            scroll_contents: std::collections::HashMap::new(),
            scroll_horiz: std::collections::HashMap::new(),
            pointer_events: std::collections::HashMap::new(),
        };
        let record = |tree: &mut MountedTree, batch: &nsh_core::MountBatch| {
            for item in &batch.items {
                match item {
                    MountItem::Create { id, kind } => {
                        tree.kinds.insert(*id, *kind);
                    }
                    MountItem::Insert {
                        parent,
                        child,
                        before,
                    } => {
                        let list = tree.children.entry(*parent).or_default();
                        list.retain(|c| c != child);
                        match before.and_then(|b| list.iter().position(|c| *c == b)) {
                            Some(pos) => list.insert(pos, *child),
                            None => list.push(*child),
                        }
                    }
                    MountItem::Frame { id, frame } => {
                        tree.frames.insert(*id, *frame);
                    }
                    MountItem::ScrollConfig { id, horizontal, .. } => {
                        tree.scroll_horiz.insert(*id, *horizontal);
                    }
                    MountItem::ScrollContent { id, size } => {
                        tree.scroll_contents.insert(*id, *size);
                    }
                    MountItem::ViewProps { id, props } => {
                        tree.pointer_events.insert(*id, props.pointer_events);
                    }
                    _ => {}
                }
            }
        };
        let pump = |core: &mut Core<ApproxTextEngine>, tree: &mut MountedTree| {
            let mut evs = Vec::new();
            for _ in 0..64 {
                let batch = core.commit();
                if let Some(b) = &batch {
                    record(tree, b);
                }
                evs.clear();
                core.take_events(&mut evs);
                let mut any = false;
                for ev in evs.drain(..) {
                    if let nsh_core::element::JsEvent::Event(id, kind, payload) = ev {
                        any |= core.component_event(id, kind, &payload);
                    }
                }
                if batch.is_none() && !any && !core.has_paced_mounts() {
                    break;
                }
            }
        };

        pump(&mut core, &mut tree);
        core.component_event(
            2,
            EventKind::Press,
            &EventPayload::Press(PressEvent::default()),
        );
        pump(&mut core, &mut tree);
        // Under nsh 0.6.0, below-the-fold widgets stay culled until scrolled into view
        // or culling is turned off; drain the culled batch so `sync_culled` mounts them.
        core.set_content_culling(false);
        pump(&mut core, &mut tree);
        tree
    }

    // Finds every `editor()` outer view in `tree`: a `ViewKind::View` whose first
    // child is `ed.vscroll` (`ViewKind::Scroll`) and whose next sibling is the
    // full-viewport `Layer::Paint` (`ViewKind::View`) + `sink` (`ViewKind::Input`).
    let find_editor_paint_overlays = |tree: &MountedTree| -> Vec<(NodeId, NodeId, NodeId)> {
        let mut found = Vec::new();
        for (&outer_id, kids) in &tree.children {
            if tree.kinds.get(&outer_id) != Some(&ViewKind::View) || kids.len() < 3 {
                continue;
            }
            let has_sink = kids
                .iter()
                .any(|k| tree.kinds.get(k) == Some(&ViewKind::Input));
            if tree.kinds.get(&kids[0]) == Some(&ViewKind::Scroll)
                && tree.kinds.get(&kids[1]) == Some(&ViewKind::View)
                && has_sink
            {
                found.push((outer_id, kids[0], kids[1]));
            }
        }
        found
    };

    assert_eq!(super::editor_box_height(&sample_json, 0.0), 286.0);

    // 1. Verify nsh 0.5.1+ upstream fix: even a raw editor inside a culled scroll
    //    now emits `MountItem::ViewProps { pointer_events: None }` on `Layer::Paint`.
    let unguarded = drive_select_flow::<UnguardedRawEditorHarness>();
    let unguarded_editors = find_editor_paint_overlays(&unguarded);
    assert_eq!(unguarded_editors.len(), 1);
    let (_, _, unguarded_paint_id) = unguarded_editors[0];
    assert_eq!(
        unguarded.pointer_events.get(&unguarded_paint_id),
        Some(&PointerEvents::None),
        "nsh 0.5.1+ emits ViewProps(pointer_events: None) on Layer::Paint for raw editor inside culled scroll"
    );

    // 2. Verify our guarded setup: EVERY editor's `Layer::Paint` overlay receives
    //    `MountItem::ViewProps { pointer_events: None }` (so wheel/trackpad/mouse
    //    reach `ed.vscroll` instead of falling through to the outer `detail_pane`),
    //    and every scrollable widget has `ScrollContent > Frame` along its scroll axis.
    let guarded = drive_select_flow::<GuardedScrollHarness>();
    let guarded_editors = find_editor_paint_overlays(&guarded);
    assert_eq!(
        guarded_editors.len(),
        2,
        "expected code_body + compose_editor to mount"
    );
    for (outer_id, vscroll_id, paint_id) in &guarded_editors {
        assert_eq!(
            guarded.pointer_events.get(paint_id),
            Some(&PointerEvents::None),
            "editor (outer={outer_id}) Layer::Paint ({paint_id}) MUST mount pointer_events: None so vscroll ({vscroll_id}) receives wheel/trackpad!"
        );
        let vf = guarded.frames[vscroll_id];
        let vc = guarded.scroll_contents[vscroll_id];
        assert!(
            vc.height.0 > vf.size.height.0,
            "editor vscroll {vscroll_id} must overflow vertically: content={vc:?}, frame={vf:?}"
        );
    }

    // Also check outer scroll, plain/hex inner scroll, diff_view vscroll, and horizontal swimlane scroll!
    let vertical_overflowing = guarded
        .frames
        .iter()
        .filter(|(id, f)| {
            guarded.kinds.get(id) == Some(&ViewKind::Scroll)
                && !guarded.scroll_horiz.get(id).copied().unwrap_or(false)
                && guarded
                    .scroll_contents
                    .get(id)
                    .is_some_and(|c| c.height.0 > f.size.height.0)
        })
        .count();
    assert!(
        vertical_overflowing >= 5,
        "expected outer scroll + code_body + compose_editor + plain scroll + diff_view vscroll to all overflow vertically, got {vertical_overflowing}"
    );
    let horizontal_overflowing = guarded
        .frames
        .iter()
        .filter(|(id, f)| {
            guarded.kinds.get(id) == Some(&ViewKind::Scroll)
                && guarded.scroll_horiz.get(id).copied().unwrap_or(false)
                && guarded
                    .scroll_contents
                    .get(id)
                    .is_some_and(|c| c.width.0 > f.size.width.0)
        })
        .count();
    assert!(
        horizontal_overflowing >= 1,
        "expected swimlane horizontal scroll to overflow horizontally, got {horizontal_overflowing}"
    );
}

#[test]
fn json_tree_builds_pointer_ids_root_only_expansion_search_cap_and_copy() {
    let raw = br#"{
        "statusCode": 400,
        "category": "validate",
        "extra": {
            "route": "/utils/v1/recognize",
            "a/b~c": true
        },
        "tags": ["alpha", null],
        "empty": {}
    }"#;
    let parsed = crate::present::parse_json_body(Some("application/json"), raw).expect("json");
    assert_eq!(parsed["statusCode"], 400);

    // GraphQL JSON & MessagePack also parse; form & truncated/non-JSON reject.
    let gql = br#"{"operationName":"GetUser","query":"query GetUser { id }","variables":{"id":1}}"#;
    assert!(crate::present::parse_json_body(Some("application/json"), gql).is_some());
    assert!(
        crate::present::parse_json_body(Some("application/x-msgpack"), b"\x82\xa2ok\xc3\xa1n\x07")
            .is_some()
    );
    assert!(
        crate::present::parse_json_body(Some("application/x-www-form-urlencoded"), b"a=1&b=2")
            .is_none()
    );
    assert!(
        crate::present::parse_json_body(Some("application/json"), br#"{"truncated":"#).is_none()
    );

    // Default unfiltered tree: expands only `$` one level; nested `extra` and `tags` start collapsed.
    let view = super::build_json_tree_view(&parsed, "");
    assert_eq!(view.auto_expand_ids, vec![SharedString::from("$")]);
    assert_eq!(view.items.len(), 1);
    assert_eq!(view.items[0].id.as_ref(), "$");
    assert_eq!(view.items[0].children.len(), 5);
    assert_eq!(
        super::visible_tree_row_count(&view.items, &view.auto_expand_ids),
        6,
        "root + 5 direct children visible; grandchildren under `extra` and `tags` remain collapsed"
    );

    // RFC 6901 pointer IDs & copy behavior (scalars unquoted, containers pretty JSON).
    assert!(view.meta.contains_key("$/extra/a~1b~0c"));
    assert_eq!(
        super::copy_json_node_value(&parsed, "$/category").as_deref(),
        Some("validate")
    );
    assert_eq!(
        super::copy_json_node_value(&parsed, "$/statusCode").as_deref(),
        Some("400")
    );
    assert_eq!(
        super::copy_json_node_value(&parsed, "$/tags/1").as_deref(),
        Some("null")
    );
    assert_eq!(
        super::copy_json_node_value(&parsed, "$/extra/a~1b~0c").as_deref(),
        Some("true")
    );
    let extra_copy = super::copy_json_node_value(&parsed, "$/extra").expect("extra object");
    assert!(extra_copy.contains("\"route\": \"/utils/v1/recognize\""));
    assert_eq!(super::copy_json_node_value(&parsed, "__truncated:$"), None);

    // Search filters to matching leaves & auto-expands ancestor paths down to matches.
    let jb = super::JsonBody {
        flow_id: 1,
        body_hash: 42,
        value: std::sync::Arc::new(parsed.clone()),
    };
    assert_eq!(super::json_tree_search_hits(&jb, "recognize"), 1);
    let searched = super::build_json_tree_view(&parsed, "recognize");
    assert!(searched.auto_expand_ids.contains(&SharedString::from("$")));
    assert!(
        searched
            .auto_expand_ids
            .contains(&SharedString::from("$/extra"))
    );
    assert_eq!(
        super::visible_tree_row_count(&searched.items, &searched.auto_expand_ids),
        3,
        "only `$` -> `extra` -> `route` should remain visible when searching `recognize`"
    );

    // Node budget cap prevents pathological huge JSON from stalling UI.
    let huge = serde_json::Value::Array((0..25).map(|i| serde_json::json!(i)).collect());
    let capped = super::build_json_tree_with_limit(&huge, "", 5);
    assert!(capped.items[0].children.len() <= 5);
    assert!(
        capped
            .meta
            .values()
            .any(|m| m.kind == super::JsonNodeKind::Truncated)
    );
}

#[tokio::test]
async fn json_tree_panel_mounts_inside_culled_detail_scroll_with_overflow() {
    use nsh_core::prelude::*;
    use nsh_core::{
        ApproxTextEngine, Core, EventKind, EventPayload, MountItem, NodeId, Op, PressEvent, Prop,
        ROOT_ID, Tag, ViewKind,
    };

    #[component]
    fn CulledJsonTreeHarness() -> impl IntoElement {
        provide_context(signal(Some(crate::i18n::Lang::En)));
        let open = signal(false);
        let search = signal(String::new());
        let obj: serde_json::Map<String, serde_json::Value> = (0..24)
            .map(|i| {
                (
                    format!("field_{i:02}"),
                    serde_json::json!(format!("val_{i}")),
                )
            })
            .collect();
        let json = signal(Some(super::JsonBody {
            flow_id: 7,
            body_hash: 99,
            value: std::sync::Arc::new(serde_json::Value::Object(obj)),
        }));
        view()
            .w(px(540.0))
            .h(px(600.0))
            .on_press(move || open.set(true))
            .child(
                scroll().flex_1().w_full().child(
                    super::uncull_nested_scroll(view())
                        .w_full()
                        .p_3()
                        .child(when(
                            move || open.get(),
                            move || super::json_tree_panel(json, search, |_| {}),
                        )),
                ),
            )
    }

    let mut core = Core::new(ApproxTextEngine, Size::new(px(540.0), px(600.0)));
    core.set_content_culling(true);
    let mut registry = Registry::new();
    registry.register::<CulledJsonTreeHarness>();
    core.set_components(registry);
    for op in [
        Op::Create {
            id: 2,
            tag: Tag::View,
        },
        Op::SetProp {
            id: 2,
            prop: Prop::Component(std::any::type_name::<CulledJsonTreeHarness>().into()),
        },
        Op::Insert {
            parent: ROOT_ID,
            child: 2,
            before: None,
        },
    ] {
        core.apply(op).expect("apply op");
    }

    let mut kinds = std::collections::HashMap::<NodeId, ViewKind>::new();
    let mut frames = std::collections::HashMap::<NodeId, Bounds<Points>>::new();
    let mut scroll_contents = std::collections::HashMap::<NodeId, Size<Points>>::new();
    let mut evs = Vec::new();
    let mut pump = |core: &mut Core<ApproxTextEngine>| {
        for _ in 0..12 {
            let batch = core.commit();
            if let Some(b) = &batch {
                for item in &b.items {
                    match item {
                        MountItem::Create { id, kind } => {
                            kinds.insert(*id, *kind);
                        }
                        MountItem::Frame { id, frame } => {
                            frames.insert(*id, *frame);
                        }
                        MountItem::ScrollContent { id, size } => {
                            scroll_contents.insert(*id, *size);
                        }
                        _ => {}
                    }
                }
            }
            evs.clear();
            core.take_events(&mut evs);
            let mut any = false;
            for ev in evs.drain(..) {
                if let nsh_core::element::JsEvent::Event(id, kind, payload) = ev {
                    any |= core.component_event(id, kind, &payload);
                }
            }
            if batch.is_none() && !any {
                break;
            }
        }
    };

    pump(&mut core);
    core.component_event(
        2,
        EventKind::Press,
        &EventPayload::Press(PressEvent::default()),
    );
    pump(&mut core);

    let tree_vscrolls = frames
        .iter()
        .filter(|(id, f)| {
            kinds.get(id) == Some(&ViewKind::Scroll)
                && f.size.height.0 > 0.0
                && scroll_contents
                    .get(id)
                    .is_some_and(|c| c.height.0 > f.size.height.0)
        })
        .count();
    assert!(
        tree_vscrolls >= 1,
        "expected json_tree_panel internal Scroll to mount and overflow when 25 rows are expanded"
    );
}

#[test]
fn web_poll_delay_and_long_poll_url_behave_cleanly() {
    use crate::ui::web_poll::{
        WEB_ACTIVE_FLOOR_MS, WEB_LONG_POLL_WAIT_MS, build_web_state_url, web_poll_delay_ms,
    };

    assert_eq!(build_web_state_url(false, None, false), "api/state");
    assert_eq!(build_web_state_url(true, None, false), "api/state?lite=1");
    assert_eq!(
        build_web_state_url(true, Some((3, 9)), true),
        "api/state?lite=1"
    );
    assert_eq!(
        build_web_state_url(true, Some((3, 9)), false),
        format!("api/state?lite=1&since_gen=3&since_rev=9&wait_ms={WEB_LONG_POLL_WAIT_MS}")
    );

    // State changes enforce the 400ms minimum floor; long-poll idle waits settle cleanly;
    // network errors back off 1s -> 2s -> 4s.
    assert_eq!(web_poll_delay_ms(true, true, true, 0), WEB_ACTIVE_FLOOR_MS);
    assert_eq!(web_poll_delay_ms(false, true, true, 5), WEB_ACTIVE_FLOOR_MS);
    assert_eq!(web_poll_delay_ms(true, false, true, 0), 500);
    assert_eq!(web_poll_delay_ms(false, false, true, 0), 1_500);
    assert_eq!(web_poll_delay_ms(true, false, false, 0), 1_000);
    assert_eq!(web_poll_delay_ms(true, false, false, 1), 1_000);
    assert_eq!(web_poll_delay_ms(true, false, false, 2), 2_000);
    assert_eq!(web_poll_delay_ms(true, false, false, 4), 4_000);
}

#[test]
fn unified_error_filter_matches_4xx_5xx_and_transport_err_while_fail_stays_transport_only() {
    use crate::session::{FlowDraft, Protocol, Session};

    let mut session = Session::default();
    let ok_id = session.open(FlowDraft {
        method: "GET".into(),
        url: "http://example.com/ok".into(),
        host: "example.com".into(),
        path: "/ok".into(),
        protocol: Protocol::Http11,
        request_headers: Vec::new(),
    });
    session.set_response(ok_id, 200, Vec::new(), None);
    let client_err_id = session.open(FlowDraft {
        method: "GET".into(),
        url: "http://example.com/missing".into(),
        host: "example.com".into(),
        path: "/missing".into(),
        protocol: Protocol::Http11,
        request_headers: Vec::new(),
    });
    session.set_response(client_err_id, 404, Vec::new(), None);
    let server_err_id = session.open(FlowDraft {
        method: "GET".into(),
        url: "http://example.com/gw".into(),
        host: "example.com".into(),
        path: "/gw".into(),
        protocol: Protocol::Http11,
        request_headers: Vec::new(),
    });
    session.set_response(server_err_id, 502, Vec::new(), None);
    let transport_err_id = session.open(FlowDraft {
        method: "GET".into(),
        url: "http://example.com/down".into(),
        host: "example.com".into(),
        path: "/down".into(),
        protocol: Protocol::Http11,
        request_headers: Vec::new(),
    });
    session.set_error(transport_err_id, "connection refused");

    assert_eq!(
        session.ids_matching("status:4xx|5xx|err"),
        vec![client_err_id, server_err_id, transport_err_id]
    );
    assert_eq!(session.ids_matching("失败"), vec![transport_err_id]);
    assert_eq!(session.ids_matching("err"), vec![transport_err_id]);
}

#[test]
fn web_cached_flow_detail_publishes_loading_then_hydrates_cleanly() {
    use nsh_core::element::{Runtime, create_root};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::{Duration, SystemTime};

    static SEQ: AtomicUsize = AtomicUsize::new(0);
    let n = SEQ.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("zniff-ui-detail-test-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let ca = Arc::new(crate::ca::Ca::open(&dir).unwrap());
    let hub = Hub::new(ca).unwrap();
    let _ = std::fs::remove_dir_all(&dir);

    let t0 = SystemTime::UNIX_EPOCH + Duration::from_millis(10_000);
    let post_view = crate::session::CachedFlowView {
        id: 1,
        rev: 3,
        protocol: Protocol::Http11,
        method: "POST".into(),
        host: "172.18.0.1:19876".into(),
        path: "/api/v1/orders".into(),
        url: "http://172.18.0.1:19876/api/v1/orders".into(),
        client_ip: "127.0.0.1".into(),
        conn_info: crate::session::ConnInfo::default(),
        status: Some(201),
        error: None,
        started: t0,
        response_at: Some(t0 + Duration::from_millis(1)),
        finished_at: Some(t0 + Duration::from_millis(2)),
        request_bytes: 38,
        response_bytes: 61,
        pinned: false,
        rule_hit: String::new(),
        comment: String::new(),
        relay_source: None,
    };
    let inflight_v1 = crate::session::CachedFlowView {
        id: 2,
        rev: 10,
        protocol: Protocol::Http11,
        method: "POST".into(),
        host: "172.18.0.1:19876".into(),
        path: "/upload".into(),
        url: "http://172.18.0.1:19876/upload".into(),
        client_ip: "127.0.0.1".into(),
        conn_info: crate::session::ConnInfo::default(),
        status: None,
        error: None,
        started: t0,
        response_at: None,
        finished_at: None,
        request_bytes: 16,
        response_bytes: 0,
        pinned: false,
        rule_hit: String::new(),
        comment: String::new(),
        relay_source: None,
    };
    let empty_view = crate::session::CachedFlowView {
        id: 3,
        rev: 1,
        protocol: Protocol::Http11,
        method: "DELETE".into(),
        host: "172.18.0.1:19876".into(),
        path: "/api/v1/orders/1".into(),
        url: "http://172.18.0.1:19876/api/v1/orders/1".into(),
        client_ip: "127.0.0.1".into(),
        conn_info: crate::session::ConnInfo::default(),
        status: Some(204),
        error: None,
        started: t0,
        response_at: Some(t0 + Duration::from_millis(1)),
        finished_at: Some(t0 + Duration::from_millis(1)),
        request_bytes: 0,
        response_bytes: 0,
        pinned: false,
        rule_hit: String::new(),
        comment: String::new(),
        relay_source: None,
    };
    hub.with_session(|session| {
        session.replace_cached_views(vec![post_view.clone(), inflight_v1.clone(), empty_view]);
    });

    let runtime = Runtime::default();
    let _entered = runtime.enter();
    let (owner, ()) = create_root(move || {
        let shown = super::Shown {
            phase: signal(super::Phase::Empty),
            method: signal(String::new()),
            url: signal(String::new()),
            protocol: signal(String::new()),
            rules: signal(String::new()),
            status: signal(None),
            error: signal(String::new()),
            request_headers: signal(Vec::new()),
            response_headers: signal(Vec::new()),
            request_body: signal(String::new()),
            response_body: signal(String::new()),
            request_hex: signal(Vec::new()),
            response_hex: signal(Vec::new()),
            pane_width: signal(0.0),
            request_language: signal(String::new()),
            response_language: signal(String::new()),
            request_doc: signal(None),
            response_doc: signal(None),
            req_doc_preview: signal(false),
            res_doc_preview: signal(false),
            request_json: signal(None),
            response_json: signal(None),
            req_json_tree: signal(false),
            res_json_tree: signal(false),
            request_form: signal(Vec::new()),
            request_note: signal(String::new()),
            response_note: signal(String::new()),
            metrics: signal(String::new()),
            timing_bar: signal(crate::present::TimingBreakdown::default()),
            query: signal(Vec::new()),
            preview: signal(String::new()),
            preview_w: signal(0.0),
            preview_h: signal(0.0),
            image_caption: signal(String::new()),
            pretty: signal(true),
            default_json_tree: signal(false),
            messages: signal(Vec::new()),
            llm_stream: signal(None),
            msg_llm_view: signal(true),
            diff: signal(String::new()),
            diff_old: signal(String::new()),
            diff_new: signal(String::new()),
            diff_scope: signal(0_u8),
            pinned: signal(false),
            comment: signal(String::new()),
            editing_comment: signal(false),
            repeating: signal(false),
            repeat_count: signal("1".into()),
            repeat_interval: signal("0".into()),
            repeat_concurrency: signal("1".into()),
            req_search: signal(String::new()),
            res_search: signal(String::new()),
            msg_search: signal(String::new()),
            msg_dir: signal(0),
            msg_kind: signal(0),
            relay: signal(String::new()),
            request_offset: signal(0),
            response_offset: signal(0),
            request_window: signal(super::TextWindow::default()),
            response_window: signal(super::TextWindow::default()),
            req_headers_loading: signal(false),
            res_headers_loading: signal(false),
            req_body_loading: signal(false),
            res_body_loading: signal(false),
            messages_loading: signal(false),
            diff_loading: signal(false),
        };

        // 1. Before detail arrives for POST 201 (req 38 B, res 61 B), loading flags are on.
        super::publish_detail(&shown, &hub, Some(1), Lang::En, None);
        assert!(shown.phase.get() == super::Phase::Ready);
        assert!(shown.req_headers_loading.get());
        assert!(shown.res_headers_loading.get());
        assert!(shown.req_body_loading.get());
        assert!(shown.res_body_loading.get());
        assert!(!shown.messages_loading.get());
        assert!(!shown.diff_loading.get());

        // 2. After fill_cached_detail, loading clears and bodies + headers populate.
        hub.with_session(|session| {
            session.fill_cached_detail(crate::session::CachedFlowDetail {
                id: 1,
                rev: 3,
                request_headers: vec![("Content-Type".into(), "application/json".into())],
                response_headers: vec![("Content-Type".into(), "application/json".into())],
                request_body: br#"{"item":"book"}"#.to_vec(),
                response_body: br#"{"ok":true}"#.to_vec(),
                request_truncated: false,
                response_truncated: false,
                request_encoding_error: None,
                response_encoding_error: None,
                request_expired: None,
                response_expired: None,
                messages: Vec::new(),
                messages_truncated: false,
            });
        });
        super::publish_detail(&shown, &hub, Some(1), Lang::En, None);
        assert!(!shown.req_headers_loading.get());
        assert!(!shown.res_headers_loading.get());
        assert!(!shown.req_body_loading.get());
        assert!(!shown.res_body_loading.get());
        assert_eq!(shown.request_headers.with(Vec::len), 1);
        assert_eq!(shown.response_headers.with(Vec::len), 1);
        assert!(shown.request_body.get().contains("\"item\""));
        assert!(shown.response_body.get().contains("\"ok\""));

        // 3. In-flight flow hydrated with request detail preserves request when response arrives later.
        hub.with_session(|session| {
            session.fill_cached_detail(crate::session::CachedFlowDetail {
                id: 2,
                rev: 10,
                request_headers: vec![("Host".into(), "172.18.0.1:19876".into())],
                response_headers: Vec::new(),
                request_body: b"{\"part\":1}".to_vec(),
                response_body: Vec::new(),
                request_truncated: false,
                response_truncated: false,
                request_encoding_error: None,
                response_encoding_error: None,
                request_expired: None,
                response_expired: None,
                messages: Vec::new(),
                messages_truncated: false,
            });
        });
        super::publish_detail(&shown, &hub, Some(2), Lang::En, None);
        assert!(!shown.req_headers_loading.get());
        assert!(!shown.req_body_loading.get());
        assert!(!shown.res_headers_loading.get());
        assert!(!shown.res_body_loading.get());

        let mut inflight_v2 = inflight_v1;
        inflight_v2.rev = 12;
        inflight_v2.status = Some(201);
        inflight_v2.response_at = Some(t0 + Duration::from_millis(5));
        inflight_v2.finished_at = Some(t0 + Duration::from_millis(7));
        inflight_v2.response_bytes = 61;
        hub.with_session(|session| {
            let empty_v = session.get(3).unwrap();
            let empty_summary = crate::session::CachedFlowView {
                id: 3,
                rev: 1,
                protocol: empty_v.protocol,
                method: empty_v.method.clone(),
                host: empty_v.host.clone(),
                path: empty_v.path.clone(),
                url: empty_v.url.clone(),
                client_ip: empty_v.client_ip.clone(),
                conn_info: empty_v.conn_info.clone(),
                status: empty_v.status,
                error: empty_v.error.clone(),
                started: empty_v.started,
                response_at: empty_v.response_at,
                finished_at: empty_v.finished_at,
                request_bytes: empty_v.request_bytes,
                response_bytes: empty_v.response_bytes,
                pinned: empty_v.pinned,
                rule_hit: empty_v.rule_hit.clone(),
                comment: empty_v.comment.clone(),
                relay_source: empty_v.relay_source.clone(),
            };
            session.replace_cached_views(vec![inflight_v2, empty_summary]);
        });
        super::publish_detail(&shown, &hub, Some(2), Lang::En, None);
        assert!(!shown.req_headers_loading.get());
        assert!(!shown.req_body_loading.get());
        assert!(shown.res_headers_loading.get());
        assert!(shown.res_body_loading.get());

        // 4. Empty 0 B flow advances rev on empty detail payload and clears header loading flags.
        super::publish_detail(&shown, &hub, Some(3), Lang::En, None);
        assert!(shown.req_headers_loading.get());
        assert!(shown.res_headers_loading.get());
        assert!(!shown.req_body_loading.get());
        assert!(!shown.res_body_loading.get());
        let rev_before = hub.with_session(|session| session.get(3).unwrap().rev);
        hub.with_session(|session| {
            session.fill_cached_detail(crate::session::CachedFlowDetail {
                id: 3,
                rev: 1,
                request_headers: Vec::new(),
                response_headers: Vec::new(),
                request_body: Vec::new(),
                response_body: Vec::new(),
                request_truncated: false,
                response_truncated: false,
                request_encoding_error: None,
                response_encoding_error: None,
                request_expired: None,
                response_expired: None,
                messages: Vec::new(),
                messages_truncated: false,
            });
        });
        let rev_after = hub.with_session(|session| session.get(3).unwrap().rev);
        assert!(rev_after > rev_before);
        super::publish_detail(&shown, &hub, Some(3), Lang::En, None);
        assert!(!shown.req_headers_loading.get());
        assert!(!shown.res_headers_loading.get());
    });
    owner.dispose();
}

#[test]
fn toolbar_side_padding_reserves_title_bar_controls_on_all_desktop_sides() {
    assert_eq!(
        super::toolbar_side_padding(0.0, 0.0),
        (super::LIST_INSET, super::TOOLBAR_PAD_END)
    );
    assert_eq!(
        super::toolbar_side_padding(78.0, 0.0),
        (78.0 + super::TITLE_BAR_CONTROLS_GAP, super::TOOLBAR_PAD_END)
    );
    assert_eq!(
        super::toolbar_side_padding(0.0, 104.0),
        (super::LIST_INSET, 104.0 + super::TITLE_BAR_CONTROLS_GAP)
    );
    assert_eq!(
        super::toolbar_side_padding(0.0, 138.0),
        (super::LIST_INSET, 138.0 + super::TITLE_BAR_CONTROLS_GAP)
    );
    assert_eq!(
        super::toolbar_side_padding(36.0, 104.0),
        (
            36.0 + super::TITLE_BAR_CONTROLS_GAP,
            104.0 + super::TITLE_BAR_CONTROLS_GAP
        )
    );
    assert_eq!(
        super::toolbar_side_padding(4.0, 4.0),
        (super::LIST_INSET, super::TOOLBAR_PAD_END)
    );
    assert_eq!(super::toolbar_bar_height(0.0), super::TOOLBAR_H);
    assert_eq!(super::toolbar_bar_height(42.0), 42.0);
    assert_eq!(super::toolbar_bar_height(48.0), 48.0);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn toolbar_start_stop_clears_right_and_left_title_bar_controls() {
    use nsh_core::element::Environment;
    use nsh_core::geometry::Edges;
    use nsh_core::prelude::*;
    use nsh_core::{
        ApproxTextEngine, Core, EventKind, EventPayload, MountItem, NodeId, Op, PressEvent, Prop,
        ROOT_ID, Tag, ViewKind,
    };
    use std::cell::RefCell;

    #[derive(Default)]
    struct ToggleTrace {
        running: bool,
        toggles: usize,
    }

    thread_local! {
        static TRACE: RefCell<ToggleTrace> = RefCell::new(ToggleTrace::default());
    }

    TRACE.with(|cell| *cell.borrow_mut() = ToggleTrace::default());

    #[component]
    fn ToolbarTitleBarHarness() -> impl IntoElement {
        provide_context(signal(Some(crate::i18n::Lang::En)));
        super::provide_mono_font(false);

        let dir = std::env::temp_dir().join(format!(
            "zniff-toolbar-titlebar-test-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let ca = Arc::new(crate::ca::Ca::open(&dir).expect("ca"));
        let hub = Arc::new(Hub::new(ca).expect("hub"));
        let _ = std::fs::remove_dir_all(&dir);

        let controls = Arc::new(Controls {
            hub,
            capture: Mutex::new(None),
            status: Mutex::new(StatusMsg::Idle),
            running: Mutex::new(false),
            listen: Mutex::new(String::new()),
            listen_port: Mutex::new(8895),
            ca_status: Mutex::new(crate::ca::CaStatus::NotInstalled),
            active_external_ca: Mutex::new(None),
            external_cas: Mutex::new(Vec::new()),
            use_system_proxy: Mutex::new(false),
            proxy_applied: Mutex::new(false),
            ops: Mutex::new(()),
            listen_all: Mutex::new(false),
            update_state: Mutex::new(UpdateState::default()),
            redact_export: Mutex::new(false),
            redact_strict: Mutex::new(false),
            redact_fields: Mutex::new(String::new()),
            relay_url: Mutex::new(String::new()),
            relay_token: Mutex::new(String::new()),
            relay_push: Mutex::new(None),
            toast_seq: std::sync::atomic::AtomicU64::new(0),
            toast_queue: Mutex::new(Vec::new()),
            config_error: Mutex::new(None),
            config_path: Mutex::new(String::new()),
        });

        let status = signal(StatusMsg::Idle);
        let running = signal(false);
        let ca_status = signal(crate::ca::CaStatus::NotInstalled);
        let ca_ui = super::CaSignals {
            status: ca_status,
            active_external: signal(None),
            detected_external: signal(Vec::new()),
        };
        let use_system_proxy = signal(false);
        let rows = signal(Vec::<Arc<RowLine>>::new());
        let flow_total = signal(0_usize);
        let filter = signal(String::new());
        let pretty = signal(true);
        let shown_phase = signal(Phase::Empty);
        let share_id = signal(String::new());
        let selected = signal(None::<u64>);
        let update_state = signal(UpdateState::default());

        let profiles = ProfileSignals {
            saved: signal(Vec::new()),
            history: signal(Vec::new()),
            name: signal(String::new()),
            filter,
            filter_enabled: signal(true),
            bypass: signal(String::new()),
            rules: signal(String::new()),
            rule_groups: signal(Vec::new()),
            editing_group: signal(None),
            upstream: signal(String::new()),
            rules_on: signal(true),
            breaks_on: signal(true),
            show_tunnels: signal(false),
            proxy_auth: signal(String::new()),
            relay_url: signal(String::new()),
            relay_accept: signal(false),
            list_width: signal(660.0),
            list_height: signal(320.0),
            sidebar_width: signal(super::SIDEBAR_WIDTH_DEFAULT),
            sidebar_collapsed: signal(false),
            layout_mode: signal(crate::config::LayoutMode::Split),
            panel_open: signal(false),
            machine_settings_open: signal(false),
            about_open: signal(false),
            default_json_tree: signal(false),
            deselect_rev: signal(0),
        };
        let compose = Compose {
            composing: signal(false),
            response: signal(false),
            method: signal(String::new()),
            url: signal(String::new()),
            headers: signal(String::new()),
            body: signal(String::new()),
            status: signal(String::new()),
            draft_saved: signal((String::new(), String::new(), String::new(), String::new())),
            recent: signal(Vec::new()),
        };
        let seq_diagram = signal(present::SequenceDiagramData::default());
        let seq_viewport_w = signal(0.0_f32);
        let seq_builder = SeqBuilder {
            open: signal(false),
            items: signal(Vec::new()),
            include_response: signal(true),
            include_query: signal(false),
            output: signal(String::new()),
            diagram: seq_diagram,
            view_mode: signal(0),
            source_flowchart: signal(false),
            hover_bar: signal(None),
            redact_open: signal(false),
            redact_export: signal(false),
            redact_strict: signal(false),
            redact_fields: signal(String::new()),
            redact_flow_id: signal(None),
            viewport_w: seq_viewport_w,
            track_w: signal(0.0_f32),
            layout: computed(move || {
                seq_diagram.with(|d| swimlane_layout(d, seq_viewport_w.get()))
            }),
            flat_rows: computed(move || seq_diagram.with(build_dep_flat_rows)),
        };

        let bar = super::toolbar(
            status,
            running,
            ca_ui,
            use_system_proxy,
            || {},
            || {},
            || {},
            || {},
            || {},
            || {},
            || {},
            move || {
                let next = !running.get();
                running.set(next);
                TRACE.with(|cell| {
                    let mut trace = cell.borrow_mut();
                    trace.running = next;
                    trace.toggles += 1;
                });
            },
            controls,
            rows,
            flow_total,
            filter,
            pretty,
            shown_phase,
            share_id,
            profiles,
            compose,
            seq_builder,
            selected,
            update_state,
        );

        view().w(px(800.0)).h(px(500.0)).child(bar)
    }

    let mut core = Core::new(ApproxTextEngine, Size::new(px(800.0), px(500.0)));
    let mut registry = Registry::new();
    registry.register::<ToolbarTitleBarHarness>();
    core.set_components(registry);
    for op in [
        Op::Create {
            id: 2,
            tag: Tag::View,
        },
        Op::SetProp {
            id: 2,
            prop: Prop::Component(std::any::type_name::<ToolbarTitleBarHarness>().into()),
        },
        Op::Insert {
            parent: ROOT_ID,
            child: 2,
            before: None,
        },
    ] {
        core.apply(op).expect("apply op");
    }

    struct MountedToolbar {
        kinds: HashMap<NodeId, ViewKind>,
        children: HashMap<NodeId, Vec<NodeId>>,
        frames: HashMap<NodeId, Bounds<Points>>,
        window_drag: HashMap<NodeId, Option<bool>>,
    }

    let mut tree = MountedToolbar {
        kinds: HashMap::new(),
        children: HashMap::new(),
        frames: HashMap::new(),
        window_drag: HashMap::new(),
    };

    let pump = |core: &mut Core<ApproxTextEngine>, tree: &mut MountedToolbar| {
        let mut evs = Vec::new();
        for _ in 0..12 {
            let batch = core.commit();
            if let Some(b) = &batch {
                for item in &b.items {
                    match item {
                        MountItem::Create { id, kind } => {
                            tree.kinds.insert(*id, *kind);
                        }
                        MountItem::Insert {
                            parent,
                            child,
                            before,
                        } => {
                            let list = tree.children.entry(*parent).or_default();
                            list.retain(|c| c != child);
                            match before.and_then(|b| list.iter().position(|c| *c == b)) {
                                Some(pos) => list.insert(pos, *child),
                                None => list.push(*child),
                            }
                        }
                        MountItem::Frame { id, frame } => {
                            tree.frames.insert(*id, *frame);
                        }
                        MountItem::ViewProps { id, props } => {
                            tree.window_drag.insert(*id, props.window_drag);
                        }
                        _ => {}
                    }
                }
            }
            evs.clear();
            core.take_events(&mut evs);
            let mut any = false;
            for ev in evs.drain(..) {
                if let nsh_core::element::JsEvent::Event(id, kind, payload) = ev {
                    any |= core.component_event(id, kind, &payload);
                }
            }
            if batch.is_none() && !any {
                break;
            }
        }
    };

    pump(&mut core, &mut tree);

    let toolbar_id = tree.children[&2][0];
    assert_eq!(tree.kinds.get(&toolbar_id), Some(&ViewKind::View));
    assert_eq!(tree.window_drag.get(&toolbar_id), Some(&Some(true)));

    let toolbar_kids = &tree.children[&toolbar_id];
    let brand_id = toolbar_kids[0];
    let start_btn_id = *toolbar_kids.last().expect("Start button child");
    assert_eq!(
        tree.window_drag.get(&start_btn_id),
        Some(&Some(false)),
        "Start/Stop button must opt out of window_drag"
    );

    // 1. Default environment (`title_bar` = 0): standard edge insets.
    assert!((tree.frames[&brand_id].origin.x.0 - super::LIST_INSET).abs() < 0.5);
    let default_btn = tree.frames[&start_btn_id];
    assert!(
        (default_btn.origin.x.0 + default_btn.size.width.0 - (800.0 - super::TOOLBAR_PAD_END))
            .abs()
            < 0.5
    );

    // 2. Simulate Linux GTK4 asynchronous `WindowControls` measurement (`right = 104pt` at 800x500).
    let mut env = Environment {
        window: Size::new(px(800.0), px(500.0)),
        title_bar: Edges {
            top: Points(42.0),
            right: Points(104.0),
            bottom: Points(0.0),
            left: Points(0.0),
        },
        ..Default::default()
    };
    core.set_environment(env);
    pump(&mut core, &mut tree);

    let linux_toolbar = tree.frames[&toolbar_id];
    let linux_btn = tree.frames[&start_btn_id];
    let linux_btn_right = linux_toolbar.origin.x.0 + linux_btn.origin.x.0 + linux_btn.size.width.0;
    assert!(
        linux_btn_right <= 800.0 - 104.0 - super::TITLE_BAR_CONTROLS_GAP + 0.5,
        "Linux Start button right edge ({linux_btn_right}) must stay strictly clear of overlaid GTK WindowControls [696.0, 800.0]"
    );
    assert!(
        (tree.frames[&brand_id].origin.x.0 - super::LIST_INSET).abs() < 0.5,
        "left edge stays at LIST_INSET when title_bar.left is 0"
    );

    // 3. Simulate Windows caption buttons (`right = 138pt`).
    env.title_bar = Edges {
        top: Points(42.0),
        right: Points(138.0),
        bottom: Points(0.0),
        left: Points(0.0),
    };
    core.set_environment(env);
    pump(&mut core, &mut tree);

    let win_btn = tree.frames[&start_btn_id];
    let win_btn_right = win_btn.origin.x.0 + win_btn.size.width.0;
    assert!(
        win_btn_right <= 800.0 - 138.0 - super::TITLE_BAR_CONTROLS_GAP + 0.5,
        "Windows Start button right edge ({win_btn_right}) must clear caption buttons"
    );

    // 4. Simulate macOS traffic lights (`left = 78pt`, `right = 0pt`) and tall bar (`top = 48pt`).
    env.title_bar = Edges {
        top: Points(48.0),
        right: Points(0.0),
        bottom: Points(0.0),
        left: Points(78.0),
    };
    core.set_environment(env);
    pump(&mut core, &mut tree);

    assert!(
        (tree.frames[&toolbar_id].size.height.0 - 48.0).abs() < 0.5,
        "toolbar height expands to cover taller native title bar"
    );
    assert!(
        tree.frames[&brand_id].origin.x.0 >= 78.0 + super::TITLE_BAR_CONTROLS_GAP - 0.5,
        "brand must clear macOS traffic lights on the left"
    );
    let mac_btn = tree.frames[&start_btn_id];
    assert!(
        (mac_btn.origin.x.0 + mac_btn.size.width.0 - (800.0 - super::TOOLBAR_PAD_END)).abs() < 0.5
    );

    // 5. Pressing Start/Stop button node toggles capture state twice cleanly.
    assert!(core.component_event(
        start_btn_id,
        EventKind::Press,
        &EventPayload::Press(PressEvent::default()),
    ));
    pump(&mut core, &mut tree);
    TRACE.with(|cell| {
        let trace = cell.borrow();
        assert!(trace.running);
        assert_eq!(trace.toggles, 1);
    });

    assert!(core.component_event(
        start_btn_id,
        EventKind::Press,
        &EventPayload::Press(PressEvent::default()),
    ));
    pump(&mut core, &mut tree);
    TRACE.with(|cell| {
        let trace = cell.borrow();
        assert!(!trace.running);
        assert_eq!(trace.toggles, 2);
    });
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn update_pill_tooltip_shows_release_notes_and_truncates_cleanly() {
    let with_notes = UpdateState {
        phase: UpdatePhase::Idle,
        nudge: Some("v0.2.0".into()),
        ignored: None,
        notes: Some("- Fix download\n- Auto restart".into()),
        last_check: None,
    };
    let zh = super::format_update_tooltip(&with_notes, Lang::Zh);
    assert!(zh.contains("v0.2.0"));
    assert!(zh.contains("- Fix download\n- Auto restart"));

    let no_notes = UpdateState {
        phase: UpdatePhase::Idle,
        nudge: Some("v0.2.0".into()),
        ignored: None,
        notes: None,
        last_check: None,
    };
    assert_eq!(
        super::format_update_tooltip(&no_notes, Lang::En),
        "New version v0.2.0 · Click to update and restart"
    );

    let many_lines = (1..=25)
        .map(|i| format!("- item {i}"))
        .collect::<Vec<_>>()
        .join("\n");
    let full = super::format_update_tooltip(
        &UpdateState {
            notes: Some(many_lines),
            ..with_notes
        },
        Lang::En,
    );
    assert!(full.contains("- item 1"));
    assert!(full.contains("- item 25"));
}

#[test]
fn about_modal_copyright_and_system_info_include_expected_fields() {
    assert_eq!(
        super::copyright_notice("2026"),
        "© 2026 Tree xie. All rights reserved."
    );
    assert_eq!(
        super::copyright_notice("2027"),
        "© 2026 - 2027 Tree xie. All rights reserved."
    );
    assert!(!env!("ZNIFF_GIT_SHA").is_empty());
    assert_eq!(env!("ZNIFF_BUILD_YEAR").len(), 4);
    assert!(super::about_logo_uri().starts_with("data:image/png;base64,"));
    assert_eq!(
        super::parse_macos_gpu_info("Graphics/Displays:\n  Chipset Model: Apple M3 Max\n")
            .as_deref(),
        Some("Apple M3 Max")
    );
    assert_eq!(super::parse_macos_gpu_info("  Chipset Model:   \n"), None);
    assert_eq!(
        super::parse_windows_gpu_info("Name\r\nNVIDIA GeForce RTX 4070\r\n\r\n").as_deref(),
        Some("NVIDIA GeForce RTX 4070")
    );
    assert_eq!(super::parse_windows_gpu_info("Name\r\n  \r\n"), None);
    assert_eq!(
        super::parse_linux_gpu_info(
            "00:02.0 VGA compatible controller: Intel Corporation UHD Graphics 770\n"
        )
        .as_deref(),
        Some("Intel Corporation UHD Graphics 770")
    );
    assert_eq!(super::parse_linux_gpu_info("   \n"), None);
}

#[test]
fn wide_three_layout_places_request_and_response_side_by_side() {
    use nsh_core::prelude::*;
    use nsh_core::{ApproxTextEngine, Core, Op, Prop, ROOT_ID, Tag};
    use std::cell::RefCell;

    type SideBounds = (Option<Bounds<Points>>, Option<Bounds<Points>>);
    thread_local! {
        static BOUNDS: RefCell<SideBounds> = const { RefCell::new((None, None)) };
    }

    #[component]
    fn WideThreeHarness() -> impl IntoElement {
        let layout_mode = signal(crate::config::LayoutMode::TopBottom);
        let side_by_side = move || layout_mode.get() == crate::config::LayoutMode::TopBottom;
        view().w(px(600.0)).h(px(600.0)).child(
            scroll().w_full().h_full().child(
                super::uncull_nested_scroll(view())
                    .pl_3p5()
                    .pr_5()
                    .py_3p5()
                    .gap_3p5()
                    .child(
                        when(side_by_side, move || {
                            view()
                                .w_full()
                                .flex_row()
                                .items_start()
                                .gap_3p5()
                                .child(
                                    view()
                                        .flex_1()
                                        .min_w(px(0.0))
                                        .on_layout(move |b: &Bounds<Points>| {
                                            BOUNDS.with(|cell| cell.borrow_mut().0 = Some(*b));
                                        })
                                        .child(view().w_full().h(px(100.0))),
                                )
                                .child(
                                    view()
                                        .flex_1()
                                        .min_w(px(0.0))
                                        .on_layout(move |b: &Bounds<Points>| {
                                            BOUNDS.with(|cell| cell.borrow_mut().1 = Some(*b));
                                        })
                                        .child(view().w_full().h(px(120.0))),
                                )
                        })
                        .otherwise(move || view().w_full().h(px(200.0))),
                    ),
            ),
        )
    }

    let mut core = Core::new(ApproxTextEngine, Size::new(px(600.0), px(600.0)));
    let mut registry = Registry::new();
    registry.register::<WideThreeHarness>();
    core.set_components(registry);
    for op in [
        Op::Create {
            id: 2,
            tag: Tag::View,
        },
        Op::SetProp {
            id: 2,
            prop: Prop::Component(std::any::type_name::<WideThreeHarness>().into()),
        },
        Op::Insert {
            parent: ROOT_ID,
            child: 2,
            before: None,
        },
    ] {
        core.apply(op).expect("apply op");
    }

    for _ in 0..8 {
        let batch = core.commit();
        let mut evs = Vec::new();
        core.take_events(&mut evs);
        let mut any = false;
        for ev in evs {
            if let nsh_core::element::JsEvent::Event(id, kind, payload) = ev {
                any |= core.component_event(id, kind, &payload);
            }
        }
        if batch.is_none() && !any {
            break;
        }
    }

    let (l, r) = BOUNDS.with(|cell| *cell.borrow());
    let l = l.expect("left_bounds");
    let r = r.expect("right_bounds");
    assert!(
        l.size.width.0 > 200.0 && r.size.width.0 > 200.0 && r.origin.x.0 > l.origin.x.0 + 100.0,
        "left={l:?}, right={r:?}"
    );
}

#[test]
fn root_domain_and_grouping_collapse_subdomains_cleanly() {
    assert_eq!(super::root_domain("mbd.baidu.com"), "baidu.com");
    assert_eq!(super::root_domain("assets.msn.cn"), "msn.cn");
    assert_eq!(super::root_domain("api.example.com.cn"), "example.com.cn");
    assert_eq!(super::root_domain("127.0.0.1:8877"), "127.0.0.1");

    let entries = vec![
        crate::session::FacetEntry {
            label: "mbd.baidu.com".into(),
            token: "mbd.baidu.com".into(),
            kind: crate::session::FacetKind::Domain,
            count: 5,
            error_count: 0,
        },
        crate::session::FacetEntry {
            label: "sp1.baidu.com".into(),
            token: "sp1.baidu.com".into(),
            kind: crate::session::FacetKind::Domain,
            count: 4,
            error_count: 0,
        },
        crate::session::FacetEntry {
            label: "assets.msn.cn".into(),
            token: "assets.msn.cn".into(),
            kind: crate::session::FacetKind::Domain,
            count: 22,
            error_count: 0,
        },
    ];
    let groups = super::group_domains_by_root(&entries, "");
    assert_eq!(groups.len(), 2);
    assert_eq!(groups[0].root, "msn.cn");
    assert_eq!(groups[0].total_count, 22);
    assert_eq!(groups[1].root, "baidu.com");
    assert_eq!(groups[1].total_count, 9);
    assert_eq!(groups[1].entries.len(), 2);

    let filtered = super::group_domains_by_root(&entries, "sp1");
    assert_eq!(filtered.len(), 1);
    assert_eq!(filtered[0].entries[0].token, "sp1.baidu.com");
}

#[test]
fn top_bottom_layout_expands_unset_columns_to_fill_window_width() {
    let shown = super::ListColumns::from_config(&crate::config::Config::default());
    let mut custom = std::collections::HashMap::new();
    custom.insert("path".to_string(), 300);

    let split_cols = super::flow_table_columns_for_layout(shown, &custom, false, 1200.0);
    let split_sum: f32 = split_cols.iter().map(|c| c.width.unwrap().0).sum();
    assert_eq!(split_sum, 648.0);

    let tb_unset = super::flow_table_columns_for_layout(
        shown,
        &std::collections::HashMap::new(),
        true,
        1200.0,
    );
    let tb_sum: f32 = tb_unset.iter().map(|c| c.width.unwrap().0).sum();
    assert!((1150.0..=1184.0).contains(&tb_sum), "tb_sum={tb_sum}");
    let path_w = tb_unset
        .iter()
        .find(|c| c.key.as_ref() == "path")
        .unwrap()
        .width
        .unwrap()
        .0;
    assert!(path_w > 500.0, "path_w={path_w}");

    // Explicitly set column keeps its custom width while unset columns expand.
    let tb_partial = super::flow_table_columns_for_layout(shown, &custom, true, 1200.0);
    let custom_path_w = tb_partial
        .iter()
        .find(|c| c.key.as_ref() == "path")
        .unwrap()
        .width
        .unwrap()
        .0;
    assert_eq!(custom_path_w, 300.0);
}

#[test]
fn app_and_source_device_classification_works() {
    use crate::session::{FacetKind, classify_client_facet_kind};

    assert_eq!(
        classify_client_facet_kind(
            "127.0.0.1 (Microsoft Edge Helper)",
            &[("User-Agent".into(), "Mozilla/5.0".into())]
        ),
        FacetKind::App
    );
    assert_eq!(
        classify_client_facet_kind(
            "10.49.83.239",
            &[(
                "User-Agent".into(),
                "Mozilla/5.0 (iPhone; CPU iPhone OS 18_0 like Mac OS X)".into()
            )]
        ),
        FacetKind::Mobile
    );
    assert_eq!(
        classify_client_facet_kind(
            "10.49.83.239",
            &[("User-Agent".into(), "okhttp/4.12.0".into())]
        ),
        FacetKind::Mobile
    );
    assert_eq!(
        classify_client_facet_kind(
            "10.49.83.100",
            &[(
                "User-Agent".into(),
                "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7)".into()
            )]
        ),
        FacetKind::Desktop
    );
    assert_eq!(
        super::base_app_name("Microsoft Edge Helper"),
        "Microsoft Edge"
    );
    assert_eq!(super::base_app_name("Code Helper"), "Visual Studio Code");
    assert!(matches!(
        super::fallback_app_glyph("Microsoft Edge Helper"),
        crate::icon::Glyph::Globe
    ));
    assert!(matches!(
        super::fallback_app_glyph("curl"),
        crate::icon::Glyph::Prompt
    ));
}

#[test]
fn wide_three_layout_clamps_list_width_and_preserves_detail_space() {
    let clamped = super::clamp_list_px_for_layout(
        660.0,
        1200.0,
        crate::config::LayoutMode::Split,
        super::SIDEBAR_WIDTH_DEFAULT,
    );
    let detail_remaining = 1200.0 - super::SIDEBAR_WIDTH_DEFAULT - clamped;
    assert!(
        detail_remaining >= 396.0,
        "detail pane should stay >= 396px at 1200px Split with sidebar, got {detail_remaining}"
    );
    let split_clamped =
        super::clamp_list_px_for_layout(660.0, 1200.0, crate::config::LayoutMode::Split, 0.0);
    assert_eq!(split_clamped, 660.0);
}

#[test]
fn toggle_root_domain_facet_switches_wildcard_and_root_tokens() {
    let next = super::toggle_root_domain_facet("", "shop.io");
    assert!(super::root_domain_facet_active(&next, "shop.io"));
    assert_eq!(next, "host:*.shop.io|shop.io");
    let cleared = super::toggle_root_domain_facet(&next, "shop.io");
    assert!(!super::root_domain_facet_active(&cleared, "shop.io"));
    assert!(cleared.is_empty());
}

#[test]
fn toolbar_status_summary_avoids_false_paused_label_and_counts_multi_rules() {
    let (rules_label, has_condition, dimmed) = super::toolbar_status_summary(
        "",
        "",
        false,
        "",
        "delay shop.io 120\nheader api.shop.io req set X-Demo 1",
        "",
        true,
        crate::i18n::Lang::Zh,
    );
    assert!(has_condition);
    assert!(!dimmed);
    assert!(
        !rules_label.contains("已暂停"),
        "unexpected paused label: {rules_label}"
    );
    assert!(
        rules_label.contains("规则×2:"),
        "expected multi-rule summary: {rules_label}"
    );

    let (secondary_label, sec_cond, sec_dimmed) = super::toolbar_status_summary(
        "",
        "app:curl",
        false,
        "",
        "delay shop.io 120\nheader api.shop.io req set X-Demo 1",
        "",
        true,
        crate::i18n::Lang::Zh,
    );
    assert!(sec_cond);
    assert!(!sec_dimmed);
    assert_eq!(secondary_label, "app:curl");

    let (paused_label, paused_cond, paused_dimmed) = super::toolbar_status_summary(
        "host:shop.io",
        "app:curl",
        false,
        "",
        "",
        "",
        true,
        crate::i18n::Lang::Zh,
    );
    assert!(paused_cond);
    assert!(!paused_dimmed);
    assert!(paused_label.contains("主过滤已暂停"), "{paused_label}");
}

#[test]
fn rule_line_assist_toggles_comments_and_fills_draft_fields() {
    let sample = "delay api.shop.io 150\nblock ads.shop.io status=451";
    let commented = super::toggle_rule_line_comment(sample, 1);
    assert_eq!(
        commented,
        "delay api.shop.io 150\n# block ads.shop.io status=451"
    );
    let uncommented = super::toggle_rule_line_comment(&commented, 1);
    assert_eq!(uncommented, sample);

    let runtime = nsh_core::element::Runtime::default();
    let _entered = runtime.enter();
    let kind = signal(0_u8);
    let host = signal(String::new());
    let path = signal(String::new());
    let op = signal(String::new());
    let from = signal(String::new());
    let to = signal(String::new());
    let modes = super::QuickRuleModeSignals::new();
    let ok = super::fill_draft_from_rule_line(
        "header api.shop.io req set X-Env staging",
        kind,
        host,
        path,
        op,
        from,
        to,
        modes,
    );
    assert!(ok);
    assert_eq!(kind.get(), 6);
    assert_eq!(host.get(), "api.shop.io");
    assert_eq!(from.get(), "X-Env");
    assert_eq!(to.get(), "staging");
    assert!(!modes.req_res.get());
}

#[test]
fn next_paged_match_offset_finds_match_across_window_boundaries() {
    let cap = super::BODY_DISPLAY_CAP;
    let text = format!("{}needle-here", "a".repeat(cap + 50));
    let win = super::TextWindow {
        offset: 0,
        end: cap,
        total: text.len(),
    };
    let next = super::next_paged_match_offset(&text, win, "NEEDLE");
    assert_eq!(next, Some(cap));
}

#[test]
fn wave1_onboarding_predicate_and_connect_hub_labels() {
    use crate::i18n::Lang;

    assert!(super::should_show_onboarding(super::Phase::Empty, 0));
    assert!(!super::should_show_onboarding(super::Phase::Empty, 3));
    assert!(!super::should_show_onboarding(super::Phase::Gone, 0));
    assert!(!super::should_show_onboarding(super::Phase::Ready, 0));

    assert_eq!(super::connect_hub_trigger_label(Lang::Zh, 0), "接入");
    assert_eq!(
        super::connect_hub_trigger_label(Lang::Zh, 2),
        "接入 · ADB 2"
    );
    assert_eq!(super::connect_hub_trigger_label(Lang::En, 0), "Connect");
    assert_eq!(
        super::connect_hub_trigger_label(Lang::En, 1),
        "Connect · ADB 1"
    );
}

#[test]
fn wave2_migration_preview_and_dropped_extensions() {
    use crate::i18n::Lang;

    assert_eq!(
        super::dropped_ext_path("file:///tmp/session.chlsj", "chlsj").as_deref(),
        Some("/tmp/session.chlsj")
    );
    assert_eq!(
        super::dropped_ext_path("file:///tmp/charles.xml", "xml").as_deref(),
        Some("/tmp/charles.xml")
    );
    assert_eq!(
        super::dropped_ext_path("file:///tmp/whistle.txt", "txt").as_deref(),
        Some("/tmp/whistle.txt")
    );

    let w = super::preview_rule_import("api.vicanso.dev 127.0.0.1:8080", Lang::Zh).unwrap();
    assert!(w.contains("Whistle"), "{w}");
    assert!(w.contains('1'), "{w}");

    let c = super::preview_rule_import(
        "<charles-export><noCaching><locations><location><host>api.vicanso.dev</host></location></locations></noCaching></charles-export>",
        Lang::En,
    )
    .unwrap();
    assert!(c.contains("Charles XML"), "{c}");
    assert!(c.contains('1'), "{c}");

    assert_eq!(
        super::preview_rule_import("[[profiles]]\nname = \"a\"", Lang::En),
        None
    );
}

#[test]
fn wave3_rule_groups_buffer_toggle_and_remove() {
    use crate::config::RuleGroup;

    let groups = vec![
        RuleGroup {
            name: "Dev".into(),
            enabled: true,
            rules: "header api.vicanso.dev req set X-Env dev".into(),
        },
        RuleGroup {
            name: "Paused".into(),
            enabled: false,
            rules: "block test.vicanso.dev".into(),
        },
    ];

    assert_eq!(
        super::active_rules_buffer_text("cors api.vicanso.dev", &groups, None),
        "cors api.vicanso.dev"
    );
    assert_eq!(
        super::active_rules_buffer_text("cors api.vicanso.dev", &groups, Some("dev")),
        "header api.vicanso.dev req set X-Env dev"
    );

    let effective = super::effective_rules_text("cors api.vicanso.dev", &groups);
    assert!(effective.starts_with("cors api.vicanso.dev"));
    assert!(effective.contains("X-Env dev"));
    assert!(!effective.contains("test.vicanso.dev"));

    let toggled = super::toggle_rule_group_enabled(&groups, "paused");
    assert!(toggled[1].enabled);
    let effective_toggled = super::effective_rules_text("cors api.vicanso.dev", &toggled);
    assert!(effective_toggled.contains("test.vicanso.dev"));

    let removed = super::remove_rule_group(&toggled, "DEV");
    assert_eq!(removed.len(), 1);
    assert_eq!(removed[0].name, "Paused");
}

#[test]
fn onboarding_grid_cols_responsive_threshold() {
    assert_eq!(super::onboarding_grid_cols(0.0), 2);
    assert_eq!(super::onboarding_grid_cols(396.0), 1);
    assert_eq!(super::onboarding_grid_cols(459.0), 1);
    assert_eq!(super::onboarding_grid_cols(460.0), 2);
    assert_eq!(super::onboarding_grid_cols(720.0), 2);
}

#[test]
fn onboarding_grid_aligns_columns_and_collapses_narrow() {
    use nsh_core::prelude::*;
    use nsh_core::{ApproxTextEngine, Core, Op, Prop, ROOT_ID, Tag};
    use std::cell::{Cell, RefCell};

    thread_local! {
        static TEST_WIDTH: Cell<f32> = const { Cell::new(580.0) };
        static GRID_BOUNDS: RefCell<[Option<Bounds<Points>>; 4]> = const { RefCell::new([None; 4]) };
    }

    #[component]
    fn OnboardingGridHarness() -> impl IntoElement {
        let w = TEST_WIDTH.with(Cell::get);
        let tile = |idx: usize| {
            nsh_base::button()
                .w_full()
                .min_w(px(0.0))
                .h(px(56.0))
                .on_layout(move |b: &Bounds<Points>| {
                    GRID_BOUNDS.with(|cell| cell.borrow_mut()[idx] = Some(*b));
                })
                .child(text(format!("Card {idx}")))
        };
        view().w(px(w)).h(px(400.0)).child(
            scroll().w_full().h_full().child(
                super::uncull_nested_scroll(view()).w_full().p_4().child(
                    view()
                        .w_full()
                        .grid()
                        .grid_cols(super::onboarding_grid_cols(w))
                        .gap_2p5()
                        .child(tile(0))
                        .child(tile(1))
                        .child(tile(2))
                        .child(tile(3)),
                ),
            ),
        )
    }

    let measure_grid = |width: f32| -> [Bounds<Points>; 4] {
        TEST_WIDTH.with(|cell| cell.set(width));
        GRID_BOUNDS.with(|cell| *cell.borrow_mut() = [None; 4]);
        let mut core = Core::new(ApproxTextEngine, Size::new(px(width), px(400.0)));
        let mut registry = Registry::new();
        registry.register::<OnboardingGridHarness>();
        core.set_components(registry);
        for op in [
            Op::Create {
                id: 2,
                tag: Tag::View,
            },
            Op::SetProp {
                id: 2,
                prop: Prop::Component(std::any::type_name::<OnboardingGridHarness>().into()),
            },
            Op::Insert {
                parent: ROOT_ID,
                child: 2,
                before: None,
            },
        ] {
            core.apply(op).expect("apply op");
        }
        for _ in 0..8 {
            let batch = core.commit();
            let mut evs = Vec::new();
            core.take_events(&mut evs);
            let mut any = false;
            for ev in evs {
                if let nsh_core::element::JsEvent::Event(id, kind, payload) = ev {
                    any |= core.component_event(id, kind, &payload);
                }
            }
            if batch.is_none() && !any {
                break;
            }
        }
        GRID_BOUNDS.with(|cell| {
            let arr = *cell.borrow();
            [
                arr[0].expect("card 0"),
                arr[1].expect("card 1"),
                arr[2].expect("card 2"),
                arr[3].expect("card 3"),
            ]
        })
    };

    let wide = measure_grid(580.0);
    assert!(
        (wide[0].size.width.0 - wide[1].size.width.0).abs() <= 0.5
            && (wide[0].size.width.0 - wide[2].size.width.0).abs() <= 0.5
            && (wide[0].size.width.0 - wide[3].size.width.0).abs() <= 0.5,
        "2x2 columns should share equal widths: {wide:?}"
    );
    assert!(
        (wide[0].origin.x.0 - wide[2].origin.x.0).abs() <= 0.5
            && (wide[1].origin.x.0 - wide[3].origin.x.0).abs() <= 0.5
            && wide[1].origin.x.0 > wide[0].origin.x.0 + 100.0
            && wide[2].origin.y.0 > wide[0].origin.y.0 + 40.0,
        "2x2 items should align vertically and horizontally: {wide:?}"
    );

    let narrow = measure_grid(410.0);
    for idx in 0..4 {
        assert!(
            (narrow[idx].origin.x.0 - narrow[0].origin.x.0).abs() <= 0.5
                && narrow[idx].size.width.0 > 360.0,
            "1-col card {idx} should fill narrow width cleanly: {narrow:?}"
        );
        if idx > 0 {
            assert!(
                narrow[idx].origin.y.0 > narrow[idx - 1].origin.y.0 + 40.0,
                "1-col cards should stack vertically: {narrow:?}"
            );
        }
    }
}
