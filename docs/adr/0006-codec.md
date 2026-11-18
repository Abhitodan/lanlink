# 0006 H.264 only in the first release

Every Android phone has a hardware H.264 decoder; HEVC/AV1 support and their
low-latency behaviour vary by vendor. Codec ids for HEVC and AV1 are reserved
and the client reports capabilities, so they can be added without a protocol
change after end-to-end latency and power are measured on real phones. AV1
is not assumed to be lower latency.
