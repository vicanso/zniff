use super::*;

use std::net::Ipv4Addr;

use super::{LanAddr, cert_url, choose_lan, usable_v4};
use crate::i18n::Lang;
use crate::session::Protocol;

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
    assert!(!super::filter_has_token("type:jsonl", super::JSON_ALIASES));
    assert!(super::filter_has_token("pin:", super::PIN_ALIASES));
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
    assert_eq!(super::editor_box_height("only"), 34.0);
    let long = "x\n".repeat(30);
    assert_eq!(super::editor_box_height(&long), 286.0);
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
        .with_full_url(true)
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
        url: "https://api.example.com/v1/users".to_string(),
        comment: String::new(),
        protocol: "HTTP/2".to_string(),
        size: "128 B".to_string(),
        size_bytes: 128,
        failed: false,
        pinned: false,
        relay_short: String::new(),
    };
    assert_eq!(super::row_target_text(&row, false), "/v1/users");
    assert_eq!(
        super::row_target_text(&row, true),
        "https://api.example.com/v1/users"
    );
    let empty_url_row = super::RowLine {
        url: String::new(),
        ..row
    };
    assert_eq!(super::row_target_text(&empty_url_row, true), "/v1/users");
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
        full_url: false,
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
    assert_eq!(default_sum, 568.0);

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
        full_url: false,
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
        false,
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
        false,
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
        false,
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
fn toolbar_summary_shows_profile_name_and_primary_filter_conditions() {
    let saved = vec![crate::config::SavedProfile {
        name: "test".into(),
        filter: "host:test.gf.com.cn|t1.gf.com.cn path:/finance-api/online-hall".into(),
        bypass_hosts: String::new(),
        rules: "no-cache *".into(),
        upstream_proxy: String::new(),
        rules_enabled: true,
        breaks_enabled: true,
        show_tunnel_failures: false,
    }];
    assert_eq!(
        super::toolbar_active_profile_name(
            &saved,
            "",
            "host:test.gf.com.cn|t1.gf.com.cn path:/finance-api/online-hall",
            "",
            "no-cache *",
            "",
        ),
        Some("test".to_string())
    );
    assert_eq!(
        super::toolbar_active_profile_name(&saved, "custom", "host:a.com", "", "", ""),
        Some("custom".to_string())
    );
    assert_eq!(
        super::toolbar_filter_conditions(
            "host:test.gf.com.cn|t1.gf.com.cn path:/finance-api/online-hall",
            "",
            "no-cache *",
            "",
            true,
            crate::i18n::Lang::Zh,
        ),
        (
            "host:test.gf.com.cn|t1.gf.com.cn path:/finance-api/online-hall".to_string(),
            true,
        )
    );
    assert_eq!(
        super::toolbar_filter_conditions("", "", "", "", true, crate::i18n::Lang::Zh),
        ("全部流量 · 未设置过滤条件".to_string(), false)
    );
    assert_eq!(
        super::toolbar_filter_conditions(
            "",
            "",
            "",
            "http://upuser:s3cr3t@127.0.0.1:7890",
            true,
            crate::i18n::Lang::Zh,
        ),
        ("upstream:upuser@127.0.0.1:7890".to_string(), true)
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
        include_str!("lan_qr.rs"),
        "\n",
        include_str!("list_columns.rs"),
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
            for _ in 0..12 {
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
                if batch.is_none() && !any {
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

    assert_eq!(super::editor_box_height(&sample_json), 286.0);

    // 1. Reproduce the unguarded bug: `Layer::Paint` above `ed.vscroll` defaults to
    //    `Auto` because `MountItem::ViewProps { pointer_events: None }` is never emitted!
    let unguarded = drive_select_flow::<UnguardedRawEditorHarness>();
    let unguarded_editors = find_editor_paint_overlays(&unguarded);
    assert_eq!(unguarded_editors.len(), 1);
    let (_, _, unguarded_paint_id) = unguarded_editors[0];
    assert_ne!(
        unguarded.pointer_events.get(&unguarded_paint_id),
        Some(&PointerEvents::None),
        "unguarded editor inside culled scroll omits ViewProps(pointer_events: None) on Layer::Paint"
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
fn deselecting_active_profile_clears_config_and_keeps_saved_presets() {
    let test_preset = crate::config::SavedProfile {
        name: "test".into(),
        filter: "host:test.gf.com.cn|t1.gf.com.cn path:/finance-api/online-hall|/super".into(),
        bypass_hosts: "internal.example.com".into(),
        rules: "no-cache *".into(),
        upstream_proxy: "127.0.0.1:9000".into(),
        rules_enabled: false,
        breaks_enabled: false,
        show_tunnel_failures: true,
    };
    let mut cfg = crate::config::Config {
        active_profile: "test".into(),
        filter: test_preset.filter.clone(),
        bypass_hosts: test_preset.bypass_hosts.clone(),
        rules: test_preset.rules.clone(),
        upstream_proxy: test_preset.upstream_proxy.clone(),
        rules_enabled: false,
        breaks_enabled: false,
        show_tunnel_failures: true,
        saved_profiles: vec![test_preset.clone()],
        ..crate::config::Config::default()
    };

    let history = super::push_profile_history(
        &cfg.profile_history,
        &cfg.active_profile,
        &cfg.filter,
        &cfg.bypass_hosts,
        &cfg.rules,
        &cfg.upstream_proxy,
        cfg.rules_enabled,
        cfg.breaks_enabled,
        cfg.show_tunnel_failures,
    );
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].name, "test");

    super::clear_working_setup_config(&mut cfg);
    assert_eq!(cfg.active_profile, "");
    assert_eq!(cfg.filter, "");
    assert_eq!(cfg.bypass_hosts, "");
    assert_eq!(cfg.rules, "");
    assert_eq!(cfg.upstream_proxy, "");
    assert!(cfg.rules_enabled);
    assert!(cfg.breaks_enabled);
    assert!(!cfg.show_tunnel_failures);
    assert_eq!(cfg.saved_profiles, vec![test_preset]);

    let mut saved_with_blank = cfg.saved_profiles.clone();
    saved_with_blank.push(crate::config::SavedProfile {
        name: "empty".into(),
        filter: String::new(),
        bypass_hosts: String::new(),
        rules: String::new(),
        upstream_proxy: String::new(),
        rules_enabled: true,
        breaks_enabled: true,
        show_tunnel_failures: false,
    });
    assert_eq!(
        super::toolbar_active_profile_name(
            &saved_with_blank,
            &cfg.active_profile,
            &cfg.filter,
            &cfg.bypass_hosts,
            &cfg.rules,
            &cfg.upstream_proxy,
        ),
        None
    );
    assert_eq!(
        super::toolbar_filter_conditions(
            &cfg.filter,
            &cfg.bypass_hosts,
            &cfg.rules,
            &cfg.upstream_proxy,
            cfg.rules_enabled,
            crate::i18n::Lang::Zh,
        ),
        ("全部流量 · 未设置过滤条件".to_string(), false)
    );
}
