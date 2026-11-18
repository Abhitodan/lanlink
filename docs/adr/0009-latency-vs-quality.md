# 0009 Latency over quality: drop, never queue

| Stage | Bound | On overload |
|---|---|---|
| Capture | newest image only (`not_before` pacing) | older images superseded |
| Encoder input | no queue | frame dropped |
| Host video queue | 3 frames | drop; next frame forced to a keyframe |
| Host send | deltas older than 250 ms | dropped; keyframe requested |
| Client hand-off | 4 frames | drop; keyframe requested |
| Decoder output | render immediately | newest frame wins |

The receiver's `FrameGate` decodes a delta frame only when its reference
chain is intact (property-tested). Bitrate adapts from measured signals
(drops, RTT above path minimum, loss): `session-manager/src/bitrate.rs`.
