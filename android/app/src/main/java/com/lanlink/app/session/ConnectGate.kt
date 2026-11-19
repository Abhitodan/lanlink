package com.lanlink.app.session

/** What a connect loop does with the native handle it just opened. */
enum class ConnectDecision {
    /** Publish it as the session's handle. */
    ACCEPT,

    /** Not wanted: close it and end the loop. */
    CLOSE,
}

/**
 * Connection epochs. `start()`, `retry()` and `stop()` each begin a new
 * epoch. A connect loop, and the event loop that follows from it, carry the
 * epoch they were started in and write nothing once it is no longer current.
 * Two connect loops can overlap (for example `retry()` while an earlier loop
 * is still inside a blocking connect); without the epoch the stale loop could
 * publish a second handle, leak it, or later fail the healthy connection.
 *
 * Not thread-safe on its own: the owner calls it under its lock.
 */
class ConnectGate {
    var epoch = 0
        private set

    /** Begins a new epoch and returns it. */
    fun advance(): Int {
        epoch += 1
        return epoch
    }

    fun isCurrent(e: Int): Boolean = e == epoch

    /**
     * The loop of epoch [e] opened a handle. It may publish it only while its
     * epoch is current, the session is not stopped and no handle is published
     * yet; otherwise the new handle must be closed.
     */
    fun decide(e: Int, stopped: Boolean, handle: Long): ConnectDecision =
        if (!stopped && isCurrent(e) && handle == 0L) ConnectDecision.ACCEPT else ConnectDecision.CLOSE
}
