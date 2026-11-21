package dev.nori.music

import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicInteger
import java.util.concurrent.atomic.AtomicReference
import kotlin.coroutines.Continuation
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withContext
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import uniffi.UNIFFI_RUST_FUTURE_COMPLETE
import uniffi.UNIFFI_RUST_FUTURE_PENDING
import uniffi.uniffiAwaitFuture
import uniffi.uniffiContinuationResume

/**
 * Lifetime guarantees of [uniffiAwaitFuture]: a Rust future's handle must never be passed to `cancel`
 * after `free` has released it, nor freed more than once.
 */
class UniffiFutureTest {
    /**
     * When `poll` throws synchronously (a UniFFI error like `BackupException`), `suspendCancellableCoroutine`
     * must detach its continuation rather than leaving `invokeOnCancellation` armed on the enclosing job to
     * call `cancel` after `finally` already freed the handle.
     */
    @Test
    fun syncPollErrorNeverCancelsAfterFree() = runBlocking {
        val freed = AtomicBoolean(false)
        val freedCount = AtomicInteger(0)
        val cancelAfterFree = AtomicBoolean(false)

        val thrown = runCatching {
            withContext(Dispatchers.Default) {
                uniffiAwaitFuture(
                    rustFuture = 1L,
                    poll = { _, _ -> throw IllegalStateException("not a backup") },
                    cancel = { if (freed.get()) cancelAfterFree.set(true) },
                    free = {
                        freed.set(true)
                        freedCount.incrementAndGet()
                    },
                )
            }
        }.exceptionOrNull()

        assertTrue("the error from poll must reach the caller", thrown is IllegalStateException)
        assertFalse("cancel must never run after free", cancelAfterFree.get())
        assertEquals("free must run once", 1, freedCount.get())
    }

    /**
     * Cancelling an awaiting coroutine from another thread while a waker may also resume it must serialize
     * `cancel` before `free` and run `free` once.
     */
    @Test
    fun concurrentCancelAndWakeNeverTouchFreedHandle() = runBlocking {
        repeat(400) { iter ->
            val freed = AtomicBoolean(false)
            val freedCount = AtomicInteger(0)
            val cancelCount = AtomicInteger(0)
            val cancelAfterFree = AtomicBoolean(false)
            val inCancel = AtomicBoolean(false)
            val freeDuringCancel = AtomicBoolean(false)
            val stored = AtomicReference<Continuation<Boolean>?>(null)

            val polled = AtomicBoolean(false)
            val job = launch(Dispatchers.Default) {
                uniffiAwaitFuture(
                    rustFuture = iter.toLong() + 1L,
                    poll = { _, cont ->
                        polled.set(true)
                        if (stored.compareAndSet(null, cont)) UNIFFI_RUST_FUTURE_PENDING
                        else UNIFFI_RUST_FUTURE_COMPLETE
                    },
                    cancel = {
                        inCancel.set(true)
                        if (freed.get()) cancelAfterFree.set(true)
                        cancelCount.incrementAndGet()
                        // Simulate Rust's scheduler.cancel() resuming a stored continuation.
                        stored.getAndSet(null)?.let { uniffiContinuationResume(it) }
                        inCancel.set(false)
                    },
                    free = {
                        if (inCancel.get()) freeDuringCancel.set(true)
                        freed.set(true)
                        freedCount.incrementAndGet()
                    },
                )
            }

            while (!polled.get()) Thread.yield()
            // Race a wake against cancellation across iterations.
            val waker = if (iter % 2 == 0) {
                launch(Dispatchers.Default) {
                    stored.getAndSet(null)?.let { uniffiContinuationResume(it) }
                }
            } else {
                null
            }
            job.cancel(CancellationException("cancelled"))
            job.join()
            waker?.join()

            assertFalse("iter $iter: cancel ran after free", cancelAfterFree.get())
            assertFalse("iter $iter: free ran while cancel was in flight", freeDuringCancel.get())
            assertTrue("iter $iter: cancel ran ${cancelCount.get()} times", cancelCount.get() <= 1)
            assertEquals("iter $iter: free must run once", 1, freedCount.get())
        }
    }

    /** A future that suspends, wakes and completes runs `free` once and never `cancel`. */
    @Test
    fun pendingThenReadyFreesOnceWithoutCancel() = runBlocking {
        val polls = AtomicInteger(0)
        val cancels = AtomicInteger(0)
        val frees = AtomicInteger(0)
        val cont = AtomicReference<Continuation<Boolean>?>(null)

        val job = launch(Dispatchers.Default) {
            uniffiAwaitFuture(
                rustFuture = 42L,
                poll = { _, c ->
                    if (polls.getAndIncrement() == 0) {
                        cont.set(c)
                        UNIFFI_RUST_FUTURE_PENDING
                    } else {
                        UNIFFI_RUST_FUTURE_COMPLETE
                    }
                },
                cancel = { cancels.incrementAndGet() },
                free = { frees.incrementAndGet() },
            )
        }
        while (cont.get() == null) Thread.yield()
        uniffiContinuationResume(cont.get()!!)
        job.join()

        assertEquals(2, polls.get())
        assertEquals(0, cancels.get())
        assertEquals(1, frees.get())
    }
}
