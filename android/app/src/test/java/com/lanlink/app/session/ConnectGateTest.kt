package com.lanlink.app.session

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class ConnectGateTest {
    @Test fun currentLoopWithNoHandleAccepts() {
        val g = ConnectGate()
        val e = g.advance()
        assertEquals(ConnectDecision.ACCEPT, g.decide(e, stopped = false, handle = 0L))
    }

    @Test fun staleEpochCloses() {
        val g = ConnectGate()
        val old = g.advance()
        g.advance()
        assertFalse(g.isCurrent(old))
        assertEquals(ConnectDecision.CLOSE, g.decide(old, stopped = false, handle = 0L))
    }

    @Test fun stoppedSessionCloses() {
        val g = ConnectGate()
        val e = g.advance()
        assertEquals(ConnectDecision.CLOSE, g.decide(e, stopped = true, handle = 0L))
    }

    @Test fun heldHandleCloses() {
        val g = ConnectGate()
        val e = g.advance()
        assertEquals(ConnectDecision.CLOSE, g.decide(e, stopped = false, handle = 42L))
    }

    /** retry() while the first loop is still inside a blocking connect. */
    @Test fun retryDuringConnectLetsOnlyTheNewLoopAccept() {
        val g = ConnectGate()
        val first = g.advance()
        val second = g.advance()
        // The new loop connects first and publishes its handle.
        assertEquals(ConnectDecision.ACCEPT, g.decide(second, stopped = false, handle = 0L))
        // The stale loop's connect returns afterwards: it must close its handle.
        assertEquals(ConnectDecision.CLOSE, g.decide(first, stopped = false, handle = 7L))
        // Even before the new loop publishes, the stale loop may not accept.
        assertEquals(ConnectDecision.CLOSE, g.decide(first, stopped = false, handle = 0L))
    }

    /** stop() then start(): a loop from before the stop stays stale. */
    @Test fun stopThenStartKeepsTheOldLoopStale() {
        val g = ConnectGate()
        val before = g.advance() // start()
        g.advance() // stop()
        val after = g.advance() // start()
        assertTrue(g.isCurrent(after))
        assertEquals(ConnectDecision.CLOSE, g.decide(before, stopped = false, handle = 0L))
        assertEquals(ConnectDecision.ACCEPT, g.decide(after, stopped = false, handle = 0L))
    }
}
