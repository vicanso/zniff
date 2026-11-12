#[allow(unused_imports)]
use super::*;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct LanAddr {
    pub(crate) name: String,
    pub(crate) ip: Ipv4Addr,
}

#[derive(Clone, PartialEq)]
pub(crate) struct RowLine {
    pub(crate) id: u64,
    pub(crate) method: String,
    pub(crate) status: Option<u16>,
    pub(crate) time: String,
    pub(crate) gap_nanos: u128,
    /// Local `HH:MM:SS.mmm`, shown when the gap cell is hovered.
    pub(crate) clock: String,
    /// Request start until the full response. `—` while it is still open.
    pub(crate) elapsed: String,
    pub(crate) elapsed_nanos: Option<u128>,
    pub(crate) elapsed_slow: bool,
    pub(crate) host: String,
    pub(crate) client_ip: String,
    pub(crate) path: String,
    pub(crate) url: String,
    pub(crate) comment: String,
    pub(crate) protocol: String,
    pub(crate) size: String,
    pub(crate) size_bytes: u64,
    pub(crate) failed: bool,
    pub(crate) pinned: bool,
    /// First 8 hex characters of the share that pushed this row. Empty for local capture.
    pub(crate) relay_short: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ListSort {
    pub(crate) column: ListColumn,
    pub(crate) desc: bool,
}

impl ListSort {
    pub(crate) fn toggle(current: Option<Self>, column: ListColumn) -> Option<Self> {
        match current {
            Some(Self {
                column: cur,
                desc: false,
            }) if cur == column => Some(Self { column, desc: true }),
            Some(Self {
                column: cur,
                desc: true,
            }) if cur == column => None,
            _ => Some(Self {
                column,
                desc: false,
            }),
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct ListColumns {
    pub(crate) id: bool,
    pub(crate) method: bool,
    pub(crate) path: bool,
    pub(crate) status: bool,
    pub(crate) gap: bool,
    pub(crate) elapsed: bool,
    pub(crate) size: bool,
    pub(crate) protocol: bool,
    pub(crate) host: bool,
    pub(crate) client_ip: bool,
    pub(crate) full_url: bool,
    pub(crate) ellipsize: config::ListEllipsize,
    pub(crate) order: [ListColumn; 10],
    pub(crate) order_rev: u64,
}

impl PartialEq for ListColumns {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
            && self.method == other.method
            && self.path == other.path
            && self.status == other.status
            && self.gap == other.gap
            && self.elapsed == other.elapsed
            && self.size == other.size
            && self.protocol == other.protocol
            && self.host == other.host
            && self.client_ip == other.client_ip
            && self.full_url == other.full_url
            && self.ellipsize == other.ellipsize
            && self.order == other.order
    }
}

impl Eq for ListColumns {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ListColumn {
    Id,
    Method,
    Path,
    Status,
    Gap,
    Elapsed,
    Size,
    Protocol,
    Host,
    ClientIp,
}

impl ListColumn {
    pub(crate) const ORDERED: [Self; 10] = [
        Self::Id,
        Self::Method,
        Self::Path,
        Self::Status,
        Self::Gap,
        Self::Elapsed,
        Self::Size,
        Self::Protocol,
        Self::Host,
        Self::ClientIp,
    ];

    pub(crate) fn from_key(key: &str) -> Option<Self> {
        match key.trim() {
            "id" => Some(Self::Id),
            "method" => Some(Self::Method),
            "path" => Some(Self::Path),
            "status" => Some(Self::Status),
            "gap" => Some(Self::Gap),
            "elapsed" => Some(Self::Elapsed),
            "size" => Some(Self::Size),
            "protocol" => Some(Self::Protocol),
            "host" => Some(Self::Host),
            "client_ip" => Some(Self::ClientIp),
            _ => None,
        }
    }

    pub(crate) fn is_fixed(self) -> bool {
        matches!(self, Self::Id | Self::Method | Self::Path)
    }

    pub(crate) fn order_from_keys(keys: &[String]) -> [Self; 10] {
        let resolved = config::resolve_column_order(keys);
        let mut out = Self::ORDERED;
        for (slot, key) in out.iter_mut().zip(resolved.iter()) {
            if let Some(col) = Self::from_key(key) {
                *slot = col;
            }
        }
        out
    }

    pub(crate) fn key(self) -> &'static str {
        match self {
            Self::Id => "id",
            Self::Method => "method",
            Self::Path => "path",
            Self::Status => "status",
            Self::Gap => "gap",
            Self::Elapsed => "elapsed",
            Self::Size => "size",
            Self::Protocol => "protocol",
            Self::Host => "host",
            Self::ClientIp => "client_ip",
        }
    }

    pub(crate) fn labels(self) -> (&'static str, &'static str) {
        match self {
            Self::Id => ("序号", "#"),
            Self::Method => ("方法", "Method"),
            Self::Path => ("路径", "Path"),
            Self::Status => ("状态", "Status"),
            Self::Gap => ("间隔", "Gap"),
            Self::Elapsed => ("耗时", "Elapsed"),
            Self::Size => ("大小", "Size"),
            Self::Protocol => ("协议", "Protocol"),
            Self::Host => ("主机", "Host"),
            Self::ClientIp => ("来源 IP", "Client IP"),
        }
    }
}

impl ListColumns {
    pub(crate) fn from_config(config: &config::Config) -> Self {
        Self {
            id: config.show_id,
            method: config.show_method,
            path: config.show_path,
            status: config.show_status,
            gap: config.show_gap,
            elapsed: config.show_elapsed,
            size: config.show_size,
            protocol: config.show_protocol,
            host: config.show_host,
            client_ip: config.show_client_ip,
            full_url: config.show_full_url,
            ellipsize: config.ellipsize,
            order: ListColumn::order_from_keys(&config.column_order),
            order_rev: 0,
        }
    }

    pub(crate) fn on(self, column: ListColumn) -> bool {
        match column {
            ListColumn::Id => self.id,
            ListColumn::Method => self.method,
            ListColumn::Path => self.path,
            ListColumn::Status => self.status,
            ListColumn::Gap => self.gap,
            ListColumn::Elapsed => self.elapsed,
            ListColumn::Size => self.size,
            ListColumn::Protocol => self.protocol,
            ListColumn::Host => self.host,
            ListColumn::ClientIp => self.client_ip,
        }
    }

    pub(crate) fn first_visible(self) -> Option<ListColumn> {
        self.order.into_iter().find(|&col| self.on(col))
    }

    pub(crate) fn last_visible(self) -> Option<ListColumn> {
        self.order.into_iter().rev().find(|&col| self.on(col))
    }

    pub(crate) fn visible_count(self) -> usize {
        self.order.into_iter().filter(|&col| self.on(col)).count()
    }

    pub(crate) fn toggle(self, column: ListColumn) -> Self {
        let mut next = self;
        match column {
            ListColumn::Id => next.id = !next.id,
            ListColumn::Method => next.method = !next.method,
            ListColumn::Path => next.path = !next.path,
            ListColumn::Status => next.status = !next.status,
            ListColumn::Gap => next.gap = !next.gap,
            ListColumn::Elapsed => next.elapsed = !next.elapsed,
            ListColumn::Size => next.size = !next.size,
            ListColumn::Protocol => next.protocol = !next.protocol,
            ListColumn::Host => next.host = !next.host,
            ListColumn::ClientIp => next.client_ip = !next.client_ip,
        }
        if next.visible_count() == 0 {
            return self;
        }
        next
    }

    pub(crate) fn can_move_slot(slot: usize, delta: isize) -> bool {
        let Some(target) = slot
            .checked_add_signed(delta)
            .filter(|&t| t < ListColumn::ORDERED.len())
        else {
            return false;
        };
        (slot < 3) == (target < 3)
    }

    #[cfg(test)]
    pub(crate) fn can_move(self, column: ListColumn, delta: isize) -> bool {
        let Some(slot) = self.order.iter().position(|&col| col == column) else {
            return false;
        };
        Self::can_move_slot(slot, delta)
    }

    pub(crate) fn move_by(mut self, column: ListColumn, delta: isize) -> Self {
        let Some(slot) = self.order.iter().position(|&col| col == column) else {
            return self;
        };
        if !Self::can_move_slot(slot, delta) {
            return self;
        }
        let target = (slot as isize + delta) as usize;
        self.order.swap(slot, target);
        self.order_rev = self.order_rev.wrapping_add(1);
        self
    }

    pub(crate) fn apply_visible_order(mut self, visible_keys: &[SharedString]) -> Self {
        let before = self;
        for fixed_zone in [true, false] {
            let picked: Vec<ListColumn> = visible_keys
                .iter()
                .filter_map(|key| ListColumn::from_key(key.as_ref()))
                .filter(|&col| col.is_fixed() == fixed_zone && before.on(col))
                .collect();
            let current: Vec<ListColumn> = self
                .order
                .into_iter()
                .filter(|&col| col.is_fixed() == fixed_zone && before.on(col))
                .collect();
            if picked.len() != current.len() || !current.iter().all(|col| picked.contains(col)) {
                continue;
            }
            let mut iter = picked.into_iter();
            for slot in &mut self.order {
                if slot.is_fixed() == fixed_zone
                    && before.on(*slot)
                    && let Some(col) = iter.next()
                {
                    *slot = col;
                }
            }
        }
        self
    }

    pub(crate) fn has_custom_order(self) -> bool {
        self.order != ListColumn::ORDERED
    }

    pub(crate) fn reset_order(mut self) -> Self {
        if self.order == ListColumn::ORDERED {
            return self;
        }
        self.order = ListColumn::ORDERED;
        self.order_rev = self.order_rev.wrapping_add(1);
        self
    }

    pub(crate) fn with_full_url(self, full_url: bool) -> Self {
        Self { full_url, ..self }
    }

    pub(crate) fn with_ellipsize(self, ellipsize: config::ListEllipsize) -> Self {
        Self { ellipsize, ..self }
    }

    pub(crate) fn ellipsize_mode(self) -> EllipsizeMode {
        match self.ellipsize {
            config::ListEllipsize::Head => EllipsizeMode::Head,
            config::ListEllipsize::Middle => EllipsizeMode::Middle,
            config::ListEllipsize::Tail => EllipsizeMode::Tail,
        }
    }

    pub(crate) fn order_keys(self) -> Vec<String> {
        let keys: Vec<String> = self
            .order
            .into_iter()
            .map(|col| col.key().to_string())
            .collect();
        config::normalize_column_order(&keys)
    }

    pub(crate) fn write(self, config: &mut config::Config) {
        config.show_id = self.id;
        config.show_method = self.method;
        config.show_path = self.path;
        config.show_status = self.status;
        config.show_gap = self.gap;
        config.show_elapsed = self.elapsed;
        config.show_size = self.size;
        config.show_protocol = self.protocol;
        config.show_host = self.host;
        config.show_client_ip = self.client_ip;
        config.show_full_url = self.full_url;
        config.ellipsize = self.ellipsize;
        config.column_order = self.order_keys();
    }
}

/// Columns the table shows, in order. Id, Method, and Path (when shown) stay in
/// the fixed left group. The rest scroll under them.
/// Widths are the last saved drag, not a live value: the table keeps a drag
/// until this list changes, and a changed width here would fight the drag.
pub(crate) fn flow_table_columns(
    shown: ListColumns,
    widths: &HashMap<String, u32>,
) -> Vec<DataTableColumn> {
    shown
        .order
        .into_iter()
        .filter(|&col| shown.on(col))
        .map(|col| {
            let key = col.key();
            let fixed = col.is_fixed();
            let limit = config::column_width_limit(key).expect("known column");
            let width = widths.get(key).copied().unwrap_or(limit.default);
            let mut column = DataTableColumn::default();
            column.key = key.into();
            column.width = Some(px(width as f32));
            column.min_width = Some(px(limit.min as f32));
            column.max_width = Some(px(limit.max as f32));
            column.fixed = ColumnFixed::from(fixed);
            column.resizable = Some(true);
            column.movable = Some(true);
            column.selectable = Some(false);
            column
        })
        .collect()
}

/// Column titles follow the same `tr` pair as the rest of the window.
/// Called while the `.columns` prop runs, so a language change rebuilds them.
pub(crate) fn flow_column_title(key: &str, full_url: bool) -> String {
    match key {
        "method" => tr("方法", "Method")(),
        "id" => tr("序号", "#")(),
        "path" => {
            if full_url {
                "URL".to_string()
            } else {
                tr("路径", "Path")()
            }
        }
        "status" => tr("状态", "Status")(),
        "protocol" => tr("协议", "Protocol")(),
        "gap" => tr("间隔", "Gap")(),
        "elapsed" => tr("耗时", "Elapsed")(),
        "host" => tr("主机", "Host")(),
        "client_ip" => tr("来源 IP", "Client IP")(),
        "size" => tr("大小", "Size")(),
        _ => String::new(),
    }
}

pub(crate) fn flow_table_classes() -> DataTableClasses {
    let mut classes = DataTableClasses::default();
    classes.root = Some("bg-white dark:bg-[#10141d]".into());
    classes.header = Some(
        "bg-[#f1f5f9] dark:bg-[#151b26] border-b border-b-[#e2e8f0] dark:border-b-[#222938]".into(),
    );
    classes.header_cell =
        Some("bg-[#f1f5f9] dark:bg-[#151b26] cursor-grab active:cursor-grabbing".into());
    classes.header_cell_dragging = Some("opacity-60 bg-[#e2e8f0] dark:bg-[#222938]".into());
    classes.move_indicator = Some("bg-[#2563eb] dark:bg-[#3b82f6]".into());
    classes.row = Some(
        "bg-white dark:bg-[#10141d] border-b border-b-[#f1f5f9] dark:border-b-[#19202e] cursor-pointer"
            .into(),
    );
    classes.row_odd = Some("bg-[#fafbfd] dark:bg-[#121722]".into());
    classes.row_hover = Some("bg-[#f1f5f9] dark:bg-[#1a2232]".into());
    classes.row_selected = Some("bg-[#eff6ff] dark:bg-[#172542]".into());
    // The table draws a 1px rule after the pinned columns. Painting it made
    // a line only beside the first scrolling column. The drag target stays
    // and shows a thicker 3px bar across the full 5px hit band on hover/drag.
    classes.resize_handle = Some(
        "w-[3px] bg-transparent hover:bg-[#2563eb] group-hover:bg-[#2563eb] group-active:bg-[#2563eb] dark:hover:bg-[#3b82f6] dark:group-hover:bg-[#3b82f6] dark:group-active:bg-[#3b82f6]"
            .into(),
    );
    classes
}
