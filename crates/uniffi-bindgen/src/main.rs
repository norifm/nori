// Generates the Kotlin bindings: `bindings src:nori-android <out dir>`.
fn main() {
    uniffi_bindgen_kotlin_jni::main().unwrap();
    let args: Vec<String> = std::env::args().collect();
    if let (Some("bindings"), Some(out)) = (args.get(1).map(String::as_str), args.get(3)) {
        let path = std::path::Path::new(out).join("uniffi").join("Uniffi.kt");
        if let Ok(kt) = std::fs::read_to_string(&path) {
            std::fs::write(&path, cancellable(&nullable_futures(&kt))).unwrap();
        }
        pin_calls_in(std::path::Path::new(out));
    }
}

fn pin_calls_in(dir: &std::path::Path) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            pin_calls_in(&path);
        } else if path.extension().is_some_and(|e| e == "kt") {
            let kt = std::fs::read_to_string(&path).unwrap();
            let pinned = pinned_calls(&kt);
            if pinned != kt {
                std::fs::write(&path, pinned).unwrap();
            }
        }
    }
}

/// Upstream bug workaround: async exports end in `return completion.value!!`, which throws when a
/// nullable return (`T?`) is null. Drops the `!!` for those. Remove once the pinned revision fixes it.
fn nullable_futures(kt: &str) -> String {
    let mut out = String::with_capacity(kt.len());
    let mut nullable = false;
    for line in kt.split_inclusive('\n') {
        if let Some(ret) = line.trim_end().strip_prefix(") : ") {
            nullable = ret.ends_with('?');
        }
        if nullable && line.trim() == "return completion.value!!" {
            out.push_str(&line.replacen("completion.value!!", "completion.value", 1));
        } else {
            out.push_str(line);
        }
    }
    out
}

/// NORI: upstream's await does not hear the coroutine's cancellation, so a flow that let go of a call
/// waited for all of it (`flatMapLatest` joins the call it cancels). Each `awaitRustFuture*` is routed
/// through [`AWAIT_FUTURE`], which catches synchronous exceptions from `poll` (so `suspendCancellableCoroutine`
/// detaches its `ChildContinuation` instead of firing `invokeOnCancellation` after `free`) and reference-counts
/// the future handle across `cancel` and `free` (so a concurrent cancel can never touch a freed handle).
/// Async Kotlin calls Rust makes are kept by their oneshot handle, so a Rust future dropped mid-call cancels them.
fn cancellable(kt: &str) -> String {
    let mut out = String::with_capacity(kt.len());
    let mut lines = kt.split_inclusive('\n').peekable();
    let mut call = false;
    let mut awaits = 0usize;
    while let Some(line) = lines.next() {
        if let Some(id) = line.strip_prefix("suspend fun awaitRustFuture").and_then(|l| l.strip_suffix("(\n")) {
            let mut header = String::from(line);
            for h in lines.by_ref() {
                header.push_str(h);
                if h == "{\n" {
                    break;
                }
            }
            let mut body = String::new();
            for b in lines.by_ref() {
                body.push_str(b);
                if b == "}\n" {
                    break;
                }
            }
            assert!(
                body.contains("kotlin.coroutines.suspendCoroutine<kotlin.Boolean>")
                    && body.contains(&format!("Scaffolding.rustFuturePoll{id}("))
                    && body.contains(&format!("Scaffolding.rustFutureFree{id}(rustFuture)")),
                "awaitRustFuture{id} has an unexpected shape",
            );
            let completion = format!("CompleteRustFuture{id}");
            out.push_str(&header);
            if body.contains(&completion) {
                let ret = body
                    .lines()
                    .map(str::trim)
                    .find(|l| l.starts_with("return completion.value"))
                    .expect("a value-returning awaitRustFuture returns completion.value");
                out.push_str(&format!(
                    "    val completion = {completion}()\n    uniffiAwaitFuture(rustFuture, {{ f, c -> Scaffolding.rustFuturePoll{id}(f, c, completion) }}, Scaffolding::rustFutureCancel{id}, Scaffolding::rustFutureFree{id})\n    {ret}\n}}\n",
                ));
            } else {
                out.push_str(&format!(
                    "    uniffiAwaitFuture(rustFuture, Scaffolding::rustFuturePoll{id}, Scaffolding::rustFutureCancel{id}, Scaffolding::rustFutureFree{id})\n}}\n",
                ));
            }
            awaits += 1;
        } else if line.trim() == "kotlinx.coroutines.GlobalScope.launch uniffiCoroutineBlock@ {" {
            call = true;
            out.push_str(&line.replacen(".launch ", ".launch(start = kotlinx.coroutines.CoroutineStart.LAZY) ", 1));
        } else if call && line == "    }\n" {
            call = false;
            out.push_str("    }.let { uniffiKotlinCalls.start(uniffiOneshotHandle, it) }\n");
        } else {
            out.push_str(line);
        }
    }
    assert!(awaits > 0, "expected at least one awaitRustFuture function");
    out.push_str(AWAIT_FUTURE);
    out.push_str(KOTLIN_CALLS);
    out
}

const AWAIT_FUTURE: &str = "
/** NORI: polls a Rust future cancellably, keeping its handle alive across any concurrent cancel until both await and cancel finish. */
internal suspend fun uniffiAwaitFuture(
    rustFuture: kotlin.Long,
    poll: (kotlin.Long, kotlin.coroutines.Continuation<kotlin.Boolean>) -> kotlin.Int,
    cancel: (kotlin.Long) -> Unit,
    free: (kotlin.Long) -> Unit,
) {
    val refs = java.util.concurrent.atomic.AtomicInteger(1)
    val cancelled = java.util.concurrent.atomic.AtomicBoolean(false)
    fun cancelOnce() {
        if (!cancelled.compareAndSet(false, true)) return
        while (true) {
            val n = refs.get()
            if (n == 0) return
            if (refs.compareAndSet(n, n + 1)) break
        }
        try {
            cancel(rustFuture)
        } finally {
            if (refs.decrementAndGet() == 0) free(rustFuture)
        }
    }
    try {
        while (true) {
            val futureReady = kotlinx.coroutines.suspendCancellableCoroutine<kotlin.Boolean> { continuation ->
                continuation.invokeOnCancellation { cancelOnce() }
                val pollResult = try {
                    poll(rustFuture, continuation)
                } catch (e: Throwable) {
                    continuation.resumeWith(Result.failure(e))
                    return@suspendCancellableCoroutine
                }
                when (pollResult) {
                    UNIFFI_RUST_FUTURE_PENDING -> {}
                    UNIFFI_RUST_FUTURE_COMPLETE -> continuation.resumeWith(Result.success(true))
                    UNIFFI_RUST_FUTURE_CANCELLED -> continuation.resumeWith(
                        Result.failure(kotlin.coroutines.cancellation.CancellationException())
                    )
                    else -> continuation.resumeWith(
                        Result.failure(uniffi.InternalException(\"Error polling Rust future (code: $pollResult)\"))
                    )
                }
            }
            if (futureReady) return
        }
    } catch (e: kotlin.coroutines.cancellation.CancellationException) {
        cancelOnce()
        throw e
    } finally {
        if (refs.decrementAndGet() == 0) free(rustFuture)
    }
}
";

const KOTLIN_CALLS: &str = "
/** NORI: the async Kotlin calls Rust waits for, by oneshot handle (held by Rust while it waits, so never reused). */
internal object uniffiKotlinCalls {
    private val jobs = java.util.concurrent.ConcurrentHashMap<kotlin.Long, kotlinx.coroutines.Job>()

    fun start(handle: kotlin.Long, job: kotlinx.coroutines.Job) {
        jobs[handle] = job
        job.invokeOnCompletion { jobs.remove(handle, job) }
        job.start()
    }

    fun cancel(handle: kotlin.Long) {
        jobs.remove(handle)?.cancel()
    }
}

/** NORI: Rust stopped waiting for the call under [handle]. */
fun uniffiKotlinCallDropped(handle: kotlin.Long) = uniffiKotlinCalls.cancel(handle)
";

/// NORI: upstream hands Rust `this` as a borrowed handle and nothing keeps the object alive for the call,
/// so the Cleaner (or `destroy`) could free the Rust object while the call still used it. Each such call
/// now runs under the object's call count, as upstream's JNA bindings do: the object stays reachable until
/// Rust returns, and `destroy` frees it only once no call is running.
fn pinned_calls(kt: &str) -> String {
    let mut out = String::with_capacity(kt.len() + kt.len() / 16);
    let (mut borrowed, mut call) = (false, false);
    for line in kt.split_inclusive('\n') {
        let lowered = line.trim().strip_prefix("val uniffiArgLowered0 = uniffi.lowerObject");
        if line.trim_end() == "    private val callCounter = java.util.concurrent.atomic.AtomicLong(1)" {
            out.push_str(line);
            out.push_str(CALL_COUNT);
        } else if lowered.is_some_and(|l| l.contains("Ref") && l.ends_with("(this)")) {
            assert!(!borrowed && !call, "a borrowed receiver is passed to the call that follows it");
            borrowed = true;
            out.push_str(line);
        } else if borrowed && line.starts_with("    val uniffiReturn = uniffi.Scaffolding.") {
            (borrowed, call) = (false, true);
            out.push_str(&line.replacen("= uniffi.Scaffolding.", "= uniffiCall { uniffi.Scaffolding.", 1));
        } else if call && line == "    )\n" {
            call = false;
            out.push_str("    ) }\n");
        } else {
            out.push_str(line);
        }
    }
    assert!(!borrowed && !call, "every borrowed receiver's call was wrapped");
    out
}

const CALL_COUNT: &str = "    private inline fun <R> uniffiCall(call: () -> R): R {
        while (true) {
            val count = callCounter.get()
            check(count > 0L) { \"${javaClass.simpleName} was destroyed\" }
            if (callCounter.compareAndSet(count, count + 1L)) break
        }
        try {
            return call()
        } finally {
            if (callCounter.decrementAndGet() == 0L) uniffiCleanable?.clean()
        }
    }
";

#[cfg(test)]
mod tests {
    #[test]
    fn calls_hold_their_object() {
        let kt = "class A {\n    private val callCounter = java.util.concurrent.atomic.AtomicLong(1)\n\n    fun m(x: kotlin.Long): kotlin.Long {\n    val uniffiArgLowered0 = uniffi.lowerObjectRef7(this)\n    val uniffiArgLowered1 = x\n\n    val uniffiReturn = uniffi.Scaffolding.m(\n        uniffiArgLowered0,\n        uniffiArgLowered1,\n    )\n    return uniffiReturn\n    }\n}\nfun f() {\n    val uniffiReturn = uniffi.Scaffolding.f(\n    )\n}\n";
        let fixed = super::pinned_calls(kt);
        assert!(fixed.contains("AtomicLong(1)\n    private inline fun <R> uniffiCall("));
        assert!(fixed.contains("    val uniffiReturn = uniffiCall { uniffi.Scaffolding.m(\n        uniffiArgLowered0,\n        uniffiArgLowered1,\n    ) }\n    return uniffiReturn\n"));
        assert!(fixed.contains("fun f() {\n    val uniffiReturn = uniffi.Scaffolding.f(\n    )\n}\n"));
    }

    #[test]
    fn nullable_future_drops_non_null_assertion() {
        let kt = "suspend fun a(\n    f: kotlin.Long,\n) : kotlin.String?\n{\n                return completion.value!!\n}\nsuspend fun b(\n    f: kotlin.Long,\n) : kotlin.String\n{\n                return completion.value!!\n}\n";
        let fixed = super::nullable_futures(kt);
        assert_eq!(fixed.matches("return completion.value!!").count(), 1);
        assert!(fixed.contains("kotlin.String?\n{\n                return completion.value\n}"));
    }

    #[test]
    fn awaits_and_kotlin_calls_cancel() {
        let kt = "\
suspend fun awaitRustFuture0(
    rustFuture: kotlin.Long,
)
{
    try {
        while(true) {
            val futureReady = kotlin.coroutines.suspendCoroutine<kotlin.Boolean> { continuation ->
                val pollResult = Scaffolding.rustFuturePoll0(
                    rustFuture,
                    continuation,
            )
            }
            if (futureReady) {
                return
            }
        }
    } finally {
        Scaffolding.rustFutureFree0(rustFuture)
    }
}
suspend fun awaitRustFuture7(
    rustFuture: kotlin.Long,
) : kotlin.String?
{
    try {
        val completion = CompleteRustFuture7();
        while(true) {
            val futureReady = kotlin.coroutines.suspendCoroutine<kotlin.Boolean> { continuation ->
                val pollResult = Scaffolding.rustFuturePoll7(
                    rustFuture,
                    continuation,
                    completion,
            )
            }
            if (futureReady) {
                return completion.value
            }
        }
    } finally {
        Scaffolding.rustFutureFree7(rustFuture)
    }
}
fun dispatch(
) {
    kotlinx.coroutines.GlobalScope.launch uniffiCoroutineBlock@ {
        try {
        }
    }
}
";
        let fixed = super::cancellable(kt);
        assert!(fixed.contains("uniffiAwaitFuture(rustFuture, Scaffolding::rustFuturePoll0, Scaffolding::rustFutureCancel0, Scaffolding::rustFutureFree0)"));
        assert!(fixed.contains("val completion = CompleteRustFuture7()\n    uniffiAwaitFuture(rustFuture, { f, c -> Scaffolding.rustFuturePoll7(f, c, completion) }, Scaffolding::rustFutureCancel7, Scaffolding::rustFutureFree7)\n    return completion.value\n"));
        assert!(fixed.contains("internal suspend fun uniffiAwaitFuture("));
        assert!(fixed.contains("GlobalScope.launch(start = kotlinx.coroutines.CoroutineStart.LAZY) uniffiCoroutineBlock@ {\n        try {\n        }\n    }.let { uniffiKotlinCalls.start(uniffiOneshotHandle, it) }\n}\n"));
    }
}
