use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HelperConfig {
    pub proxy_port: u16,
    #[serde(default)]
    pub proxy_auth: String,
    #[serde(default)]
    pub capture_all: bool,
    #[serde(default)]
    pub targets: Vec<String>,
    #[serde(default = "default_true")]
    pub block_quic: bool,
    #[serde(default)]
    pub zniff_pid: u32,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
pub enum HelperCommand {
    Update(HelperConfig),
    Stop,
    Ping,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum HelperEvent {
    #[serde(rename_all = "camelCase")]
    Ready {
        bridge_port_v4: u16,
        bridge_port_v6: u16,
    },
    Updated,
    Error {
        message: String,
    },
    Pong,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_and_event_roundtrip_json() {
        let cmd = HelperCommand::Update(HelperConfig {
            proxy_port: 8877,
            proxy_auth: "user:pass".into(),
            capture_all: false,
            targets: vec!["firefox".into(), "curl".into()],
            block_quic: true,
            zniff_pid: 4242,
        });
        let raw = serde_json::to_string(&cmd).unwrap();
        let parsed: HelperCommand = serde_json::from_str(&raw).unwrap();
        assert_eq!(parsed, cmd);

        let evt = HelperEvent::Ready {
            bridge_port_v4: 45123,
            bridge_port_v6: 45124,
        };
        let raw_evt = serde_json::to_string(&evt).unwrap();
        let parsed_evt: HelperEvent = serde_json::from_str(&raw_evt).unwrap();
        assert_eq!(parsed_evt, evt);
    }
}
