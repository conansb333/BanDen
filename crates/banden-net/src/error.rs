//! Network-layer errors.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum NetError {
    #[error("adapter query failed (code {0})")]
    AdapterQuery(u32),

    #[error("ARP table query failed (code {0})")]
    ArpQuery(u32),

    #[error("ARP probe failed (code {0})")]
    ArpProbe(u32),

    #[error("gateway not available")]
    NoGateway,

    #[error("ICMP probe failed (code {0})")]
    Icmp(u32),

    #[error("WinSock initialization failed (code {0})")]
    WsaStartup(i32),

    #[error("reverse DNS failed (code {0})")]
    ReverseDns(i32),

    #[error("invalid subnet: {0}")]
    InvalidSubnet(String),

    #[error("interface not found: {0}")]
    InterfaceNotFound(String),

    #[error("packet capture library unavailable: {0}")]
    PcapUnavailable(String),

    #[error("cannot open adapter for raw sending: {0}")]
    PcapOpen(String),

    #[error("cannot list adapters: {0}")]
    PcapList(String),

    #[error("raw frame send failed: {0}")]
    PcapSend(String),

    #[error("packet capture failed: {0}")]
    PcapRecv(String),

    #[error("malformed MAC address: {0}")]
    MacParse(String),

    #[error(transparent)]
    Io(#[from] std::io::Error),
}

pub type NetResult<T> = Result<T, NetError>;
