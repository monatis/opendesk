/**
 * @vitalops/opendesk-sdk — public API surface
 */

export { OpenDeskClient, type OpenDeskClientOptions } from "./client.js";
export { createMcpServer, runMcpStdio } from "./mcp.js";
export { ToolRegistry, createRegistry } from "./registry.js";
export {
  Tool,
  type ToolResult,
  type ToolContext,
  type PermissionHandler,
  type Attachment,
  allowAllContext,
  checkPermission,
  PermissionDeniedError,
} from "./tools/base.js";

// Remote machine control
export { connect, pairWith, type Target, type ConnectOptions, type PairWithOptions } from "./remote/client.js";
export { OpendeskServer, readDescription, writeDescription, clearDescription, DEFAULT_PORT } from "./remote/server.js";
export { discover, advertise, type DiscoveredPeer, type Advertisement } from "./remote/discovery.js";
export { RemoteComputer, SessionEvicted, type CapabilityManifest, type Connector } from "./computer/remote.js";

// Protocol — auth / identity
export { Identity, generatePairingCode, fingerprint, DEFAULT_HOME } from "./protocol/auth/identity.js";
export { TrustedPeers, type TrustedPeer } from "./protocol/auth/storage.js";
export { pairClient, pairServer, authClient, authServer, AuthFailure, type Session } from "./protocol/auth/handshake.js";

// Protocol — peer / frames / connection
export { Peer, ProtocolError, type Dispatcher, type PushHandler } from "./protocol/peer.js";
export { Connection, ConnectionClosed, loopbackPair } from "./protocol/connection.js";
export { connectWebSocket, serveWebSocket, WebSocketConnection, WebSocketServerWrapper } from "./protocol/transports/websocket.js";
export { encode, decode, CodecError } from "./protocol/codec.js";
export { PROTOCOL_VERSION, type Frame, type HelloFrame, type ReqFrame, type ResFrame, type PushFrame, type ErrorInfo, type ErrorCode } from "./protocol/frames.js";
