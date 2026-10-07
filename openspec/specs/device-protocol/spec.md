# device-protocol Specification

## Purpose
Defines the wire contract between the host daemon and every device-side agent so that drivers for different platforms interoperate with one daemon, can be versioned independently, and fail loudly instead of silently when they disagree.

## Requirements

### Requirement: Framed self-describing messages
Every message exchanged between the daemon and a device agent SHALL be a length-prefixed binary frame whose payload is a self-describing structured value. The receiver SHALL reject a frame larger than the negotiated maximum with an explicit error that names the limit, and SHALL never deliver a truncated payload.

#### Scenario: Oversized frame is rejected explicitly
- **WHEN** an agent attempts to send a response larger than the negotiated maximum frame size
- **THEN** the daemon receives an error naming the limit and the request that triggered it, and no partial payload is delivered

#### Scenario: Well-formed frame round trip
- **WHEN** the daemon sends a request frame and the agent replies
- **THEN** the reply is decoded without loss and carries the same request id

### Requirement: Version negotiation on connect
On connection, both sides SHALL exchange the set of protocol versions they speak and select the highest version in common. If there is no common version the connection SHALL fail with an error that lists both sides' versions.

#### Scenario: Common version selected
- **WHEN** the daemon speaks versions 1 and 2 and the agent speaks versions 2 and 3
- **THEN** both sides proceed on version 2

#### Scenario: No common version
- **WHEN** the daemon speaks only version 3 and the agent speaks only version 1
- **THEN** the connection is closed and the daemon reports both version sets and a hint to redeploy the agent

### Requirement: Capability advertisement
After version negotiation the agent SHALL advertise the capabilities it implements, at minimum whether it supports tree reads, input injection, screenshots, frame streaming, change events and direct text setting. The daemon SHALL refuse a command that needs a capability the agent did not advertise, with an error naming that capability, rather than attempting it.

#### Scenario: Command needs an unadvertised capability
- **WHEN** a client asks for frame streaming from an agent that advertised only screenshots
- **THEN** the daemon answers with a capability error naming frame streaming and does not forward the request

### Requirement: Request multiplexing
Requests SHALL carry a client-assigned identifier that responses echo. Multiple requests MAY be in flight on one connection and responses MAY arrive in any order.

#### Scenario: Overlapping requests
- **WHEN** a snapshot request and a screenshot request are sent back to back before either completes
- **THEN** each response is matched to its own request identifier regardless of completion order

### Requirement: Event channel with sequence numbers
Agents SHALL push unsolicited events for tree changes, window changes, frame availability and agent lifecycle on a stream separate from responses. Each event stream SHALL carry a monotonically increasing sequence number so that a consumer can detect a gap.

#### Scenario: Sequence gap detected
- **WHEN** the daemon receives event 41 followed by event 43 on the same stream
- **THEN** the daemon marks the stream as out of sync and requests a full resynchronization from the agent

### Requirement: Structured errors
Every error SHALL carry a stable machine-readable code, a human-readable message, and a flag indicating whether retrying the same request is meaningful.

#### Scenario: Transient versus permanent failure
- **WHEN** a tree read fails because the foreground window changed during acquisition
- **THEN** the error is marked retryable, while an error for an unknown method is marked not retryable

### Requirement: Deadlines and cancellation
A request MAY carry a deadline. When the deadline passes the agent SHALL stop the work, respond with a deadline error, and remain responsive to subsequent requests.

#### Scenario: Snapshot exceeds its deadline
- **WHEN** a tree read is requested with a 200 ms deadline on a screen that takes longer
- **THEN** a deadline error is returned and the next request on the same connection is served normally

### Requirement: Liveness detection
The daemon SHALL detect a dead or hung agent within a bounded interval and report the target as disconnected. Commands issued to a disconnected target SHALL fail fast with a reconnect hint instead of hanging.

#### Scenario: Agent process killed
- **WHEN** the agent process is killed on the device
- **THEN** within 5 seconds the daemon reports the target as disconnected and a subsequent command fails immediately with a reconnect hint
