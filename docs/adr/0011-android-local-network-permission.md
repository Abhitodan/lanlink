# 0011 Android local-network access

Android 17 (API 37) blocks local-network traffic for apps targeting 37 unless
they hold `ACCESS_LOCAL_NETWORK` (NEARBY_DEVICES group) or use a system
picker (`NsdManager` with `DiscoveryRequest.FLAG_SHOW_PICKER`). Apps
targeting 36 or lower keep implicit access through `INTERNET` and must not
request the permission.

**Decision for the first release.** Target SDK 35 (newest platform installed
with a working toolchain here). When moving to 37: declare
`ACCESS_LOCAL_NETWORK`, request it with an explanation before discovery or
connecting, keep the manual-address path working on denial, offer the
NsdManager picker as the permission-free path, and report "permission
needed" instead of a timeout. Denial must never crash.
