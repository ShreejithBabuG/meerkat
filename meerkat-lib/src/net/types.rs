use serde::{Deserialize, Serialize};

/// Unique identifier for sent messages (for error tracking)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MessageId(pub u64);

/// Address - canonical internet-routable address
/// Examples: 
/// - Server: "/ip4/203.0.113.10/tcp/9000/p2p/12D3..."
/// - Client: "/ip4/203.0.113.10/tcp/9000/p2p/server-id/ws/p2p/client-id"
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Address(pub String);

impl Address {
    pub fn new(addr: impl Into<String>) -> Self {
        Address(addr.into())
    }
}

/// Message types in the Meerkat protocol
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum MeerkatMessage {
    /// Ping for testing
    Ping { content: String },
    
    /// Pong response
    Pong { content: String },
    
    /// Peer announcement with their canonical address
    Announce { peer_addr: Address },
    
    /// Transaction message (for future use)
    Transaction {
        tx_id: u64,
        payload: Vec<u8>,
    },
    
    /// Propagation message (for future use)
    Propagation {
        var_id: u64,
        new_value: Vec<u8>,
    },

    /// Request to look up a member of a service on a remote node
    LookupRequest {
        request_id: u64,
        service: String,
        member: String,
        reply_to: String,  // full multiaddr of the requester
    },

    /// Response to a LookupRequest with the serialized value
    LookupResponse {
        request_id: u64,
        value: String,  // JSON-serialized Value
    },

    /// Response indicating lookup failed
    LookupError {
        request_id: u64,
        error: String,
    },

    /// Execute an action on a remote service
    ActionRequest {
        request_id: u64,
        service: String,
        stmts: Vec<crate::ast::ActionStmt>,
        env: Vec<(String, crate::ast::Value)>,
        reply_to: String,
    },

    /// Response to ActionRequest
    ActionResponse {
        request_id: u64,
        success: bool,
        error: Option<String>,
    },
}

/// Errors that can occur when sending
#[derive(Debug, Clone)]
pub enum SendError {
    /// Could not resolve/reach the address
    UnreachableAddress(Address),
    
    /// Connection dropped before send completed
    ConnectionLost(String),
    
    /// Message too large or other protocol error
    ProtocolError(String),
}

/// Describes what kind of node we are.
/// Determines how translate_address behaves.
pub enum NodeType {
    /// Server node - can dial IP directly, no translation needed
    Server,
    /// Browser client - can only reach the network via WebSocket to relay server
    BrowserClient {
        /// WebSocket address of our relay server e.g.
        /// "/ip4/server1-ip/tcp/9001/ws/p2p/server1-id"
        relay_server: Address,
    },
}
